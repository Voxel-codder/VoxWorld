# Vox World Web Gateway

This is an early HTTP and WebSocket gateway for the browser client. It serves
`web-client/web` over HTTP and hosts `/play` browser sessions backed by the
native `veloren-client` crate.

For each `/play` WebSocket, the gateway starts a headless native client session,
auto-creates or loads a character, ticks the client, accepts browser JSON input,
and sends browser-friendly JSON session messages back to the page. `/ws` remains
available as a raw WebSocket-to-TCP proxy for lower-level transport experiments.

## Run

Start the native server first, then run:

```powershell
cargo run -p voxworld-web-gateway -- --listen 127.0.0.1:14080 --upstream 127.0.0.1:14004
```

Open the web client at:

```text
http://localhost:14080
```

The page automatically points its WebSocket connection at `/play` on the same
host.

## Browser Protocol

The browser sends input messages such as:

```json
{"type":"input","move_x":0,"move_y":1,"move_z":0,"look_x":0,"look_y":1,"look_z":0}
```

The gateway responds with stage, snapshot, event, and error messages. Snapshots
currently include username, in-game state, position, player names, and character
count.
