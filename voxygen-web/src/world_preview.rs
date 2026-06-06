use common::{
    terrain::{Block, TerrainChunk, TerrainChunkSize},
    vol::{ReadVol, RectVolSize},
};
use rayon::ThreadPoolBuilder;
use vek::{Vec2, Vec3};
use veloren_world::{
    World,
    config::Features,
    index::{Index, IndexOwned, IndexRef},
    sim::{FileOpts, GenOpts, WorldOpts, WorldSim},
};

pub const FLOATS_PER_VERTEX: usize = 6;

pub struct OriginalWorldMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub chunk_dimensions: (u32, u32),
    pub chunk_pos: (i32, i32),
    pub terrain_faces: usize,
    pub filled_blocks: usize,
    pub liquid_blocks: usize,
    pub generated_entity_spawns: usize,
    pub enabled_world_features: usize,
    pub wildlife_spawn_manifests: usize,
    pub seed: u32,
}

pub fn build_original_world_mesh() -> Result<OriginalWorldMesh, String> {
    const SEED: u32 = 7;
    const MAP_LG: u32 = 5;

    let threadpool = ThreadPoolBuilder::new()
        .num_threads(1)
        .use_current_thread()
        .build()
        .map_err(|error| format!("failed to create browser worldgen threadpool: {error}"))?;
    let sim = WorldSim::generate(
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
    let preview_index = IndexRef {
        features: &preview_features,
        ..index_ref
    };
    let chunk_pos = Vec2::new(dimensions.x as i32 / 2, dimensions.y as i32 / 2);
    let (terrain_chunk, supplement) = world
        .generate_chunk(preview_index, chunk_pos, None, || false, None)
        .map_err(|_| format!("original World::generate_chunk cancelled at {chunk_pos:?}"))?;

    mesh_from_terrain_chunk(
        &terrain_chunk,
        dimensions,
        chunk_pos,
        SEED,
        enabled_world_features,
        wildlife_spawn_manifests,
        supplement.entity_spawns.len(),
    )
}

#[allow(clippy::struct_excessive_bools)]
fn terrain_chunk_preview_features(features: &Features) -> Features {
    Features {
        caverns: features.caverns,
        caves: features.caves,
        rocks: features.rocks,
        // These layers can pull .vox structure assets; they are enabled once the web
        // asset pack grows beyond the first world manifest bundle.
        shrubs: false,
        trees: false,
        scatter: features.scatter,
        paths: features.paths,
        spots: false,
        wildlife_density: features.wildlife_density,
        peak_naming: features.peak_naming,
        biome_naming: features.biome_naming,
        train_tracks: false,
    }
}

fn mesh_from_terrain_chunk(
    chunk: &TerrainChunk,
    dimensions: Vec2<u32>,
    chunk_pos: Vec2<i32>,
    seed: u32,
    enabled_world_features: usize,
    wildlife_spawn_manifests: usize,
    generated_entity_spawns: usize,
) -> Result<OriginalWorldMesh, String> {
    let min_z = chunk.get_min_z();
    let max_z = chunk.get_max_z();
    if max_z <= min_z {
        return Err(format!(
            "original TerrainChunk at {chunk_pos:?} has no vertical span"
        ));
    }

    let mut builder = BlockMeshBuilder::new((min_z + max_z) as f32 * 0.5);
    let mut filled_blocks = 0usize;
    let mut liquid_blocks = 0usize;

    for (pos, block) in chunk.iter_changed() {
        let renderable = block.is_filled() || block.is_liquid();
        if !renderable {
            continue;
        }
        filled_blocks += usize::from(block.is_filled());
        liquid_blocks += usize::from(block.is_liquid());

        for face in Face::ALL {
            if face_visible(chunk, pos, block, face) {
                builder.add_face(pos, block, face);
            }
        }
    }

    if builder.indices.is_empty() {
        return Err(format!(
            "original TerrainChunk at {chunk_pos:?} produced no visible block faces"
        ));
    }

    Ok(OriginalWorldMesh {
        vertices: builder.vertices,
        indices: builder.indices,
        chunk_dimensions: (dimensions.x, dimensions.y),
        chunk_pos: (chunk_pos.x, chunk_pos.y),
        terrain_faces: builder.face_count,
        filled_blocks,
        liquid_blocks,
        generated_entity_spawns,
        enabled_world_features,
        wildlife_spawn_manifests,
        seed,
    })
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

    fn add_face(&mut self, pos: Vec3<i32>, block: &Block, face: Face) {
        let base = (self.vertices.len() / FLOATS_PER_VERTEX) as u32;
        let color = face.shade_color(block_color(block));
        for corner in face.corners(pos) {
            let [x, y, z] = self.render_point(corner);
            self.vertices
                .extend_from_slice(&[x, y, z, color[0], color[1], color[2]]);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.face_count += 1;
    }

    fn render_point(&self, point: [f32; 3]) -> [f32; 3] {
        let chunk_center = TerrainChunkSize::RECT_SIZE.x as f32 * 0.5;
        [
            (point[0] - chunk_center) * 0.64,
            (point[2] - self.vertical_origin) * 0.28,
            (point[1] - chunk_center) * 0.64,
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
