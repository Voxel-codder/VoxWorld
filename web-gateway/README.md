# Vox World Web Gateway

This is an early HTTP and WebSocket gateway for the browser client. It serves
`web-client/web` over HTTP and hosts `/play` browser sessions backed by the
native `veloren-client` crate.

For each `/play` WebSocket, the gateway starts a headless native client session,
auto-creates or loads a character, ticks the client, accepts browser JSON input,
and sends browser-friendly JSON session messages back to the page. `/ws` remains
available as a raw WebSocket-to-TCP proxy for lower-level transport experiments.
Browsers may pass `?name=guest_name` on `/play`; the gateway sanitizes that name
before using it for the native session.

The gateway defaults to 100 active `/play` sessions, matching the server's
default player cap. Override it with `--max-sessions` or
`VOXWORLD_WEB_MAX_SESSIONS`.

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

and chat messages such as:

```json
{"type":"chat","message":"hello"}
```

It can also forward basic gameplay actions:

```json
{"type":"action","action":"primary","pressed":true}
```

The gateway responds with stage, snapshot, event, and error messages. Snapshots
currently include username, in-game state, position, health, energy, online
player names, character count, and up to 96 nearby entity positions.
`/api/status` also reports active and maximum browser play sessions.
