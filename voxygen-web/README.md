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
- embed original common RON assets so browser world generation can load economy,
  profession, trading, item, recipe, ability, and NPC metadata;
- generate a small original `World` in the browser build path using the real
  civ/site/economy/spot generation pipeline instead of only a terrain-only
  `WorldSim`;
- choose the browser preview's initial player/chunk position from the generated
  original settlement sites so the first frame lands near a real town instead
  of an arbitrary world-center terrain patch;
- keep the generated original `World`, civ/site data, and index data alive in
  the browser so new terrain patches can be generated without rebuilding the
  whole simulation;
- call the real `World::generate_chunk` path for a 5x5 patch around the preview
  camera and convert the generated `TerrainChunk` blocks into a merged WebGPU
  3D block-face mesh with a camera, vertex/index buffers, and depth testing;
- cache generated original `TerrainChunk` data, supplements, and chunk-local
  block-face mesh fragments so moving across chunk boundaries only generates
  newly visible chunks and meshes;
- upload visible terrain as per-chunk WebGPU vertex/index buffers and cache
  those buffers across chunk-boundary movement;
- track a browser-side player block position with continuous keyboard movement,
  update the camera every animation frame, and upload a regenerated terrain
  patch when the player crosses a chunk boundary;
- ground the browser-side player marker and camera target using cached original
  `TerrainChunk::find_accessible_pos` samples instead of only approximate
  `WorldSim` surface altitude;
- constrain browser-side movement with cached accessible terrain z samples so
  steep steps and drops block or slide movement instead of allowing flat-plane
  traversal through every slope;
- map keyboard movement through the current third-person camera basis so WASD
  movement follows the rendered play view instead of fixed world axes;
- frame the WebGPU scene with a player-following third-person camera instead
  of the earlier far-away terrain-preview view;
- convert original `ChunkSupplement.entity_spawns` into visible 3D markers and
  render them alongside a browser-side player marker;
- convert non-empty original terrain `SpriteKind` blocks into temporary 3D
  prop meshes so plants, resources, containers, lamps, and structural sprites
  from generated chunks are visible in the browser preview;
- cache original terrain sprite prop metadata and report the nearest
  collectable/interactable terrain prop and entity target around the
  browser-side player;
- handle browser-side `E`/`Enter` interaction requests against the cached
  original terrain prop and entity targets, including range checks and a
  persistent last-interaction status line;
- report original site, settlement, POI, and selected starting-site metadata in
  the browser status panel for validation;
- derive temporary visible NPC, trader, captain, guard, villager, and market
  board markers from the generated original site plots so starter towns no
  longer render as empty terrain plus only waypoint supplements;
- include those site-derived NPC/trader/market markers in nearest-target
  reporting and `E`/`Enter` interaction previews, including market-board trade
  preview status text;
- preserve original `EntityInfo.body` categories in the web preview and render
  body-aware temporary silhouettes for humanoid, quadruped, flyer, fish, large,
  and object entities;
- store marker yaw and rotate temporary player/entity silhouette parts so the
  browser-side player visibly faces the last intended movement direction;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- replace the temporary block-face mesh with Voxygen's real greedy terrain mesh
  and atlas/material pipeline;
- replace entity and player temporary silhouettes with Voxygen body meshes,
  loadouts, and animation state;
- replace the temporary site-derived NPC/trader/market markers with direct
  original rtsim NPC data, profession inventories, dialogue, and loaded-agent
  state;
- connect market-board and trader previews to real inventory/trade UI data
  instead of status-line-only interaction feedback;
- replace the visible 5x5 GPU chunk-buffer set with fully streamed loading,
  eviction, and independent chunk draw management around the live player
  position;
- attach player/session state so the scene follows the live character instead
  of a fixed terrain camera;
- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
