use std::{
    error::Error,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    select,
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
        static_dir = %args.static_dir.display(),
        "web gateway listening"
    );

    loop {
        let (socket, peer) = listener.accept().await?;
        let upstream = args.upstream;
        let static_dir = args.static_dir.clone();

        tokio::spawn(async move {
            if let Err(error) = handle_connection(socket, peer, upstream, static_dir).await {
                warn!(%peer, %error, "web gateway connection ended");
            }
        });
    }
}

async fn handle_connection(
    socket: TcpStream,
    peer: SocketAddr,
    upstream: SocketAddr,
    static_dir: PathBuf,
) -> GatewayResult<()> {
    let mut peek = [0_u8; 2048];
    let read = socket.peek(&mut peek).await?;
    let request_head = String::from_utf8_lossy(&peek[..read]);

    if !is_websocket_request(&request_head) {
        return serve_static(socket, peer, &static_dir).await;
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

async fn serve_static(
    mut socket: TcpStream,
    peer: SocketAddr,
    static_dir: &Path,
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
    let headers = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: \
         {}\r\ncache-control: public, max-age=60\r\nconnection: close\r\n\r\n",
        body.len()
    );

    socket.write_all(headers.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.shutdown().await?;
    Ok(())
}
