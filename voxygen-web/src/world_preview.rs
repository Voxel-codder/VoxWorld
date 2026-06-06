use std::collections::HashMap;

use common::{
    comp::{Alignment, Body},
    generation::{EntityInfo, EntitySpawn, SpecialEntity},
    spot::Spot,
    terrain::{Block, TerrainChunk, TerrainChunkSize},
    vol::{ReadVol, RectVolSize},
};
use rayon::ThreadPoolBuilder;
use vek::{Vec2, Vec3};
use veloren_world::{
    World,
    config::Features,
    index::{Index, IndexOwned, IndexRef},
    layer::spot::SpotGenerate,
    sim::{FileOpts, GenOpts, WorldOpts, WorldSim},
};

pub const FLOATS_PER_VERTEX: usize = 6;
pub const TERRAIN_HORIZONTAL_SCALE: f32 = 0.64;
const SEED: u32 = 7;
const MAP_LG: u32 = 5;
const CHUNK_RADIUS: i32 = 2;

pub struct OriginalWorldPreview {
    world: World,
    index_owned: IndexOwned,
    preview_features: Features,
    chunk_cache: HashMap<(i32, i32), GeneratedPreviewChunk>,
    dimensions: Vec2<u32>,
    enabled_world_features: usize,
    wildlife_spawn_manifests: usize,
    seed: u32,
}

pub struct OriginalWorldMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub entity_markers: Vec<OriginalEntityMarker>,
    pub chunk_dimensions: (u32, u32),
    pub center_chunk_pos: (i32, i32),
    pub chunk_patch: (u32, u32),
    pub vertical_origin: f32,
    pub generated_chunks: usize,
    pub newly_generated_chunks: usize,
    pub cached_chunks: usize,
    pub terrain_faces: usize,
    pub filled_blocks: usize,
    pub liquid_blocks: usize,
    pub generated_entity_spawns: usize,
    pub enabled_world_features: usize,
    pub wildlife_spawn_manifests: usize,
    pub seed: u32,
}

#[derive(Clone, Copy)]
pub struct OriginalEntityMarker {
    pub render_pos: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub color: [f32; 3],
    pub shape: OriginalEntityMarkerShape,
}

#[derive(Clone, Copy)]
pub enum OriginalEntityMarkerShape {
    Humanoid,
    Quadruped,
    Flyer,
    Fish,
    Large,
    Object,
}

impl OriginalWorldPreview {
    pub fn new() -> Result<Self, String> {
        let threadpool = ThreadPoolBuilder::new()
            .num_threads(1)
            .use_current_thread()
            .build()
            .map_err(|error| format!("failed to create browser worldgen threadpool: {error}"))?;
        let mut sim = WorldSim::generate(
            SEED,
            WorldOpts {
                seed_elements: true,
                world_file: FileOpts::Generate(GenOpts {
                    x_lg: MAP_LG,
                    y_lg: MAP_LG,
                    ..GenOpts::default()
                }),
                calendar: None,
            },
            &threadpool,
            &|_| {},
        );
        Spot::generate(&mut sim);
        let dimensions = sim.get_size();
        if dimensions.x < 2 || dimensions.y < 2 {
            return Err("original WorldSim generated too few chunks for terrain mesh".to_owned());
        }

        let world = World::from_sim_for_web_preview(sim);
        let index_owned = IndexOwned::new(Index::new(SEED));
        let index_ref = index_owned.as_index_ref();
        let enabled_world_features = count_enabled_features(index_ref.features);
        let wildlife_spawn_manifests = index_ref.wildlife_spawns.len();
        let preview_features = terrain_chunk_preview_features(index_ref.features);

        Ok(Self {
            world,
            index_owned,
            preview_features,
            chunk_cache: HashMap::new(),
            dimensions,
            enabled_world_features,
            wildlife_spawn_manifests,
            seed: SEED,
        })
    }

    pub fn initial_center_chunk_pos(&self) -> Vec2<i32> {
        Vec2::new(self.dimensions.x as i32 / 2, self.dimensions.y as i32 / 2)
    }

    pub fn initial_player_wpos(&self) -> Vec2<f32> {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
        self.clamp_player_wpos(
            chunk_center_wpos(self.initial_center_chunk_pos())
                + Vec2::new(rect_size.x * 0.28, -rect_size.y * 0.22),
        )
    }

    pub fn center_chunk_for_wpos(&self, wpos: Vec2<f32>) -> Vec2<i32> {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
        self.clamp_center_chunk_pos(Vec2::new(
            (wpos.x / rect_size.x).floor() as i32,
            (wpos.y / rect_size.y).floor() as i32,
        ))
    }

    pub fn clamp_center_chunk_pos(&self, chunk_pos: Vec2<i32>) -> Vec2<i32> {
        let max_world_x = self.dimensions.x.saturating_sub(1) as i32;
        let max_world_y = self.dimensions.y.saturating_sub(1) as i32;
        let min_x = CHUNK_RADIUS.min(max_world_x);
        let min_y = CHUNK_RADIUS.min(max_world_y);
        let max_x = (max_world_x - CHUNK_RADIUS).max(min_x);
        let max_y = (max_world_y - CHUNK_RADIUS).max(min_y);

        Vec2::new(
            chunk_pos.x.clamp(min_x, max_x),
            chunk_pos.y.clamp(min_y, max_y),
        )
    }

    pub fn clamp_player_wpos(&self, wpos: Vec2<f32>) -> Vec2<f32> {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
        let max = self.dimensions.as_::<f32>() * rect_size - Vec2::broadcast(1.0);
        Vec2::new(wpos.x.clamp(0.0, max.x), wpos.y.clamp(0.0, max.y))
    }

    pub fn player_render_position(
        &self,
        player_wpos: Vec2<f32>,
        center_chunk_pos: Vec2<i32>,
        vertical_origin: f32,
    ) -> [f32; 3] {
        let center = chunk_center_wpos(center_chunk_pos);
        let delta = player_wpos - center;
        let sample_wpos = Vec2::new(player_wpos.x.floor() as i32, player_wpos.y.floor() as i32);
        let surface_alt = self.world.sim().get_surface_alt_approx(sample_wpos);
        [
            delta.x * TERRAIN_HORIZONTAL_SCALE,
            (surface_alt - vertical_origin) * 0.28 + 0.25,
            delta.y * TERRAIN_HORIZONTAL_SCALE,
        ]
    }

    pub fn generate_mesh(
        &mut self,
        center_chunk_pos: Vec2<i32>,
    ) -> Result<OriginalWorldMesh, String> {
        let center_chunk_pos = self.clamp_center_chunk_pos(center_chunk_pos);
        let required_chunk_positions = required_chunk_positions(center_chunk_pos, self.dimensions);
        let mut newly_generated_chunks = 0usize;

        for chunk_pos in &required_chunk_positions {
            let key = chunk_key(*chunk_pos);
            if self.chunk_cache.contains_key(&key) {
                continue;
            }

            let (terrain_chunk, supplement) = {
                let index_ref = self.index_owned.as_index_ref();
                let preview_index = IndexRef {
                    features: &self.preview_features,
                    ..index_ref
                };
                self.world
                    .generate_chunk(preview_index, *chunk_pos, None, || false, None)
                    .map_err(|_| {
                        format!("original World::generate_chunk cancelled at {chunk_pos:?}")
                    })?
            };
            self.chunk_cache.insert(key, GeneratedPreviewChunk {
                pos: *chunk_pos,
                chunk: terrain_chunk,
                entity_spawns: supplement.entity_spawns,
            });
            newly_generated_chunks += 1;
        }

        let preview_chunks = required_chunk_positions
            .iter()
            .filter_map(|chunk_pos| self.chunk_cache.get(&chunk_key(*chunk_pos)))
            .collect::<Vec<_>>();

        mesh_from_terrain_chunks(
            &preview_chunks,
            self.dimensions,
            center_chunk_pos,
            CHUNK_RADIUS,
            newly_generated_chunks,
            self.chunk_cache.len(),
            self.seed,
            self.enabled_world_features,
            self.wildlife_spawn_manifests,
        )
    }
}

fn required_chunk_positions(center_chunk_pos: Vec2<i32>, dimensions: Vec2<u32>) -> Vec<Vec2<i32>> {
    let mut chunk_positions = Vec::new();
    for y in -CHUNK_RADIUS..=CHUNK_RADIUS {
        for x in -CHUNK_RADIUS..=CHUNK_RADIUS {
            let chunk_pos = center_chunk_pos + Vec2::new(x, y);
            if chunk_pos.x < 0
                || chunk_pos.y < 0
                || chunk_pos.x >= dimensions.x as i32
                || chunk_pos.y >= dimensions.y as i32
            {
                continue;
            }
            chunk_positions.push(chunk_pos);
        }
    }
    chunk_positions
}

fn chunk_key(chunk_pos: Vec2<i32>) -> (i32, i32) { (chunk_pos.x, chunk_pos.y) }

pub fn build_original_world_preview() -> Result<OriginalWorldPreview, String> {
    OriginalWorldPreview::new()
}

#[allow(clippy::struct_excessive_bools)]
fn terrain_chunk_preview_features(features: &Features) -> Features {
    Features {
        caverns: features.caverns,
        caves: features.caves,
        rocks: features.rocks,
        shrubs: features.shrubs,
        trees: features.trees,
        scatter: features.scatter,
        paths: features.paths,
        spots: features.spots,
        wildlife_density: features.wildlife_density,
        peak_naming: features.peak_naming,
        biome_naming: features.biome_naming,
        train_tracks: false,
    }
}

struct GeneratedPreviewChunk {
    pos: Vec2<i32>,
    chunk: TerrainChunk,
    entity_spawns: Vec<EntitySpawn>,
}

fn mesh_from_terrain_chunks(
    chunks: &[&GeneratedPreviewChunk],
    dimensions: Vec2<u32>,
    center_chunk_pos: Vec2<i32>,
    chunk_radius: i32,
    newly_generated_chunks: usize,
    cached_chunks: usize,
    seed: u32,
    enabled_world_features: usize,
    wildlife_spawn_manifests: usize,
) -> Result<OriginalWorldMesh, String> {
    if chunks.is_empty() {
        return Err("original WorldSim generated no preview chunks".to_owned());
    }

    let min_z = chunks
        .iter()
        .map(|preview_chunk| preview_chunk.chunk.get_min_z())
        .min()
        .ok_or_else(|| "original preview chunks have no minimum z".to_owned())?;
    let max_z = chunks
        .iter()
        .map(|preview_chunk| preview_chunk.chunk.get_max_z())
        .max()
        .ok_or_else(|| "original preview chunks have no maximum z".to_owned())?;
    if max_z <= min_z {
        return Err(format!(
            "original TerrainChunk patch around {center_chunk_pos:?} has no vertical span"
        ));
    }

    let mut builder = BlockMeshBuilder::new((min_z + max_z) as f32 * 0.5);
    let mut filled_blocks = 0usize;
    let mut liquid_blocks = 0usize;
    let mut generated_entity_spawns = 0usize;
    let mut entity_markers = Vec::new();
    let vertical_origin = (min_z + max_z) as f32 * 0.5;

    for preview_chunk in chunks {
        generated_entity_spawns += count_entity_spawns(&preview_chunk.entity_spawns);
        append_entity_markers(
            &preview_chunk.entity_spawns,
            center_chunk_pos,
            vertical_origin,
            &mut entity_markers,
        );
        let chunk_origin = relative_chunk_origin(preview_chunk.pos, center_chunk_pos);
        for (pos, block) in preview_chunk.chunk.iter_changed() {
            let renderable = block.is_filled() || block.is_liquid();
            if !renderable {
                continue;
            }
            filled_blocks += usize::from(block.is_filled());
            liquid_blocks += usize::from(block.is_liquid());

            for face in Face::ALL {
                if face_visible(&preview_chunk.chunk, pos, block, face) {
                    builder.add_face(pos, block, face, chunk_origin);
                }
            }
        }
    }

    if builder.indices.is_empty() {
        return Err(format!(
            "original TerrainChunk patch around {center_chunk_pos:?} produced no visible block \
             faces"
        ));
    }

    let patch_side = (chunk_radius * 2 + 1).max(1) as u32;
    Ok(OriginalWorldMesh {
        vertices: builder.vertices,
        indices: builder.indices,
        entity_markers,
        chunk_dimensions: (dimensions.x, dimensions.y),
        center_chunk_pos: (center_chunk_pos.x, center_chunk_pos.y),
        chunk_patch: (patch_side, patch_side),
        vertical_origin,
        generated_chunks: chunks.len(),
        newly_generated_chunks,
        cached_chunks,
        terrain_faces: builder.face_count,
        filled_blocks,
        liquid_blocks,
        generated_entity_spawns,
        enabled_world_features,
        wildlife_spawn_manifests,
        seed,
    })
}

fn count_entity_spawns(entity_spawns: &[EntitySpawn]) -> usize {
    entity_spawns
        .iter()
        .map(|entity_spawn| match entity_spawn {
            EntitySpawn::Entity(_) => 1,
            EntitySpawn::Group(group) => group.len(),
        })
        .sum()
}

fn append_entity_markers(
    entity_spawns: &[EntitySpawn],
    center_chunk_pos: Vec2<i32>,
    vertical_origin: f32,
    entity_markers: &mut Vec<OriginalEntityMarker>,
) {
    for entity_spawn in entity_spawns {
        match entity_spawn {
            EntitySpawn::Entity(entity) => {
                entity_markers.push(entity_marker(entity, center_chunk_pos, vertical_origin));
            },
            EntitySpawn::Group(group) => {
                entity_markers.extend(
                    group
                        .iter()
                        .map(|entity| entity_marker(entity, center_chunk_pos, vertical_origin)),
                );
            },
        }
    }
}

fn entity_marker(
    entity: &EntityInfo,
    center_chunk_pos: Vec2<i32>,
    vertical_origin: f32,
) -> OriginalEntityMarker {
    let center = chunk_center_wpos(center_chunk_pos);
    let render_pos = [
        (entity.pos.x - center.x) * TERRAIN_HORIZONTAL_SCALE,
        (entity.pos.z - vertical_origin) * 0.28 + 0.25,
        (entity.pos.y - center.y) * TERRAIN_HORIZONTAL_SCALE,
    ];
    let (radius, height, shape) = entity_marker_style(&entity.body, entity.scale);
    OriginalEntityMarker {
        render_pos,
        radius,
        height,
        color: entity_marker_color(entity),
        shape,
    }
}

fn entity_marker_style(body: &Body, scale: f32) -> (f32, f32, OriginalEntityMarkerShape) {
    use OriginalEntityMarkerShape::{Fish, Flyer, Humanoid, Large, Object, Quadruped};

    let (radius, height, shape) = match body {
        Body::Humanoid(_) => (0.34, 1.45, Humanoid),
        Body::BipedSmall(_) => (0.28, 0.82, Humanoid),
        Body::BipedLarge(_) => (0.68, 1.85, Humanoid),
        Body::QuadrupedSmall(_) => (0.28, 0.82, Quadruped),
        Body::QuadrupedMedium(_) | Body::QuadrupedLow(_) | Body::Theropod(_) => {
            (0.48, 1.05, Quadruped)
        },
        Body::Crustacean(_) | Body::Arthropod(_) => (0.34, 0.72, Quadruped),
        Body::BirdMedium(_) | Body::BirdLarge(_) => (0.42, 0.9, Flyer),
        Body::Dragon(_) => (1.15, 2.25, Flyer),
        Body::FishSmall(_) => (0.28, 0.82, Fish),
        Body::FishMedium(_) => (0.34, 0.72, Fish),
        Body::Golem(_) => (0.68, 1.85, Large),
        Body::Ship(_) => (1.35, 1.1, Large),
        Body::Object(_) | Body::Item(_) | Body::Plugin(_) => (0.24, 0.48, Object),
    };
    let scale = scale.clamp(0.4, 3.0);
    (radius * scale, height * scale, shape)
}

fn entity_marker_color(entity: &EntityInfo) -> [f32; 3] {
    if entity.trading_information.is_some() {
        return [1.0, 0.78, 0.18];
    }
    if let Some(special_entity) = &entity.special_entity {
        return match special_entity {
            SpecialEntity::Waypoint => [0.38, 0.78, 1.0],
            SpecialEntity::Teleporter(_) => [0.82, 0.42, 1.0],
            SpecialEntity::ArenaTotem { .. } => [1.0, 0.35, 0.26],
        };
    }

    match entity.alignment {
        Alignment::Enemy => [1.0, 0.18, 0.16],
        Alignment::Npc => [0.24, 0.48, 1.0],
        Alignment::Tame | Alignment::Owned(_) => [0.30, 0.92, 0.42],
        Alignment::Passive => [0.72, 0.76, 0.82],
        Alignment::Wild => [1.0, 0.56, 0.18],
    }
}

fn relative_chunk_origin(chunk_pos: Vec2<i32>, center_chunk_pos: Vec2<i32>) -> Vec2<i32> {
    let rect_size = TerrainChunkSize::RECT_SIZE.as_::<i32>();
    let chunk_delta = chunk_pos - center_chunk_pos;
    Vec2::new(chunk_delta.x * rect_size.x, chunk_delta.y * rect_size.y)
}

fn chunk_center_wpos(chunk_pos: Vec2<i32>) -> Vec2<f32> {
    TerrainChunkSize::center_wpos(chunk_pos).as_::<f32>()
}

fn face_visible(chunk: &TerrainChunk, pos: Vec3<i32>, block: &Block, face: Face) -> bool {
    match chunk.get(pos + face.normal()) {
        Ok(neighbor) if block.is_liquid() => !neighbor.is_liquid(),
        Ok(neighbor) => !neighbor.is_filled(),
        Err(_) => true,
    }
}

fn count_enabled_features(features: &Features) -> usize {
    [
        features.caverns,
        features.caves,
        features.rocks,
        features.shrubs,
        features.trees,
        features.scatter,
        features.paths,
        features.spots,
        features.peak_naming,
        features.biome_naming,
        features.train_tracks,
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count()
}

struct BlockMeshBuilder {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    face_count: usize,
    vertical_origin: f32,
}

impl BlockMeshBuilder {
    fn new(vertical_origin: f32) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            face_count: 0,
            vertical_origin,
        }
    }

    fn add_face(&mut self, pos: Vec3<i32>, block: &Block, face: Face, chunk_origin: Vec2<i32>) {
        let base = (self.vertices.len() / FLOATS_PER_VERTEX) as u32;
        let color = face.shade_color(block_color(block));
        for corner in face.corners(pos) {
            let [x, y, z] = self.render_point(corner, chunk_origin);
            self.vertices
                .extend_from_slice(&[x, y, z, color[0], color[1], color[2]]);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.face_count += 1;
    }

    fn render_point(&self, point: [f32; 3], chunk_origin: Vec2<i32>) -> [f32; 3] {
        let chunk_center = TerrainChunkSize::RECT_SIZE.x as f32 * 0.5;
        [
            (point[0] + chunk_origin.x as f32 - chunk_center) * TERRAIN_HORIZONTAL_SCALE,
            (point[2] - self.vertical_origin) * 0.28,
            (point[1] + chunk_origin.y as f32 - chunk_center) * TERRAIN_HORIZONTAL_SCALE,
        ]
    }
}

#[derive(Clone, Copy)]
enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    const ALL: [Self; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    fn normal(self) -> Vec3<i32> {
        match self {
            Self::PosX => Vec3::unit_x(),
            Self::NegX => -Vec3::unit_x(),
            Self::PosY => Vec3::unit_y(),
            Self::NegY => -Vec3::unit_y(),
            Self::PosZ => Vec3::unit_z(),
            Self::NegZ => -Vec3::unit_z(),
        }
    }

    fn shade_color(self, color: [f32; 3]) -> [f32; 3] {
        let shade = match self {
            Self::PosZ => 1.15,
            Self::NegZ => 0.55,
            Self::PosX | Self::PosY => 0.88,
            Self::NegX | Self::NegY => 0.72,
        };
        color.map(|channel| (channel * shade).min(1.0))
    }

    fn corners(self, pos: Vec3<i32>) -> [[f32; 3]; 4] {
        let x = pos.x as f32;
        let y = pos.y as f32;
        let z = pos.z as f32;
        let x1 = x + 1.0;
        let y1 = y + 1.0;
        let z1 = z + 1.0;

        match self {
            Self::PosX => [[x1, y, z], [x1, y1, z], [x1, y1, z1], [x1, y, z1]],
            Self::NegX => [[x, y1, z], [x, y, z], [x, y, z1], [x, y1, z1]],
            Self::PosY => [[x1, y1, z], [x, y1, z], [x, y1, z1], [x1, y1, z1]],
            Self::NegY => [[x, y, z], [x1, y, z], [x1, y, z1], [x, y, z1]],
            Self::PosZ => [[x, y, z1], [x1, y, z1], [x1, y1, z1], [x, y1, z1]],
            Self::NegZ => [[x, y1, z], [x1, y1, z], [x1, y, z], [x, y, z]],
        }
    }
}

fn block_color(block: &Block) -> [f32; 3] {
    if block.is_liquid() {
        return [0.08, 0.28, 0.72];
    }

    block
        .get_color()
        .map(|color| {
            [
                color.r as f32 / 255.0,
                color.g as f32 / 255.0,
                color.b as f32 / 255.0,
            ]
        })
        .unwrap_or([0.58, 0.58, 0.62])
}
