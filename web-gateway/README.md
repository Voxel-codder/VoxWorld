# Vox World Web Gateway

This is an early HTTP and WebSocket gateway for the browser client. It serves
`web-client/web` over HTTP and forwards `/ws` browser WebSocket connections to
the native Vox World TCP server port.

It is a bridge, not the final networking layer. The full web port still needs a
browser-compatible transport implementation that can speak the game protocol
cleanly over WebSocket or WebTransport.

## Run

Start the native server first, then run:

```powershell
cargo run -p voxworld-web-gateway -- --listen 127.0.0.1:14080 --upstream 127.0.0.1:14004
```

Open the web client at:

```text
http://localhost:14080
```

The page automatically points its WebSocket connection at `/ws` on the same host.
