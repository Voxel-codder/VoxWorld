# Voxygen Web Port

This crate is the browser-native porting target for the original Voxygen
client. It is intentionally separate from `web-client`, which was only a
minimal JSON/WebSocket play surface.

The goal here is to move the real Voxygen renderer, scene, HUD, asset loading,
input, and server connection path into the browser with WebGPU/WASM while
keeping the native client intact.

Current milestone:

- compile the original `common` and `world` crates for `wasm32-unknown-unknown`;
- load the first original world asset manifests from a WASM embedded
  `common-assets` source;
- generate a small original `WorldSim` in the browser build path;
- render that original chunk simulation as a WebGPU 3D terrain mesh with a
  camera, vertex/index buffers, and depth testing;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- use the embedded world manifests to call the real `World::generate_chunk`
  path in the web scene;
- replace the WorldSim overview mesh with Voxygen's real terrain chunk/block
  meshing path;
- attach player/session state so the scene follows the live character instead
  of a fixed terrain camera;
- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
