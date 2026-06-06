# Vox World Web Gateway

This is an early HTTP and WebSocket gateway for the browser client. It serves
`voxygen-web/web` over HTTP by default and hosts `/play` browser sessions backed
by the native `veloren-client` crate.

For each `/play` WebSocket, the gateway starts a headless native client session,
auto-creates or loads a character, ticks the client, accepts browser JSON input,
and sends browser-friendly JSON session messages back to the page. `/ws` remains
available as a raw WebSocket-to-TCP proxy for lower-level transport experiments.
Browsers may pass `?name=guest_name` on `/play`; the gateway sanitizes that name
before using it for the native session. If multiple active browser sessions ask
for the same name, later sessions get a short suffix such as `guest_name-2`.

The gateway defaults to 100 active `/play` sessions, matching the server's
default player cap. Override it with `--max-sessions` or
`VOXWORLD_WEB_MAX_SESSIONS`. Browser play sessions receive WebSocket pings every
30 seconds by default; override that with `--play-ping-interval-secs` or
`VOXWORLD_WEB_PING_INTERVAL_SECS`. A browser play session must finish character
setup and enter the game within 90 seconds, otherwise the gateway sends a clear
error message and closes the session.

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

and one-shot control actions:

```json
{"type":"control","control":"interact"}
```

The gateway responds with stage, snapshot, chat, event, and error messages.
Snapshots currently include username, in-game state, position, health, energy,
defeated state,
inventory summary, the nearest pickup/NPC interaction hint, online player names,
NPC/pickup/enemy labels, character count, and up to 96 nearby meaningful entity
positions with available health, including visible targets that do not have a
synced UID.
Chat messages include a browser-friendly scope, optional sender name, and
message text.
`/api/status` also reports readiness, active and maximum browser play sessions,
plus the configured play-session ping interval. `/api/health` is only healthy
after the native query server responds, so deploy checks wait for a playable
server rather than only an open TCP port.
