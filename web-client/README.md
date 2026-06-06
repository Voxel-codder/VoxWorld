# Vox World Web Client

This crate is the first browser/WASM porting surface for Vox World. It does not
replace the native client yet. It gives us a separate build target where browser
platform work can land without disturbing the production server or native client.

## Current Scope

- Browser boot path through `wasm-bindgen`
- Canvas bootstrap for the future renderer
- WebSocket connection shell for the `/play` headless session transport
- Static web page that can be hosted by any HTTP server
- Same-origin `/play` connection default for Railway and local gateway hosting
- Keyboard input forwarding for WASD, E/Q vertical movement, Space jump, Shift roll, and basic combat keys
- Mouse and touch look-direction forwarding from the canvas
- Mouse primary and secondary action forwarding from the canvas
- On-screen touch movement and action controls for mobile browsers
- Snapshot-driven canvas view for the player, nearby entities, health, and energy
- Browser-persisted guest name passed through `/play?name=...`
- Browser chat input and session event log over the `/play` transport

## Build

From the repository root:

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
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

1. Expand `/play` snapshots from dot-level entity positions to richer entity state.
2. Add browser-side character naming and basic account/session UX.
3. Replace the canvas snapshot view with the real renderer path.
4. Add asset manifest loading, compression, caching, and progressive download.
