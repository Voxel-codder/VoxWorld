use std::{
    net::SocketAddr,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use common::{
    ViewDistances,
    comp::{
        self, ChatType, Content, ControllerInputs, InputKind,
        body::humanoid::{Body, BodyType, Species},
        inventory::{item::ItemDesc, slot::EquipSlot},
    },
    uid::Uid,
    util::dir::Dir,
};
use serde::{Deserialize, Serialize};
use specs::Entity as EcsEntity;
use tokio::{runtime::Runtime, sync::mpsc::UnboundedSender};
use tracing::{error, warn};
use vek::{Vec2, Vec3};
use veloren_client::{Client, ClientType, Event, Join, WorldExt, addr::ConnectionArgs};

const TICK: Duration = Duration::from_millis(50);
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(250);
const SNAPSHOT_ENTITY_LIMIT: usize = 96;
const SNAPSHOT_INVENTORY_ITEM_LIMIT: usize = 8;
const PICKUP_INTERACTION_DISTANCE: f32 = 4.0;
const ENTITY_INTERACTION_DISTANCE: f32 = 5.0;
const JOIN_TIMEOUT: Duration = Duration::from_secs(90);
const CHARACTER_ERROR_LIMIT: usize = 3;
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
    Action {
        action: BrowserAction,
        pressed: bool,
    },
    Control {
        control: BrowserControl,
    },
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    Primary,
    Secondary,
    Block,
    Roll,
    Jump,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum BrowserControl {
    Interact,
    Pickup,
    ToggleWield,
    SwapLoadout,
    Sneak,
    Sit,
    Respawn,
}

impl BrowserAction {
    fn input_kind(self) -> InputKind {
        match self {
            Self::Primary => InputKind::Primary,
            Self::Secondary => InputKind::Secondary,
            Self::Block => InputKind::Block,
            Self::Roll => InputKind::Roll,
            Self::Jump => InputKind::Jump,
        }
    }
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
        health: Option<PlayerStat>,
        energy: Option<PlayerStat>,
        is_dead: bool,
        players_online: Vec<String>,
        character_count: usize,
        entities: Vec<SnapshotEntity>,
        inventory: Option<SnapshotInventory>,
        interaction: Option<SnapshotInteraction>,
    },
    Event {
        message: String,
    },
    Chat {
        scope: &'static str,
        from: Option<String>,
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
    health: Option<PlayerStat>,
}

#[derive(Debug, Serialize)]
struct SnapshotInventory {
    occupied_slots: usize,
    total_slots: usize,
    mainhand: Option<SnapshotItem>,
    offhand: Option<SnapshotItem>,
    items: Vec<SnapshotItem>,
}

#[derive(Debug, Serialize)]
struct SnapshotItem {
    name: String,
    amount: u32,
}

#[derive(Debug, Serialize)]
struct SnapshotInteraction {
    action: &'static str,
    label: String,
    distance: f32,
}

#[derive(Debug, Serialize)]
struct PlayerStat {
    current: f32,
    maximum: f32,
    fraction: f32,
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
    let mut character_errors = 0;
    let mut in_game = false;
    let join_started = Instant::now();
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
                    character_errors += 1;
                    send_json(&outbound, SessionMessage::Error {
                        message: error.clone(),
                    });
                    if character_errors >= CHARACTER_ERROR_LIMIT {
                        return Err(format!(
                            "character setup failed after {CHARACTER_ERROR_LIMIT} attempts: \
                             {error}"
                        ));
                    }
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
                    send_json(&outbound, browser_chat_message(&client, &message));
                },
                _ => {},
            }
        }

        if !in_game && join_started.elapsed() > JOIN_TIMEOUT {
            return Err(format!(
                "timed out joining game session after {} seconds",
                JOIN_TIMEOUT.as_secs()
            ));
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
            Ok(BrowserCommand::Action { action, pressed }) => {
                client.handle_input(action.input_kind(), pressed, None, None);
            },
            Ok(BrowserCommand::Control { control }) => {
                handle_browser_control(client, control);
            },
            Err(mpsc::TryRecvError::Empty) => return true,
            Err(mpsc::TryRecvError::Disconnected) => return false,
        }
    }
}

fn handle_browser_control(client: &mut Client, control: BrowserControl) {
    match control {
        BrowserControl::Interact => {
            if let Some((entity, _, _)) = nearest_pickup_target(client) {
                client.pick_up(entity);
            } else if let Some((entity, _, _)) = nearest_interactable_target(client) {
                client.npc_interact(entity);
            }
        },
        BrowserControl::Pickup => {
            if let Some((entity, _, _)) = nearest_pickup_target(client) {
                client.pick_up(entity);
            }
        },
        BrowserControl::ToggleWield => client.toggle_wield(),
        BrowserControl::SwapLoadout => client.swap_loadout(),
        BrowserControl::Sneak => client.toggle_sneak(),
        BrowserControl::Sit => client.toggle_sit(),
        BrowserControl::Respawn => {
            let _ = client.respawn();
        },
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use common::comp::{ChatType, Content};

    use super::{BrowserAction, BrowserCommand, BrowserControl, chat_scope, content_text};

    #[test]
    fn decodes_browser_action_command() {
        let command: BrowserCommand =
            serde_json::from_str(r#"{"type":"action","action":"primary","pressed":true}"#)
                .expect("action command should decode");

        match command {
            BrowserCommand::Action { action, pressed } => {
                assert!(matches!(action, BrowserAction::Primary));
                assert!(pressed);
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn decodes_browser_control_command() {
        let command: BrowserCommand =
            serde_json::from_str(r#"{"type":"control","control":"interact"}"#)
                .expect("control command should decode");

        match command {
            BrowserCommand::Control { control } => {
                assert!(matches!(control, BrowserControl::Interact));
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn maps_chat_scope_for_world_messages() {
        let scope = chat_scope(&ChatType::World(common::uid::Uid(
            NonZeroU64::new(1).expect("non-zero uid"),
        )));

        assert_eq!(scope, "world");
    }

    #[test]
    fn prefers_chat_content_fallback_text() {
        let content = Content::WithFallback(
            Box::new(Content::Key("chat-key".to_owned())),
            Box::new(Content::Plain("fallback text".to_owned())),
        );

        assert_eq!(content_text(&content), "fallback text");
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
    let inventory = snapshot_inventory(client);
    let interaction = snapshot_interaction(client);
    let (health, energy, is_dead) = snapshot_player_stats(client);

    send_json(outbound, SessionMessage::Snapshot {
        username: username.to_owned(),
        in_game,
        position,
        health,
        energy,
        is_dead,
        players_online,
        character_count: client.character_list().characters.len(),
        entities,
        inventory,
        interaction,
    });
}

fn snapshot_player_stats(client: &Client) -> (Option<PlayerStat>, Option<PlayerStat>, bool) {
    let ecs = client.state().ecs();
    let entity = client.entity();
    let healths = ecs.read_storage::<comp::Health>();
    let health = healths.get(entity);
    let health_stat = health.map(|health| PlayerStat {
        current: health.current(),
        maximum: health.maximum(),
        fraction: health.fraction().clamp(0.0, 1.0),
    });
    let is_dead = health.is_some_and(|health| health.is_dead);
    let energy = ecs
        .read_storage::<comp::Energy>()
        .get(entity)
        .map(|energy| PlayerStat {
            current: energy.current(),
            maximum: energy.maximum(),
            fraction: energy.fraction().clamp(0.0, 1.0),
        });

    (health_stat, energy, is_dead)
}

fn snapshot_entities(client: &Client) -> Vec<SnapshotEntity> {
    let Some(origin) = client.position() else {
        return Vec::new();
    };
    let self_uid = client.uid();
    let ecs = client.state().ecs();
    let ecs_entities = ecs.entities();
    let positions = ecs.read_storage::<comp::Pos>();
    let healths = ecs.read_storage::<comp::Health>();
    let uids = ecs.read_storage::<Uid>();
    let alignments = ecs.read_storage::<comp::Alignment>();
    let pickups = ecs.read_storage::<comp::PickupItem>();
    let stats = ecs.read_storage::<comp::Stats>();
    let players = client.player_list();

    let mut entities = (&ecs_entities, &uids, &positions)
        .join()
        .filter_map(|(entity, uid, position)| {
            let delta = position.0 - origin;
            let distance = delta.magnitude();

            if !distance.is_finite() || distance > 512.0 {
                return None;
            }

            let player = players.get(uid);
            let pickup = pickups.get(entity);
            let health = healths.get(entity);
            Some(SnapshotEntity {
                uid: uid.to_string(),
                name: snapshot_entity_name(
                    player.map(|player| player.player_alias.clone()),
                    pickup,
                    stats.get(entity),
                ),
                kind: snapshot_entity_kind(
                    player.is_some(),
                    pickup.is_some(),
                    alignments.get(entity),
                    health,
                ),
                is_self: self_uid == Some(*uid),
                position: [position.0.x, position.0.y, position.0.z],
                distance,
                health: health.map(|health| PlayerStat {
                    current: health.current(),
                    maximum: health.maximum(),
                    fraction: health.fraction().clamp(0.0, 1.0),
                }),
            })
        })
        .collect::<Vec<_>>();

    entities.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    entities.truncate(SNAPSHOT_ENTITY_LIMIT);
    entities
}

fn snapshot_entity_name(
    player_name: Option<String>,
    pickup: Option<&comp::PickupItem>,
    stats: Option<&comp::Stats>,
) -> Option<String> {
    player_name
        .or_else(|| pickup.map(pickup_item_label))
        .or_else(|| stats.map(|stats| content_text(&stats.name)))
}

fn snapshot_entity_kind(
    is_player: bool,
    is_pickup: bool,
    alignment: Option<&comp::Alignment>,
    health: Option<&comp::Health>,
) -> &'static str {
    if is_player {
        "player"
    } else if is_pickup {
        "pickup"
    } else {
        match alignment {
            Some(comp::Alignment::Npc) => "npc",
            Some(comp::Alignment::Tame | comp::Alignment::Owned(_) | comp::Alignment::Passive) => {
                "friendly"
            },
            Some(comp::Alignment::Enemy | comp::Alignment::Wild) if health.is_some() => "enemy",
            _ if health.is_some() => "creature",
            _ => "entity",
        }
    }
}

fn snapshot_interaction(client: &Client) -> Option<SnapshotInteraction> {
    if let Some((_, label, distance)) = nearest_pickup_target(client) {
        return Some(SnapshotInteraction {
            action: "pickup",
            label,
            distance,
        });
    }

    nearest_interactable_target(client).map(|(_, label, distance)| SnapshotInteraction {
        action: "interact",
        label,
        distance,
    })
}

fn snapshot_inventory(client: &Client) -> Option<SnapshotInventory> {
    let inventories = client.state().ecs().read_storage::<comp::Inventory>();
    let inventory = inventories.get(client.entity())?;
    let total_slots = inventory.capacity();
    let mut occupied_slots = 0;
    let mut items = Vec::new();

    for item in inventory.slots().filter_map(|slot| slot.as_ref()) {
        occupied_slots += 1;
        if items.len() < SNAPSHOT_INVENTORY_ITEM_LIMIT {
            items.push(snapshot_item(item));
        }
    }

    Some(SnapshotInventory {
        occupied_slots,
        total_slots,
        mainhand: inventory
            .equipped(EquipSlot::ActiveMainhand)
            .map(snapshot_item),
        offhand: inventory
            .equipped(EquipSlot::ActiveOffhand)
            .map(snapshot_item),
        items,
    })
}

#[allow(deprecated)]
fn nearest_pickup_target(client: &Client) -> Option<(EcsEntity, String, f32)> {
    let origin = client.position()?;
    let ecs = client.state().ecs();
    let ecs_entities = ecs.entities();
    let positions = ecs.read_storage::<comp::Pos>();
    let pickups = ecs.read_storage::<comp::PickupItem>();

    (&ecs_entities, &positions, &pickups)
        .join()
        .filter_map(|(entity, position, pickup)| {
            let distance = (position.0 - origin).magnitude();
            if !distance.is_finite() || distance > PICKUP_INTERACTION_DISTANCE {
                return None;
            }

            let label = format!("Pick up {}", pickup_item_label(pickup));
            Some((entity, label, distance))
        })
        .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
}

#[allow(deprecated)]
fn pickup_item_label(pickup: &comp::PickupItem) -> String {
    let amount = pickup.amount();
    if amount > 1 {
        format!("{} x{}", pickup.legacy_name(), amount)
    } else {
        pickup.legacy_name().to_string()
    }
}

fn nearest_interactable_target(client: &Client) -> Option<(EcsEntity, String, f32)> {
    let origin = client.position()?;
    let self_entity = client.entity();
    let ecs = client.state().ecs();
    let ecs_entities = ecs.entities();
    let positions = ecs.read_storage::<comp::Pos>();
    let alignments = ecs.read_storage::<comp::Alignment>();
    let stats = ecs.read_storage::<comp::Stats>();

    (&ecs_entities, &positions, &alignments)
        .join()
        .filter_map(|(entity, position, alignment)| {
            if entity == self_entity
                || !matches!(
                    alignment,
                    comp::Alignment::Npc | comp::Alignment::Tame | comp::Alignment::Owned(_)
                )
            {
                return None;
            }

            let distance = (position.0 - origin).magnitude();
            if !distance.is_finite() || distance > ENTITY_INTERACTION_DISTANCE {
                return None;
            }

            let label = stats
                .get(entity)
                .map(|stats| format!("Interact with {}", content_text(&stats.name)))
                .unwrap_or_else(|| "Interact".to_owned());
            Some((entity, label, distance))
        })
        .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
}

#[allow(deprecated)]
fn snapshot_item(item: &comp::Item) -> SnapshotItem {
    SnapshotItem {
        name: item.legacy_name().to_string(),
        amount: item.amount(),
    }
}

fn browser_chat_message(client: &Client, message: &comp::ChatMsg) -> SessionMessage {
    SessionMessage::Chat {
        scope: chat_scope(&message.chat_type),
        from: chat_sender(client, message),
        message: content_text(message.content()),
    }
}

fn chat_scope(chat_type: &ChatType<String>) -> &'static str {
    match chat_type {
        ChatType::Online(_) => "online",
        ChatType::Offline(_) => "offline",
        ChatType::CommandInfo => "info",
        ChatType::CommandError => "error",
        ChatType::Kill(_, _) => "death",
        ChatType::GroupMeta(_) => "group",
        ChatType::FactionMeta(_) => "faction",
        ChatType::Tell(_, _) => "tell",
        ChatType::Say(_) => "say",
        ChatType::Group(_, _) => "group",
        ChatType::Faction(_, _) => "faction",
        ChatType::Region(_) => "region",
        ChatType::World(_) => "world",
        ChatType::Npc(_) | ChatType::NpcSay(_) | ChatType::NpcTell(_, _) => "npc",
        ChatType::Meta => "system",
    }
}

fn chat_sender(client: &Client, message: &comp::ChatMsg) -> Option<String> {
    let uid = message.uid()?;
    let context = client.lookup_msg_context(message);

    context
        .player_info
        .get(&uid)
        .map(|player| player.player_alias.clone())
        .or_else(|| context.entity_name.get(&uid).map(content_text))
}

fn content_text(content: &Content) -> String {
    match content {
        Content::Plain(text) => text.clone(),
        Content::Key(key) => key.clone(),
        Content::Attr(key, attr) => format!("{key}.{attr}"),
        Content::Localized { key, .. } => key.clone(),
        Content::WithFallback(_, fallback) => content_text(fallback),
    }
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
