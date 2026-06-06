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
- keep the generated original `World` and index data alive in the browser so
  new terrain patches can be generated without rebuilding the whole simulation;
- call the real `World::generate_chunk` path for a 5x5 patch around the preview
  camera and convert the generated `TerrainChunk` blocks into a merged WebGPU
  3D block-face mesh with a camera, vertex/index buffers, and depth testing;
- track a browser-side player block position with continuous keyboard movement,
  update the camera every animation frame, and upload a regenerated terrain
  patch when the player crosses a chunk boundary;
- frame the WebGPU scene with a player-following third-person camera instead
  of the earlier far-away terrain-preview view;
- convert original `ChunkSupplement.entity_spawns` into visible 3D markers and
  render them alongside a browser-side player marker;
- preserve original `EntityInfo.body` categories in the web preview and render
  body-aware temporary silhouettes for humanoid, quadruped, flyer, fish, large,
  and object entities;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- replace the temporary block-face mesh with Voxygen's real greedy terrain mesh
  and atlas/material pipeline;
- replace entity and player temporary silhouettes with Voxygen body meshes,
  loadouts, and animation state;
- replace the regenerated 5x5 patch with incremental chunk streaming around the
  live player position;
- attach player/session state so the scene follows the live character instead
  of a fixed terrain camera;
- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
