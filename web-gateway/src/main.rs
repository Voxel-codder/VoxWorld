use std::{
    error::Error,
    net::SocketAddr,
    path::{Path, PathBuf},
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
    time::{Duration, timeout},
};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{error, info, warn};

type GatewayResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

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
        "web gateway listening"
    );

    loop {
        let (socket, peer) = listener.accept().await?;
        let upstream = args.upstream;
        let query_server = args.query_server;
        let static_dir = args.static_dir.clone();

        tokio::spawn(async move {
            if let Err(error) =
                handle_connection(socket, peer, upstream, query_server, static_dir).await
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
) -> GatewayResult<()> {
    let mut peek = [0_u8; 2048];
    let read = socket.peek(&mut peek).await?;
    let request_head = String::from_utf8_lossy(&peek[..read]);

    if !is_websocket_request(&request_head) {
        return serve_http(socket, peer, &static_dir, upstream, query_server).await;
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
    let mut lines = request_head.lines();
    let request_line = lines.next().unwrap_or_default();

    request_line.starts_with("GET /ws ")
        && request_head
            .lines()
            .any(|line| line.eq_ignore_ascii_case("upgrade: websocket"))
}

async fn serve_http(
    mut socket: TcpStream,
    peer: SocketAddr,
    static_dir: &Path,
    upstream: SocketAddr,
    query_server: SocketAddr,
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
        let body = status_body(upstream, query_server).await?;
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

async fn status_body(upstream: SocketAddr, query_server: SocketAddr) -> GatewayResult<Vec<u8>> {
    let query_result = query_server_status(query_server).await;
    let tcp_reachable = if query_result.ok {
        true
    } else {
        matches!(
            timeout(Duration::from_millis(500), TcpStream::connect(upstream)).await,
            Ok(Ok(_))
        )
    };
    let body = json!({
        "service": "voxworld-web-gateway",
        "websocket_path": "/ws",
        "upstream": {
            "tcp_addr": upstream.to_string(),
            "tcp_reachable": tcp_reachable,
        },
        "query_server": {
            "addr": query_server.to_string(),
            "result": query_result.body,
        },
    });

    Ok(serde_json::to_vec(&body)?)
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
