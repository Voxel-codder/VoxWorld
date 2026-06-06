use std::{
    net::SocketAddr,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use common::{
    ViewDistances,
    comp::{
        self, ControllerInputs,
        body::humanoid::{Body, BodyType, Species},
    },
    uid::Uid,
    util::dir::Dir,
};
use serde::{Deserialize, Serialize};
use tokio::{runtime::Runtime, sync::mpsc::UnboundedSender};
use tracing::{error, warn};
use vek::{Vec2, Vec3};
use veloren_client::{Client, ClientType, Event, Join, WorldExt, addr::ConnectionArgs};

const TICK: Duration = Duration::from_millis(50);
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(250);
const SNAPSHOT_ENTITY_LIMIT: usize = 96;
const WEB_VIEW_DISTANCE: ViewDistances = ViewDistances {
    terrain: 5,
    entity: 5,
};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserCommand {
    Input {
        move_x: f32,
        move_y: f32,
        move_z: f32,
        look_x: f32,
        look_y: f32,
        look_z: f32,
    },
    Chat {
        message: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SessionMessage {
    Stage {
        stage: String,
    },
    Snapshot {
        username: String,
        in_game: bool,
        position: Option<[f32; 3]>,
        players_online: Vec<String>,
        character_count: usize,
        entities: Vec<SnapshotEntity>,
    },
    Event {
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Serialize)]
struct SnapshotEntity {
    uid: String,
    name: Option<String>,
    kind: &'static str,
    is_self: bool,
    position: [f32; 3],
    distance: f32,
}

pub struct PlaySession {
    commands: mpsc::Sender<BrowserCommand>,
}

impl PlaySession {
    pub fn send(&self, command: BrowserCommand) -> Result<(), mpsc::SendError<BrowserCommand>> {
        self.commands.send(command)
    }
}

pub fn start(
    upstream: SocketAddr,
    username: String,
    outbound: UnboundedSender<String>,
) -> PlaySession {
    let (commands, command_rx) = mpsc::channel();

    thread::Builder::new()
        .name(format!("web-session-{username}"))
        .spawn({
            let username = username.clone();
            move || run_session(upstream, username, command_rx, outbound)
        })
        .expect("failed to spawn web session thread");

    PlaySession { commands }
}

fn run_session(
    upstream: SocketAddr,
    username: String,
    command_rx: mpsc::Receiver<BrowserCommand>,
    outbound: UnboundedSender<String>,
) {
    if let Err(error) = run_session_inner(upstream, username, command_rx, outbound.clone()) {
        error!(%error, "web play session failed");
        send_json(&outbound, SessionMessage::Error {
            message: error.to_string(),
        });
    }
}

fn run_session_inner(
    upstream: SocketAddr,
    username: String,
    command_rx: mpsc::Receiver<BrowserCommand>,
    outbound: UnboundedSender<String>,
) -> Result<(), String> {
    send_json(&outbound, SessionMessage::Stage {
        stage: "connecting".to_owned(),
    });

    let runtime = Arc::new(Runtime::new().map_err(|error| error.to_string())?);
    let runtime_clone = Arc::clone(&runtime);
    let addr = ConnectionArgs::Tcp {
        hostname: upstream.to_string(),
        prefer_ipv6: false,
    };
    let mut mismatched_server_info = None;
    let stage_sender = outbound.clone();
    let mut client = runtime
        .block_on(Client::new(
            addr,
            runtime_clone,
            &mut mismatched_server_info,
            &username,
            "",
            Some("en".to_owned()),
            |_| true,
            &move |stage| {
                send_json(&stage_sender, SessionMessage::Stage {
                    stage: format!("{stage:?}"),
                });
            },
            |_| {},
            Default::default(),
            ClientType::Game,
        ))
        .map_err(|error| format!("{error:?}"))?;

    send_json(&outbound, SessionMessage::Stage {
        stage: "registered".to_owned(),
    });
    client.load_character_list();

    let mut inputs = ControllerInputs::default();
    let mut character_create_requested = false;
    let mut character_join_requested = false;
    let mut in_game = false;
    let mut last_snapshot = Instant::now() - SNAPSHOT_INTERVAL;

    loop {
        if !handle_browser_commands(&command_rx, &mut client, &mut inputs) {
            send_json(&outbound, SessionMessage::Stage {
                stage: "browser_disconnected".to_owned(),
            });
            return Ok(());
        }

        let events = client
            .tick(inputs.clone(), TICK)
            .map_err(|error| format!("{error:?}"))?;
        for event in events {
            match event {
                Event::CharacterCreated(_) => {
                    send_json(&outbound, SessionMessage::Stage {
                        stage: "character_created".to_owned(),
                    });
                    client.load_character_list();
                },
                Event::CharacterJoined(_) => {
                    in_game = true;
                    send_json(&outbound, SessionMessage::Stage {
                        stage: "in_game".to_owned(),
                    });
                },
                Event::CharacterError(error) => {
                    character_create_requested = false;
                    character_join_requested = false;
                    send_json(&outbound, SessionMessage::Error { message: error });
                },
                Event::Disconnect => {
                    return Err("server disconnected the session".to_owned());
                },
                Event::DisconnectionNotification(seconds) => {
                    send_json(&outbound, SessionMessage::Event {
                        message: format!("disconnecting in {seconds}s"),
                    });
                },
                Event::Chat(message) => {
                    send_json(&outbound, SessionMessage::Event {
                        message: format!("{message:?}"),
                    });
                },
                _ => {},
            }
        }

        if !in_game {
            let characters = client.character_list();
            if !characters.loading
                && characters.characters.is_empty()
                && !character_create_requested
            {
                character_create_requested = true;
                send_json(&outbound, SessionMessage::Stage {
                    stage: "creating_character".to_owned(),
                });
                client.create_character(
                    username.clone(),
                    Some("common.items.weapons.sword.starter".to_owned()),
                    None,
                    default_body().into(),
                    false,
                    None,
                );
            } else if !characters.loading
                && !character_join_requested
                && let Some(character_id) = characters
                    .characters
                    .first()
                    .and_then(|character| character.character.id)
            {
                character_join_requested = true;
                send_json(&outbound, SessionMessage::Stage {
                    stage: "joining_character".to_owned(),
                });
                client.request_character(character_id, WEB_VIEW_DISTANCE);
            }
        }

        if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
            last_snapshot = Instant::now();
            send_snapshot(&outbound, &username, in_game, &client);
        }

        client.cleanup();
        thread::sleep(TICK);
    }
}

fn handle_browser_commands(
    command_rx: &mpsc::Receiver<BrowserCommand>,
    client: &mut Client,
    inputs: &mut ControllerInputs,
) -> bool {
    loop {
        match command_rx.try_recv() {
            Ok(BrowserCommand::Input {
                move_x,
                move_y,
                move_z,
                look_x,
                look_y,
                look_z,
            }) => {
                inputs.move_dir = Vec2::new(move_x, move_y);
                inputs.move_z = move_z;
                inputs.look_dir =
                    Dir::from_unnormalized(Vec3::new(look_x, look_y, look_z)).unwrap_or_default();
                inputs.sanitize();
            },
            Ok(BrowserCommand::Chat { message }) => {
                if !message.trim().is_empty() {
                    client.send_chat(message);
                }
            },
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
        }
    }
}

fn send_snapshot(
    outbound: &UnboundedSender<String>,
    username: &str,
    in_game: bool,
    client: &Client,
) {
    let position = client
        .position()
        .map(|position| [position.x, position.y, position.z]);
    let players_online = client.players().map(str::to_owned).collect();
    let entities = snapshot_entities(client);

    send_json(outbound, SessionMessage::Snapshot {
        username: username.to_owned(),
        in_game,
        position,
        players_online,
        character_count: client.character_list().characters.len(),
        entities,
    });
}

fn snapshot_entities(client: &Client) -> Vec<SnapshotEntity> {
    let Some(origin) = client.position() else {
        return Vec::new();
    };
    let self_uid = client.uid();
    let ecs = client.state().ecs();
    let positions = ecs.read_storage::<comp::Pos>();
    let uids = ecs.read_storage::<Uid>();
    let players = client.player_list();

    let mut entities = (&uids, &positions)
        .join()
        .filter_map(|(uid, position)| {
            let delta = position.0 - origin;
            let distance = delta.magnitude();

            if !distance.is_finite() || distance > 512.0 {
                return None;
            }

            let player = players.get(uid);
            Some(SnapshotEntity {
                uid: uid.to_string(),
                name: player.map(|player| player.player_alias.clone()),
                kind: if player.is_some() { "player" } else { "entity" },
                is_self: self_uid == Some(*uid),
                position: [position.0.x, position.0.y, position.0.z],
                distance,
            })
        })
        .collect::<Vec<_>>();

    entities.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    entities.truncate(SNAPSHOT_ENTITY_LIMIT);
    entities
}

fn default_body() -> Body {
    Body {
        species: Species::Human,
        body_type: BodyType::Male,
        hair_style: 0,
        beard: 0,
        eyes: 0,
        accessory: 0,
        hair_color: 0,
        skin: 0,
        eye_color: 0,
    }
}

fn send_json(outbound: &UnboundedSender<String>, message: SessionMessage) {
    match serde_json::to_string(&message) {
        Ok(message) => {
            if outbound.send(message).is_err() {
                warn!("web play session output channel closed");
            }
        },
        Err(error) => warn!(%error, "failed to serialize web play session message"),
    }
}
