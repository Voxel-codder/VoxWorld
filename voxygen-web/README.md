# Voxygen Web Port

This crate is the browser-native porting target for the original Voxygen
client. It is intentionally separate from `web-client`, which was only a
minimal JSON/WebSocket play surface.

The goal here is to move the real Voxygen renderer, scene, HUD, asset loading,
input, and server connection path into the browser with WebGPU/WASM while
keeping the native client intact.

Current milestone:

- boot a browser canvas through the same WebGPU family used by Voxygen;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
