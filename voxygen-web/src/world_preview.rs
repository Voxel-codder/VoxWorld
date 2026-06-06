use rayon::ThreadPoolBuilder;
use vek::Vec2;
use veloren_world::sim::{FileOpts, GenOpts, WorldOpts, WorldSim};

pub const FLOATS_PER_VERTEX: usize = 6;

pub struct OriginalWorldMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    pub chunk_dimensions: (u32, u32),
    pub water_chunks: usize,
    pub forest_chunks: usize,
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

    mesh_from_sim(&sim, SEED)
}

fn mesh_from_sim(sim: &WorldSim, seed: u32) -> Result<OriginalWorldMesh, String> {
    let dimensions = sim.get_size();
    if dimensions.x < 2 || dimensions.y < 2 {
        return Err("original WorldSim generated too few chunks for terrain mesh".to_owned());
    }

    let mut min_alt = f32::MAX;
    let mut max_alt = f32::MIN;
    for y in 0..dimensions.y {
        for x in 0..dimensions.x {
            let chunk = sim
                .get(Vec2::new(x as i32, y as i32))
                .ok_or_else(|| format!("missing generated SimChunk at {x},{y}"))?;
            let surface_alt = chunk.alt.max(chunk.water_alt);
            min_alt = min_alt.min(surface_alt);
            max_alt = max_alt.max(surface_alt);
        }
    }

    let alt_span = (max_alt - min_alt).max(1.0);
    let center_x = (dimensions.x.saturating_sub(1)) as f32 * 0.5;
    let center_z = (dimensions.y.saturating_sub(1)) as f32 * 0.5;
    let mut vertices =
        Vec::with_capacity(dimensions.x as usize * dimensions.y as usize * FLOATS_PER_VERTEX);
    let mut water_chunks = 0usize;
    let mut forest_chunks = 0usize;

    for y in 0..dimensions.y {
        for x in 0..dimensions.x {
            let chunk = sim
                .get(Vec2::new(x as i32, y as i32))
                .ok_or_else(|| format!("missing generated SimChunk at {x},{y}"))?;
            let is_water = chunk.water_alt > chunk.alt + 0.5;
            let surface_alt = if is_water { chunk.water_alt } else { chunk.alt };
            let alt_norm = ((surface_alt - min_alt) / alt_span).clamp(0.0, 1.0);
            let height = alt_norm * 13.5 - 3.5;
            let color = chunk_color(
                chunk.temp,
                chunk.humidity,
                chunk.tree_density,
                alt_norm,
                is_water,
            );

            water_chunks += usize::from(is_water);
            forest_chunks += usize::from(!is_water && chunk.tree_density > 0.48);

            vertices.extend_from_slice(&[
                x as f32 - center_x,
                height,
                y as f32 - center_z,
                color[0],
                color[1],
                color[2],
            ]);
        }
    }

    let mut indices = Vec::with_capacity(
        dimensions.x.saturating_sub(1) as usize * dimensions.y.saturating_sub(1) as usize * 6,
    );
    for y in 0..dimensions.y - 1 {
        for x in 0..dimensions.x - 1 {
            let a = y * dimensions.x + x;
            let b = a + 1;
            let c = a + dimensions.x;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    Ok(OriginalWorldMesh {
        vertices,
        indices,
        chunk_dimensions: (dimensions.x, dimensions.y),
        water_chunks,
        forest_chunks,
        seed,
    })
}

fn chunk_color(
    temp: f32,
    humidity: f32,
    tree_density: f32,
    alt_norm: f32,
    is_water: bool,
) -> [f32; 3] {
    if is_water {
        return [
            0.06,
            0.25 + humidity.clamp(0.0, 0.45) * 0.22,
            0.58 + alt_norm * 0.1,
        ];
    }
    if alt_norm > 0.82 {
        return [0.86, 0.9, 0.88];
    }
    if temp > 0.45 && humidity < 0.26 {
        return [0.72, 0.62, 0.32];
    }
    if tree_density > 0.48 {
        return [0.08, 0.34 + tree_density.clamp(0.0, 1.0) * 0.22, 0.12];
    }
    if humidity > 0.62 {
        return [0.16, 0.42, 0.24];
    }

    [0.34 + alt_norm * 0.18, 0.48 + humidity * 0.14, 0.22]
}
