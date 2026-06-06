# Vox World Web Client

This crate is the first browser/WASM porting surface for Vox World. It does not
replace the native client yet. It gives us a separate build target where browser
platform work can land without disturbing the production server or native client.

## Current Scope

- Browser boot path through `wasm-bindgen`
- Canvas bootstrap for the future renderer
- WebSocket connection shell for the future game transport
- Static web page that can be hosted by any HTTP server
- Same-origin `/ws` connection default for Railway and local gateway hosting

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

1. Add a WebSocket transport to the shared network layer.
2. Connect this WASM shell to a small gateway or server-side WebSocket endpoint.
3. Move enough client state into browser-compatible crates to enter the world.
4. Replace the canvas placeholder with the real renderer path.
5. Add asset manifest loading, compression, caching, and progressive download.
