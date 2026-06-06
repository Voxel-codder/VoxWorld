# Vox World Web Client

This crate is a temporary legacy prototype. It is not the target for the real
browser version of Vox World because it does not render the original Voxygen
scene/HUD. New browser-native work should go into `voxygen-web`.

## Current Scope

- Browser boot path through `wasm-bindgen`
- Canvas bootstrap for the future renderer
- WebSocket connection shell for the `/play` headless session transport
- Static web page that can be hosted by any HTTP server
- Same-origin `/play` connection default for Railway and local gateway hosting
- Server status display waits for query-backed readiness and includes active browser sessions against the web session cap
- Online player names update from live `/play` snapshots
- Keyboard input forwarding for WASD movement, PageUp/PageDown vertical movement, Space jump, Shift roll, E interact, R wield, Tab loadout swap, and basic combat keys
- Mouse and touch look-direction forwarding from the canvas
- Mouse primary and secondary action forwarding from the canvas
- On-screen touch movement, combat, interact, pickup, wield, loadout, sneak, sit, and respawn controls for mobile browsers
- Snapshot-driven canvas view for the player, nearby players, NPCs, pickups, enemies, health, entity health, and energy
- Defeated-state respawn hint from `/play` snapshots
- Inventory summary HUD from `/play` snapshots
- Nearby pickup/NPC interaction hint from `/play` snapshots
- Browser-persisted guest name passed through `/play?name=...`
- Browser chat input and session event log over the `/play` transport
- Structured player chat display for messages received from the game server
- Session setup errors stay visible after the WebSocket closes
- Duplicate connection cleanup and focus-loss input release for browser sessions

## Build

From the repository root:

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.106 --locked
.\web-client\scripts\build-wasm.ps1
python -m http.server 8080 --directory web-client\web
```

Then open:

```text
http://localhost:8080
```

For a local gateway that matches the Railway deployment shape:

```powershell
cargo run -p voxworld-web-gateway -- --listen 127.0.0.1:14080 --upstream 127.0.0.1:14004
```

Then open:

```text
http://localhost:14080
```

## Porting Roadmap

The real browser-native port now lives in `voxygen-web`, where the work can move
toward Voxygen's renderer, scene, HUD, asset loading, input, and browser-safe
network transport without growing this prototype further.
