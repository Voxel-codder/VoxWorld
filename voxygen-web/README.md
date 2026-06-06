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
- greedily merge same-height, same-color generated terrain top faces inside
  each chunk so the browser mesh begins moving away from one-quad-per-block
  terrain and toward Voxygen-style terrain meshing;
- cache generated original `TerrainChunk` data, supplements, and chunk-local
  block-face mesh fragments so moving across chunk boundaries only generates
  newly visible chunks and meshes;
- upload visible terrain as per-chunk WebGPU vertex/index buffers and cache
  those buffers across chunk-boundary movement;
- evict generated chunk/mesh fragments outside a small player-centered
  retention radius and drop WebGPU terrain buffers for chunks that are no
  longer visible, keeping the browser preview on a bounded streaming path
  instead of accumulating every visited chunk;
- prefetch one retained shell of original chunks in the browser-side player's
  facing direction when the visible terrain patch is regenerated, reducing the
  next chunk-boundary miss while staying inside the bounded cache radius;
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
- rotate the browser-side third-person camera with mouse drag and reuse that
  camera basis for movement and chunk prefetch direction, moving the preview
  closer to original Voxygen play controls instead of a fixed-angle flyover;
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
- generate original rtsim data for the preview world, report existing and
  wanted population counts, and surface merchant/guard demand in the browser
  status panel;
- derive temporary visible trader, guard, captain, adventurer, and market-board
  markers from original settlement plots using an rtsim-style profession roster
  plus original service plots, so starter towns no longer render as empty
  terrain plus only waypoint supplements;
- include those rtsim-style site NPC/trader/market markers in nearest-target
  reporting and `E`/`Enter` interaction previews, including market-board trade
  preview status text;
- calculate site trader/market-board preview stock and representative buy/sell
  prices from original `SitePrices` and `TradePricing` data so the browser
  interaction path now uses original economy pricing instead of static status
  copy;
- open a browser-side trade panel when `E`/`Enter` targets a trader or market
  board, showing the selected town, site stock, representative wares, quality,
  and buy/sell coin prices from the original economy path;
- track a browser-side pending-trade preview state backed by the original
  `common::trade::PendingTrade` phase and accept flow, with selected merchant
  wares, a player coin offer, and buy/sell balance text before
  server-authoritative inventory actions are attached;
- build preview player/merchant `Inventory` values from the displayed wares and
  coins, then route preview offers through original `TradeAction::AddItem`
  processing so offer quantities are clamped by real inventory slot contents;
- when both preview parties accept, atomically commit the completed browser-side
  trade against cloned player/merchant `Inventory` values using original
  `Inventory::take_amount` and `Inventory::push_all` item transfer paths, then
  display the resulting inventory slot counts;
- derive browser-side trader/market preview wares from original
  `TradePricing::random_items` against site stock, using the server trader
  stock adjustments, profession-style good filters, quality filters, and stack
  amounts instead of fixed sample-only item lists;
- preserve original `EntityInfo.body` categories in the web preview and render
  body-aware temporary silhouettes for humanoid, quadruped, flyer, fish, large,
  and object entities;
- store marker yaw and rotate temporary player/entity silhouette parts so the
  browser-side player visibly faces the last intended movement direction;
- orient generated entity and site NPC/trader/market markers with stable yaw
  derived from original spawn positions and settlement plot positions instead
  of rendering every temporary silhouette at yaw 0;
- keep this as the stable place for future Voxygen renderer/HUD migration;
- avoid extending the temporary 2D canvas client as if it were the final game.

Next milestones:

- extend greedy terrain meshing beyond top faces, then replace the temporary
  block material colors with Voxygen's real atlas/material pipeline;
- replace oriented entity and player temporary silhouettes with Voxygen body
  meshes, loadouts, and animation state;
- replace the temporary rtsim-style profession roster markers with direct
  original rtsim NPC agents, profession inventories, dialogue, and loaded-agent
  state;
- replace the browser-side committed-trade preview panel with the real Voxygen
  trade HUD and server-authoritative trade actions against live inventories;
- expand the bounded 5x5 chunk-streaming path and single-direction prefetch
  into Voxygen's full terrain loading scheduler with priority queues,
  cancellation, and independent draw management around the live player
  position;
- attach player/session state so the scene follows the live character instead
  of a fixed terrain camera;
- introduce a browser-safe client transport that maps Voxygen networking onto a
  WebSocket/WebTransport gateway;
- split native-only Voxygen modules such as desktop audio, filesystem dialogs,
  Discord integration, and direct TCP/UDP from the web build;
- move Voxygen's renderer, scene, and HUD setup behind a reusable web entrypoint.
