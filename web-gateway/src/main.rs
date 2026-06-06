use std::{
    error::Error,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use query_server::client::QueryClient;
use serde_json::{Value, json};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    select,
    time::{Duration, interval, timeout},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

mod play_session;

type GatewayResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
static ACTIVE_PLAY_SESSIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Parser)]
#[command(name = "voxworld-web-gateway")]
#[command(about = "Bridge browser WebSocket traffic to the native Vox World TCP server.")]
struct Args {
    #[arg(long, env = "VOXWORLD_WEB_LISTEN", default_value = "127.0.0.1:14080")]
    listen: SocketAddr,

    #[arg(long, env = "VOXWORLD_UPSTREAM", default_value = "127.0.0.1:14004")]
    upstream: SocketAddr,

    #[arg(long, env = "VOXWORLD_QUERY_SERVER", default_value = "127.0.0.1:14006")]
    query_server: SocketAddr,

    #[arg(
        long,
        env = "VOXWORLD_WEB_STATIC_DIR",
        default_value = "web-client/web"
    )]
    static_dir: PathBuf,

    #[arg(long, env = "VOXWORLD_WEB_MAX_SESSIONS", default_value_t = 100)]
    max_sessions: usize,

    #[arg(long, env = "VOXWORLD_WEB_PING_INTERVAL_SECS", default_value_t = 30)]
    play_ping_interval_secs: u64,
}

#[tokio::main]
async fn main() -> GatewayResult<()> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let listener = TcpListener::bind(args.listen).await?;
    info!(
        listen = %args.listen,
        upstream = %args.upstream,
        query_server = %args.query_server,
        static_dir = %args.static_dir.display(),
        max_sessions = args.max_sessions,
        play_ping_interval_secs = args.play_ping_interval_secs,
        "web gateway listening"
    );

    loop {
        let (socket, peer) = listener.accept().await?;
        let upstream = args.upstream;
        let query_server = args.query_server;
        let static_dir = args.static_dir.clone();
        let max_sessions = args.max_sessions;
        let play_ping_interval = Duration::from_secs(args.play_ping_interval_secs.max(1));

        tokio::spawn(async move {
            if let Err(error) = handle_connection(
                socket,
                peer,
                upstream,
                query_server,
                static_dir,
                max_sessions,
                play_ping_interval,
            )
            .await
            {
                warn!(%peer, %error, "web gateway connection ended");
            }
        });
    }
}

async fn handle_connection(
    socket: TcpStream,
    peer: SocketAddr,
    upstream: SocketAddr,
    query_server: SocketAddr,
    static_dir: PathBuf,
    max_sessions: usize,
    play_ping_interval: Duration,
) -> GatewayResult<()> {
    let mut peek = [0_u8; 2048];
    let read = socket.peek(&mut peek).await?;
    let request_head = String::from_utf8_lossy(&peek[..read]);

    if !is_websocket_request(&request_head) {
        return serve_http(
            socket,
            peer,
            &static_dir,
            upstream,
            query_server,
            max_sessions,
            play_ping_interval,
        )
        .await;
    }

    if let Some(username) = play_username(&request_head) {
        return handle_play_connection(
            socket,
            peer,
            upstream,
            username,
            max_sessions,
            play_ping_interval,
        )
        .await;
    }

    info!(%peer, "browser websocket connected");

    let websocket = accept_async(socket).await?;
    let upstream = TcpStream::connect(upstream).await?;
    let (mut websocket_write, mut websocket_read) = websocket.split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();

    let websocket_to_upstream = async {
        while let Some(message) = websocket_read.next().await {
            match message? {
                Message::Binary(bytes) => upstream_write.write_all(&bytes).await?,
                Message::Text(text) => upstream_write.write_all(text.as_str().as_bytes()).await?,
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => {},
                Message::Frame(_) => {},
            }
        }

        upstream_write.shutdown().await?;
        GatewayResult::Ok(())
    };

    let upstream_to_websocket = async {
        let mut buffer = [0_u8; 16 * 1024];

        loop {
            let read = upstream_read.read(&mut buffer).await?;
            if read == 0 {
                break;
            }

            websocket_write
                .send(Message::Binary(buffer[..read].to_vec().into()))
                .await?;
        }

        if let Err(error) = websocket_write.close().await {
            error!(%peer, %error, "failed to close websocket");
        }

        GatewayResult::Ok(())
    };

    select! {
        result = websocket_to_upstream => result?,
        result = upstream_to_websocket => result?,
    }

    info!(%peer, "browser websocket disconnected");
    Ok(())
}

fn is_websocket_request(request_head: &str) -> bool {
    is_raw_proxy_request(request_head) || is_play_request(request_head)
}

fn is_raw_proxy_request(request_head: &str) -> bool {
    websocket_target(request_head)
        .is_some_and(|target| target_path(target) == "/ws" && is_websocket_upgrade(request_head))
}

fn is_play_request(request_head: &str) -> bool {
    websocket_target(request_head)
        .is_some_and(|target| target_path(target) == "/play" && is_websocket_upgrade(request_head))
}

fn is_websocket_upgrade(request_head: &str) -> bool {
    request_head
        .lines()
        .any(|line| line.eq_ignore_ascii_case("upgrade: websocket"))
}

fn websocket_target(request_head: &str) -> Option<&str> {
    let request_line = request_head.lines().next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;

    (method == "GET").then_some(target)
}

fn target_path(target: &str) -> &str {
    target.split_once('?').map_or(target, |(path, _query)| path)
}

fn play_username(request_head: &str) -> Option<String> {
    let target = websocket_target(request_head)?;
    if target_path(target) != "/play" || !is_websocket_upgrade(request_head) {
        return None;
    }

    Some(
        query_value(target, "name")
            .and_then(|name| sanitize_username(&name))
            .unwrap_or_else(next_guest_username),
    )
}

fn query_value(target: &str, key: &str) -> Option<String> {
    let query = target.split_once('?')?.1;

    query.split('&').find_map(|pair| {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        (raw_key == key).then(|| decode_query_value(raw_value))
    })
}

fn decode_query_value(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(' ');
                index += 1;
            },
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(char::from(byte));
                    index += 3;
                } else {
                    decoded.push('%');
                    index += 1;
                }
            },
            byte => {
                decoded.push(char::from(byte));
                index += 1;
            },
        }
    }

    decoded
}

fn sanitize_username(name: &str) -> Option<String> {
    let mut username = name
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_lowercase())
            } else if matches!(character, '-' | '_') {
                Some(character)
            } else {
                None
            }
        })
        .take(24)
        .collect::<String>();

    if username.len() < 3 {
        return None;
    }

    if username
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        username.insert_str(0, "web");
        username.truncate(24);
    }

    Some(username)
}

fn next_guest_username() -> String {
    static SESSION_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    format!(
        "web{:06}",
        SESSION_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

async fn handle_play_connection(
    socket: TcpStream,
    peer: SocketAddr,
    upstream: SocketAddr,
    username: String,
    max_sessions: usize,
    play_ping_interval: Duration,
) -> GatewayResult<()> {
    info!(%peer, %username, "browser play session connected");

    let websocket = accept_async(socket).await?;
    let (mut websocket_write, mut websocket_read) = websocket.split();
    let Some(_session_slot) = try_reserve_play_session(max_sessions) else {
        let message = json!({
            "type": "error",
            "message": "server full",
            "active_sessions": active_play_sessions(),
            "max_sessions": max_sessions,
        });
        websocket_write
            .send(Message::Text(message.to_string().into()))
            .await?;
        websocket_write.close().await?;
        warn!(%peer, %username, active_sessions = active_play_sessions(), max_sessions, "rejected full web play session");
        return Ok(());
    };

    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let session = play_session::start(upstream, username, outbound_tx);

    let browser_to_session = async {
        while let Some(message) = websocket_read.next().await {
            match message? {
                Message::Text(text) => {
                    match serde_json::from_str::<play_session::BrowserCommand>(&text) {
                        Ok(command) => {
                            if session.send(command).is_err() {
                                break;
                            }
                        },
                        Err(error) => warn!(%peer, %error, "invalid play command"),
                    }
                },
                Message::Close(_) => break,
                Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {},
            }
        }

        GatewayResult::Ok(())
    };

    let session_to_browser = async {
        let mut heartbeat = interval(play_ping_interval);

        loop {
            select! {
                message = outbound_rx.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    websocket_write.send(Message::Text(message.into())).await?;
                },
                _ = heartbeat.tick() => {
                    websocket_write.send(Message::Ping(Vec::new().into())).await?;
                },
            }
        }

        if let Err(error) = websocket_write.close().await {
            error!(%peer, %error, "failed to close play websocket");
        }

        GatewayResult::Ok(())
    };

    select! {
        result = browser_to_session => result?,
        result = session_to_browser => result?,
    }

    info!(%peer, "browser play session disconnected");
    Ok(())
}

struct PlaySessionSlot;

impl Drop for PlaySessionSlot {
    fn drop(&mut self) { ACTIVE_PLAY_SESSIONS.fetch_sub(1, Ordering::Relaxed); }
}

fn try_reserve_play_session(max_sessions: usize) -> Option<PlaySessionSlot> {
    loop {
        let active = ACTIVE_PLAY_SESSIONS.load(Ordering::Relaxed);
        if active >= max_sessions {
            return None;
        }

        if ACTIVE_PLAY_SESSIONS
            .compare_exchange_weak(active, active + 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return Some(PlaySessionSlot);
        }
    }
}

fn active_play_sessions() -> usize { ACTIVE_PLAY_SESSIONS.load(Ordering::Relaxed) }

async fn serve_http(
    mut socket: TcpStream,
    peer: SocketAddr,
    static_dir: &Path,
    upstream: SocketAddr,
    query_server: SocketAddr,
    max_sessions: usize,
    play_ping_interval: Duration,
) -> GatewayResult<()> {
    let mut request = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 1024];

    loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }

        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") || request.len() > 8192 {
            break;
        }
    }

    let request = String::from_utf8_lossy(&request);
    let Some(path) = request_path(&request) else {
        write_response(
            &mut socket,
            "400 Bad Request",
            "text/plain; charset=utf-8",
            b"Bad Request",
        )
        .await?;
        return Ok(());
    };

    if path == "/api/status" {
        let body = status_body(upstream, query_server, max_sessions, play_ping_interval).await?;
        write_response_no_store(
            &mut socket,
            "200 OK",
            "application/json; charset=utf-8",
            &body,
        )
        .await?;
        info!(%peer, "served status endpoint");
        return Ok(());
    }

    if path == "/api/health" {
        let health = health_body(upstream, query_server).await?;
        let status = if health.ok {
            "200 OK"
        } else {
            "503 Service Unavailable"
        };
        write_response_no_store(
            &mut socket,
            status,
            "application/json; charset=utf-8",
            &health.body,
        )
        .await?;
        info!(%peer, ok = health.ok, "served health endpoint");
        return Ok(());
    }

    let Some(file_path) = static_file_path(static_dir, path) else {
        write_response(
            &mut socket,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"Forbidden",
        )
        .await?;
        return Ok(());
    };

    match fs::read(&file_path).await {
        Ok(bytes) => {
            let content_type = content_type(&file_path);
            write_response(&mut socket, "200 OK", content_type, &bytes).await?;
            info!(%peer, path = %path, file = %file_path.display(), "served web asset");
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_response(
                &mut socket,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"Not Found",
            )
            .await?;
        },
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

struct HealthBody {
    ok: bool,
    body: Vec<u8>,
}

async fn status_body(
    upstream: SocketAddr,
    query_server: SocketAddr,
    max_sessions: usize,
    play_ping_interval: Duration,
) -> GatewayResult<Vec<u8>> {
    let readiness = readiness_status(upstream, query_server).await;
    let ready = readiness_healthy(&readiness);
    let body = json!({
        "service": "voxworld-web-gateway",
        "ready": ready,
        "play_websocket_path": "/play",
        "raw_websocket_path": "/ws",
        "web_sessions": {
            "active": active_play_sessions(),
            "max": max_sessions,
            "ping_interval_secs": play_ping_interval.as_secs(),
        },
        "upstream": {
            "tcp_addr": upstream.to_string(),
            "tcp_reachable": readiness.tcp_reachable,
        },
        "query_server": {
            "addr": query_server.to_string(),
            "result": readiness.query.body,
        },
    });

    Ok(serde_json::to_vec(&body)?)
}

async fn health_body(upstream: SocketAddr, query_server: SocketAddr) -> GatewayResult<HealthBody> {
    let readiness = readiness_status(upstream, query_server).await;
    let ok = readiness_healthy(&readiness);
    let body = json!({
        "ok": ok,
        "service": "voxworld-web-gateway",
        "upstream_tcp_reachable": readiness.tcp_reachable,
        "query_server_ok": readiness.query.ok,
    });

    Ok(HealthBody {
        ok,
        body: serde_json::to_vec(&body)?,
    })
}

fn readiness_healthy(readiness: &ReadinessStatus) -> bool { readiness.query.ok }

struct ReadinessStatus {
    query: QueryStatus,
    tcp_reachable: bool,
}

async fn readiness_status(upstream: SocketAddr, query_server: SocketAddr) -> ReadinessStatus {
    let query = query_server_status(query_server).await;
    let tcp_reachable = if query.ok {
        true
    } else {
        matches!(
            timeout(Duration::from_millis(500), TcpStream::connect(upstream)).await,
            Ok(Ok(_))
        )
    };

    ReadinessStatus {
        query,
        tcp_reachable,
    }
}

struct QueryStatus {
    ok: bool,
    body: Value,
}

async fn query_server_status(query_server: SocketAddr) -> QueryStatus {
    let mut client = QueryClient::new(query_server);

    match timeout(Duration::from_millis(1500), client.server_info()).await {
        Ok(Ok((info, latency))) => QueryStatus {
            ok: true,
            body: json!({
                "ok": true,
                "players": info.players_count,
                "player_cap": info.player_cap,
                "git_hash": info.git_hash,
                "git_timestamp": info.git_timestamp,
                "battlemode": format!("{:?}", info.battlemode),
                "latency_ms": latency.as_millis(),
            }),
        },
        Ok(Err(error)) => QueryStatus {
            ok: false,
            body: json!({
                "ok": false,
                "error": format!("{error:?}"),
            }),
        },
        Err(_) => QueryStatus {
            ok: false,
            body: json!({
                "ok": false,
                "error": "timeout",
            }),
        },
    }
}

fn request_path(request: &str) -> Option<&str> {
    let request_line = request.lines().next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let raw_path = parts.next()?;

    if method != "GET" && method != "HEAD" {
        return None;
    }

    Some(raw_path.split('?').next().unwrap_or(raw_path))
}

fn static_file_path(static_dir: &Path, request_path: &str) -> Option<PathBuf> {
    if request_path.contains("..") || request_path.contains('\\') {
        return None;
    }

    let relative = request_path.trim_start_matches('/');
    let relative = if relative.is_empty() {
        "index.html"
    } else {
        relative
    };

    Some(static_dir.join(relative))
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn write_response(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> GatewayResult<()> {
    write_response_with_cache(socket, status, content_type, body, "public, max-age=60").await
}

async fn write_response_no_store(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> GatewayResult<()> {
    write_response_with_cache(socket, status, content_type, body, "no-store").await
}

async fn write_response_with_cache(
    socket: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    cache_control: &str,
) -> GatewayResult<()> {
    let headers = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: \
         {}\r\ncache-control: {cache_control}\r\nconnection: close\r\n\r\n",
        body.len(),
    );

    socket.write_all(headers.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{QueryStatus, ReadinessStatus, readiness_healthy};

    #[test]
    fn health_requires_query_server_success() {
        let tcp_only = ReadinessStatus {
            query: QueryStatus {
                ok: false,
                body: json!({"ok": false}),
            },
            tcp_reachable: true,
        };
        let query_ready = ReadinessStatus {
            query: QueryStatus {
                ok: true,
                body: json!({"ok": true}),
            },
            tcp_reachable: false,
        };

        assert!(!readiness_healthy(&tcp_only));
        assert!(readiness_healthy(&query_ready));
    }
}
