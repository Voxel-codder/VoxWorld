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
- embed the original world `.vox` structure assets needed by terrain
  decorations;
- generate a small original `WorldSim` in the browser build path;
- run original spot placement and enable terrain decorations, trees, shrubs, and
  spot structures in the generated preview chunk;
- call the real `World::generate_chunk` path for a 3x3 patch around the preview
  camera and convert the generated `TerrainChunk` blocks into a merged WebGPU
  3D block-face mesh with a camera, vertex/index buffers, and depth testing;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- replace the temporary block-face mesh with Voxygen's real greedy terrain mesh
  and atlas/material pipeline;
- replace the fixed 3x3 preview patch with streaming generated chunks around the
  live camera/player position;
- attach player/session state so the scene follows the live character instead
  of a fixed terrain camera;
- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
