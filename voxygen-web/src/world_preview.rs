use std::collections::HashMap;

use common::{
    comp::{
        Alignment, Body,
        inventory::trade_pricing::TradePricing,
        item::{Item, ItemDefinitionIdOwned, MaterialStatManifest, Quality},
        tool::AbilityMap,
    },
    generation::{EntityInfo, EntitySpawn, SpecialEntity},
    rtsim::{Profession, WorldSettings},
    terrain::{Block, BlockKind, SpriteKind, TerrainChunk, TerrainChunkSize, sprite::Category},
    trade::{Good, SiteInformation, SitePrices},
    vol::{ReadVol, RectVolSize},
};
use rayon::ThreadPoolBuilder;
use rtsim::data::architect::TrackedPopulation;
use vek::{Vec2, Vec3};
use veloren_world::{
    World,
    config::Features,
    index::{IndexOwned, IndexRef},
    sim::{FileOpts, GenOpts, WorldOpts},
    site::{PlotKind, SiteKind, plot::PlotKindMeta},
};

pub const FLOATS_PER_VERTEX: usize = 6;
pub const TERRAIN_HORIZONTAL_SCALE: f32 = 0.64;
const SEED: u32 = 7;
const MAP_LG: u32 = 5;
const CHUNK_RADIUS: i32 = 2;
const TERRAIN_INTERACTION_RANGE_BLOCKS: f32 = 5.0;
const ENTITY_INTERACTION_RANGE_BLOCKS: f32 = 14.0;

pub struct OriginalWorldPreview {
    world: World,
    index_owned: IndexOwned,
    preview_features: Features,
    chunk_cache: HashMap<(i32, i32), GeneratedPreviewChunk>,
    dimensions: Vec2<u32>,
    enabled_world_features: usize,
    wildlife_spawn_manifests: usize,
    original_sites: usize,
    original_settlements: usize,
    original_pois: usize,
    rtsim_sites: usize,
    rtsim_existing_npcs: usize,
    rtsim_wanted_population: usize,
    rtsim_wanted_merchants: usize,
    rtsim_wanted_guards: usize,
    start: PreviewStartLocation,
    site_markers: Vec<OriginalSiteMarker>,
    seed: u32,
}

pub struct OriginalWorldMesh {
    pub terrain_chunks: Vec<OriginalTerrainChunkMesh>,
    pub entity_markers: Vec<OriginalEntityMarker>,
    pub chunk_dimensions: (u32, u32),
    pub center_chunk_pos: (i32, i32),
    pub chunk_patch: (u32, u32),
    pub vertical_origin: f32,
    pub generated_chunks: usize,
    pub newly_generated_chunks: usize,
    pub cached_chunks: usize,
    pub newly_meshed_chunks: usize,
    pub cached_mesh_chunks: usize,
    pub terrain_faces: usize,
    pub filled_blocks: usize,
    pub liquid_blocks: usize,
    pub terrain_sprite_props: usize,
    pub generated_entity_spawns: usize,
    pub enabled_world_features: usize,
    pub wildlife_spawn_manifests: usize,
    pub original_sites: usize,
    pub original_settlements: usize,
    pub original_pois: usize,
    pub rtsim_sites: usize,
    pub rtsim_existing_npcs: usize,
    pub rtsim_wanted_population: usize,
    pub rtsim_wanted_merchants: usize,
    pub rtsim_wanted_guards: usize,
    pub site_npc_markers: usize,
    pub site_trader_markers: usize,
    pub site_market_markers: usize,
    pub seed: u32,
}

pub struct InteractionAttempt {
    pub summary: String,
    pub trade_panel: Option<TradePanelPreview>,
}

pub struct TradePanelPreview {
    pub title: String,
    pub stock: Vec<String>,
    pub wares: Vec<TradePanelWare>,
}

pub struct TradePanelWare {
    pub name: String,
    pub quality: String,
    pub buy: String,
    pub sell: String,
}

pub struct OriginalTerrainChunkMesh {
    pub chunk_pos: (i32, i32),
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy)]
pub struct OriginalEntityMarker {
    pub render_pos: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub color: [f32; 3],
    pub shape: OriginalEntityMarkerShape,
    pub yaw_radians: f32,
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

struct PreviewStartLocation {
    center_chunk_pos: Vec2<i32>,
    player_wpos: Vec2<f32>,
    summary: String,
}

struct OriginalSiteMarker {
    wpos: Vec3<f32>,
    kind: OriginalSiteMarkerKind,
    label: &'static str,
    site_name: String,
    trade_preview: Option<TradePreview>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginalSiteMarkerKind {
    Trader,
    Guard,
    Captain,
    Adventurer,
    Market,
}

struct TradePreview {
    wares: Vec<TradePreviewItem>,
    stock: Vec<(Good, f32)>,
}

struct TradePreviewItem {
    name: String,
    buy_coins: f32,
    sell_coins: f32,
    quality: Quality,
}

impl TradePreview {
    fn summary(&self) -> String {
        let stock = if self.stock.is_empty() {
            "stock unavailable".to_owned()
        } else {
            format!(
                "stock {}",
                self.stock
                    .iter()
                    .map(|(good, amount)| format!("{good:?} {}", format_stock_amount(*amount)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let wares = if self.wares.is_empty() {
            "no priced wares".to_owned()
        } else {
            format!(
                "wares {}",
                self.wares
                    .iter()
                    .map(TradePreviewItem::summary)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!("{stock}; {wares}")
    }

    fn panel(&self, title: String) -> TradePanelPreview {
        TradePanelPreview {
            title,
            stock: self
                .stock
                .iter()
                .map(|(good, amount)| format!("{good:?} {}", format_stock_amount(*amount)))
                .collect(),
            wares: self
                .wares
                .iter()
                .map(TradePreviewItem::panel_ware)
                .collect(),
        }
    }
}

impl TradePreviewItem {
    fn summary(&self) -> String {
        format!(
            "{} {:?} buy {}c sell {}c",
            self.name,
            self.quality,
            format_coin_price(self.buy_coins),
            format_coin_price(self.sell_coins)
        )
    }

    fn panel_ware(&self) -> TradePanelWare {
        TradePanelWare {
            name: self.name.clone(),
            quality: format!("{:?}", self.quality),
            buy: format!("{}c", format_coin_price(self.buy_coins)),
            sell: format!("{}c", format_coin_price(self.sell_coins)),
        }
    }
}

struct RtsimPreviewStats {
    sites: usize,
    existing_npcs: usize,
    wanted_population: usize,
    wanted_merchants: usize,
    wanted_guards: usize,
}

impl RtsimPreviewStats {
    fn from_data(data: &rtsim::Data) -> Self {
        let mut wanted_merchants = 0usize;
        let mut wanted_guards = 0usize;
        for (population, count) in data.architect.wanted_population.iter() {
            match population {
                TrackedPopulation::Merchants | TrackedPopulation::OtherTownNpcs => {
                    wanted_merchants += count as usize;
                },
                TrackedPopulation::Guards => wanted_guards += count as usize,
                _ => {},
            }
        }

        Self {
            sites: data.sites.len(),
            existing_npcs: data.npcs.npcs.len(),
            wanted_population: data.architect.wanted_population.total() as usize,
            wanted_merchants,
            wanted_guards,
        }
    }
}

impl OriginalWorldPreview {
    pub fn new() -> Result<Self, String> {
        let threadpool = ThreadPoolBuilder::new()
            .num_threads(1)
            .use_current_thread()
            .build()
            .map_err(|error| format!("failed to create browser worldgen threadpool: {error}"))?;
        let (world, index_owned) = World::generate(
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
        let dimensions = world.sim().get_size();
        if dimensions.x < 2 || dimensions.y < 2 {
            return Err("original WorldSim generated too few chunks for terrain mesh".to_owned());
        }

        let (
            enabled_world_features,
            wildlife_spawn_manifests,
            preview_features,
            original_sites,
            original_settlements,
            original_pois,
            rtsim_stats,
            start,
            site_markers,
        ) = {
            let index_ref = index_owned.as_index_ref();
            let rtsim_data = rtsim::Data::generate(&WorldSettings::default(), &world, index_ref);
            let rtsim_stats = RtsimPreviewStats::from_data(&rtsim_data);
            (
                count_enabled_features(index_ref.features),
                index_ref.wildlife_spawns.len(),
                terrain_chunk_preview_features(index_ref.features),
                world.civs().sites.values().len(),
                world
                    .civs()
                    .sites
                    .values()
                    .filter(|site| site.is_settlement())
                    .count(),
                world.civs().pois.values().len(),
                rtsim_stats,
                select_preview_start(&world, index_ref, dimensions),
                collect_site_markers(&world, index_ref),
            )
        };

        Ok(Self {
            world,
            index_owned,
            preview_features,
            chunk_cache: HashMap::new(),
            dimensions,
            enabled_world_features,
            wildlife_spawn_manifests,
            original_sites,
            original_settlements,
            original_pois,
            rtsim_sites: rtsim_stats.sites,
            rtsim_existing_npcs: rtsim_stats.existing_npcs,
            rtsim_wanted_population: rtsim_stats.wanted_population,
            rtsim_wanted_merchants: rtsim_stats.wanted_merchants,
            rtsim_wanted_guards: rtsim_stats.wanted_guards,
            start,
            site_markers,
            seed: SEED,
        })
    }

    pub fn initial_center_chunk_pos(&self) -> Vec2<i32> { self.start.center_chunk_pos }

    pub fn initial_player_wpos(&self) -> Vec2<f32> { self.start.player_wpos }

    pub fn start_summary(&self) -> &str { &self.start.summary }

    pub fn center_chunk_for_wpos(&self, wpos: Vec2<f32>) -> Vec2<i32> {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
        self.clamp_center_chunk_pos(Vec2::new(
            (wpos.x / rect_size.x).floor() as i32,
            (wpos.y / rect_size.y).floor() as i32,
        ))
    }

    pub fn clamp_center_chunk_pos(&self, chunk_pos: Vec2<i32>) -> Vec2<i32> {
        clamp_center_chunk_pos_to_dimensions(chunk_pos, self.dimensions)
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
        let terrain_z = self.player_terrain_z(player_wpos);
        [
            delta.x * TERRAIN_HORIZONTAL_SCALE,
            (terrain_z - vertical_origin) * 0.28 + 0.25,
            delta.y * TERRAIN_HORIZONTAL_SCALE,
        ]
    }

    pub fn player_terrain_z(&self, player_wpos: Vec2<f32>) -> f32 {
        let sample_wpos = player_sample_wpos(player_wpos);
        self.cached_accessible_player_pos(sample_wpos)
            .map(|pos| pos.z)
            .unwrap_or_else(|| self.world.sim().get_surface_alt_approx(sample_wpos) + 0.5)
    }

    pub fn cached_player_terrain_z(&self, player_wpos: Vec2<f32>) -> Option<f32> {
        self.cached_accessible_player_pos(player_sample_wpos(player_wpos))
            .map(|pos| pos.z)
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
            let mesh = build_chunk_mesh_fragment(&terrain_chunk, *chunk_pos);
            self.chunk_cache.insert(key, GeneratedPreviewChunk {
                pos: *chunk_pos,
                chunk: terrain_chunk,
                entity_spawns: supplement.entity_spawns,
                mesh,
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
            self.original_sites,
            self.original_settlements,
            self.original_pois,
            self.rtsim_sites,
            self.rtsim_existing_npcs,
            self.rtsim_wanted_population,
            self.rtsim_wanted_merchants,
            self.rtsim_wanted_guards,
            &self.site_markers,
        )
    }

    pub fn interaction_summary(&self, player_wpos: Vec2<f32>) -> String {
        let terrain_prop = self.nearest_terrain_prop(player_wpos, 96.0);
        let entity = self.nearest_entity_target(player_wpos, 96.0);
        format!(
            " Nearest terrain target: {}. Nearest entity target: {}.",
            terrain_prop
                .map(|focus| focus.summary())
                .unwrap_or_else(|| "none".to_owned()),
            entity
                .map(|focus| focus.summary())
                .unwrap_or_else(|| "none".to_owned())
        )
    }

    pub fn interaction_attempt(&self, player_wpos: Vec2<f32>) -> InteractionAttempt {
        let terrain_prop = self.nearest_terrain_prop(player_wpos, TERRAIN_INTERACTION_RANGE_BLOCKS);
        let entity = self.nearest_entity_target(player_wpos, ENTITY_INTERACTION_RANGE_BLOCKS);
        let (interaction, trade_panel) = match (terrain_prop, entity) {
            (Some(terrain_prop), Some(entity)) if terrain_prop.distance <= entity.distance() => {
                (terrain_prop.action_summary(), None)
            },
            (Some(_), Some(entity)) => (entity.action_summary(), entity.trade_panel()),
            (Some(terrain_prop), None) => (terrain_prop.action_summary(), None),
            (None, Some(entity)) => (entity.action_summary(), entity.trade_panel()),
            (None, None) => (
                format!(
                    "no target in reach (terrain {:.1}m, entity {:.1}m)",
                    TERRAIN_INTERACTION_RANGE_BLOCKS, ENTITY_INTERACTION_RANGE_BLOCKS
                ),
                None,
            ),
        };
        InteractionAttempt {
            summary: format!("Last interaction: {interaction}."),
            trade_panel,
        }
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

fn select_preview_start(
    world: &World,
    index: IndexRef,
    dimensions: Vec2<u32>,
) -> PreviewStartLocation {
    let fallback_center = clamp_center_chunk_pos_to_dimensions(
        Vec2::new(dimensions.x as i32 / 2, dimensions.y as i32 / 2),
        dimensions,
    );
    let fallback_wpos = chunk_center_wpos(fallback_center);

    let Some(candidate) = world
        .civs()
        .sites
        .iter()
        .filter_map(|(_, civ_site)| {
            let site_id = civ_site.site_tmp?;
            let site = &index.sites[site_id];
            let site_kind = site.kind.unwrap_or(civ_site.kind);
            let town_priority = match site_kind {
                SiteKind::Refactor => 3.0,
                SiteKind::SavannahTown | SiteKind::CoastalTown | SiteKind::DesertCity => 2.0,
                SiteKind::CliffTown => 1.5,
                _ => return None,
            };
            let plot_count = site.plots().len();
            let size_score = 1.0 / (1.0 + ((plot_count as f32 - 30.0).abs() / 30.0));
            let center_score = {
                let world_center = dimensions.as_::<f32>() * 0.5;
                let distance = (civ_site.center.as_::<f32>() - world_center).magnitude();
                let max_dimension = dimensions.x.max(dimensions.y) as f32;
                1.0 - (distance / max_dimension).clamp(0.0, 1.0)
            };
            let player_wpos = site_start_wpos(site).unwrap_or_else(|| {
                chunk_center_wpos(civ_site.center).map(|coord| coord.floor() as i32)
            });
            let center_chunk_pos =
                clamp_center_chunk_pos_to_dimensions(chunk_pos_for_wpos(player_wpos), dimensions);
            let score = town_priority * 4.0 + size_score * 2.0 + center_score;
            let name = site.name().unwrap_or("unnamed site");
            Some(PreviewStartCandidate {
                center_chunk_pos,
                player_wpos: player_wpos.as_::<f32>() + Vec2::broadcast(0.5),
                score,
                plot_count,
                site_kind,
                site_id: site_id.id(),
                name: name.to_owned(),
            })
        })
        .max_by(|a, b| a.score.total_cmp(&b.score))
    else {
        return PreviewStartLocation {
            center_chunk_pos: fallback_center,
            player_wpos: fallback_wpos,
            summary: format!(
                "fallback world center chunk {:?}",
                chunk_key(fallback_center)
            ),
        };
    };

    PreviewStartLocation {
        center_chunk_pos: candidate.center_chunk_pos,
        player_wpos: clamp_player_wpos_to_dimensions(candidate.player_wpos, dimensions),
        summary: format!(
            "starting near original {:?} '{}' site {} with {} plots at chunk {:?}",
            candidate.site_kind,
            candidate.name,
            candidate.site_id,
            candidate.plot_count,
            chunk_key(candidate.center_chunk_pos)
        ),
    }
}

fn collect_site_markers(world: &World, index: IndexRef) -> Vec<OriginalSiteMarker> {
    world
        .civs()
        .sites
        .values()
        .filter_map(|civ_site| civ_site.site_tmp)
        .flat_map(|site_id| {
            let site = &index.sites[site_id];
            let site_name = site.name().unwrap_or("unnamed site").to_owned();
            let site_prices = index.get_site_prices(site_id.id());
            let site_information = site.trade_information(site_id);
            let mut markers = Vec::new();

            for plot in site.plots() {
                if let Some((kind, label, tile)) = marker_from_service_plot(plot) {
                    let trade_preview = trade_preview_for_marker(
                        kind,
                        label,
                        site_prices.as_ref(),
                        site_information.as_ref(),
                    );
                    markers.push(site_marker_at_plot(
                        world.sim(),
                        site,
                        &site_name,
                        kind,
                        label,
                        tile,
                        Vec2::zero(),
                        trade_preview,
                    ));
                }
            }

            if site.kind.is_some_and(|kind| {
                matches!(
                    kind.meta(),
                    Some(common::terrain::SiteKindMeta::Settlement(_))
                )
            }) {
                let spawn_tiles = site_npc_spawn_tiles(site);
                let town_plots = site.plots().len();
                let guards = town_plots / 4;
                let adventurers = town_plots / 5;
                let merchants = town_plots / 6 + 1;
                let town_professions = town_plots.saturating_sub(guards + adventurers);
                let roster = rtsim_site_roster(guards, adventurers, merchants, town_professions);

                for (index, profession) in roster.into_iter().enumerate() {
                    let tile = spawn_tiles
                        .get(index % spawn_tiles.len().max(1))
                        .copied()
                        .unwrap_or_else(|| {
                            site.plazas()
                                .next()
                                .map(|plaza| site.plot(plaza).root_tile())
                                .unwrap_or_default()
                        });
                    let (kind, label) = marker_from_rtsim_profession(profession);
                    let trade_preview = trade_preview_for_marker(
                        kind,
                        label,
                        site_prices.as_ref(),
                        site_information.as_ref(),
                    );
                    markers.push(site_marker_at_plot(
                        world.sim(),
                        site,
                        &site_name,
                        kind,
                        label,
                        tile,
                        marker_offset(index),
                        trade_preview,
                    ));
                }
            }

            markers
        })
        .collect()
}

fn site_marker_at_plot(
    sim: &veloren_world::sim::WorldSim,
    site: &veloren_world::site::Site,
    site_name: &str,
    kind: OriginalSiteMarkerKind,
    label: &'static str,
    tile: Vec2<i32>,
    offset: Vec2<f32>,
    trade_preview: Option<TradePreview>,
) -> OriginalSiteMarker {
    let wpos2d = site.tile_center_wpos(tile);
    OriginalSiteMarker {
        wpos: (wpos2d.as_::<f32>() + offset).with_z(
            sim.get_alt_approx(wpos2d)
                .unwrap_or_else(|| sim.get_surface_alt_approx(wpos2d))
                + 1.0,
        ),
        kind,
        label,
        site_name: site_name.to_owned(),
        trade_preview,
    }
}

fn trade_preview_for_marker(
    kind: OriginalSiteMarkerKind,
    label: &'static str,
    site_prices: Option<&SitePrices>,
    site_information: Option<&SiteInformation>,
) -> Option<TradePreview> {
    if !matches!(
        kind,
        OriginalSiteMarkerKind::Trader | OriginalSiteMarkerKind::Market
    ) {
        return None;
    }

    let site_prices = site_prices?;
    let stock = site_stock_summary(site_information, label);
    let wares = trade_sample_items(label)
        .iter()
        .filter_map(|item_id| trade_preview_item(site_prices, item_id))
        .take(3)
        .collect::<Vec<_>>();

    Some(TradePreview { wares, stock })
}

fn site_stock_summary(
    site_information: Option<&SiteInformation>,
    label: &'static str,
) -> Vec<(Good, f32)> {
    let mut goods = site_information
        .map(|information| {
            information
                .unconsumed_stock
                .iter()
                .filter_map(|(good, amount)| {
                    (*amount > 0.0 && trade_good_allowed(label, *good)).then_some((*good, *amount))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    goods.sort_by(|a, b| b.1.total_cmp(&a.1));
    goods.truncate(3);
    goods
}

fn trade_preview_item(site_prices: &SitePrices, item_id: &'static str) -> Option<TradePreviewItem> {
    let item_definition_id = ItemDefinitionIdOwned::Simple(item_id.to_owned());
    let materials = TradePricing::get_materials(&item_definition_id.as_ref())?;
    let coin_price = site_prices
        .values
        .get(&Good::Coin)
        .copied()
        .unwrap_or(1.0)
        .max(0.001);
    let quality = Item::new_from_item_definition_id(
        item_definition_id.as_ref(),
        &AbilityMap::load().read(),
        &MaterialStatManifest::load().read(),
    )
    .map_or(Quality::Low, |item| item.quality());
    let buy_coins = materials
        .iter()
        .map(|(amount, good)| site_prices.values.get(good).copied().unwrap_or_default() * amount)
        .sum::<f32>()
        / coin_price;
    let sell_coins = materials
        .iter()
        .map(|(amount, good)| {
            site_prices.values.get(good).copied().unwrap_or_default()
                * amount
                * good.sell_discount(quality)
        })
        .sum::<f32>()
        / coin_price;

    Some(TradePreviewItem {
        name: short_item_name(item_id),
        buy_coins,
        sell_coins,
        quality,
    })
}

fn trade_sample_items(label: &'static str) -> &'static [&'static str] {
    match label {
        "farmer" => &[
            "common.items.food.apple",
            "common.items.food.cheese",
            "common.items.food.lettuce",
        ],
        "chef" | "tavern" => &[
            "common.items.food.cheese",
            "common.items.food.meat.fish_cooked",
            "common.items.food.apple_mushroom_curry",
        ],
        "herbalist" | "alchemist" => &[
            "common.items.consumable.potion_minor",
            "common.items.crafting_ing.empty_vial",
            "common.items.crafting_ing.honey",
        ],
        "blacksmith" | "workshop" => &[
            "common.items.weapons.sword.starter",
            "common.items.mineral.ore.iron",
            "common.items.crafting_ing.stones",
        ],
        "hunter" => &[
            "common.items.weapons.bow.starter",
            "common.items.food.meat.beast_small_raw",
            "common.items.log.wood",
        ],
        "board" | "merchant" => &[
            "common.items.utility.coins",
            "common.items.consumable.potion_minor",
            "common.items.food.cheese",
        ],
        _ => &[
            "common.items.utility.coins",
            "common.items.food.cheese",
            "common.items.consumable.potion_minor",
        ],
    }
}

fn trade_good_allowed(label: &'static str, good: Good) -> bool {
    match label {
        "farmer" | "chef" | "tavern" => matches!(good, Good::Food | Good::Coin),
        "herbalist" | "alchemist" => matches!(
            good,
            Good::Potions | Good::Stone | Good::Wood | Good::Ingredients | Good::Coin
        ),
        "blacksmith" | "workshop" => matches!(good, Good::Armor | Good::Tools | Good::Coin),
        "hunter" => matches!(good, Good::Tools | Good::Food | Good::Coin),
        "board" => matches!(
            good,
            Good::Food
                | Good::Potions
                | Good::Stone
                | Good::Wood
                | Good::Ingredients
                | Good::Tools
                | Good::Armor
                | Good::Coin
                | Good::Recipe
        ),
        _ => !matches!(
            good,
            Good::Territory(_) | Good::Terrain(_) | Good::Transportation | Good::RoadSecurity
        ),
    }
}

fn short_item_name(item_id: &str) -> String {
    item_id
        .rsplit('.')
        .next()
        .unwrap_or(item_id)
        .replace('_', " ")
}

fn format_coin_price(value: f32) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

fn format_stock_amount(value: f32) -> String {
    if value >= 1_000_000.0 {
        format!("{:.1}m", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else {
        format!("{value:.0}")
    }
}

fn site_npc_spawn_tiles(site: &veloren_world::site::Site) -> Vec<Vec2<i32>> {
    let mut tiles = site
        .plots()
        .filter_map(|plot| match plot.meta() {
            Some(PlotKindMeta::House { door_tile })
            | Some(PlotKindMeta::Other { door_tile })
            | Some(PlotKindMeta::AirshipDock { door_tile, .. }) => Some(door_tile),
            Some(PlotKindMeta::Workshop { door_tile }) => {
                Some(door_tile.unwrap_or(plot.root_tile()))
            },
            Some(PlotKindMeta::Dungeon) | None => None,
        })
        .collect::<Vec<_>>();
    if tiles.is_empty() {
        tiles.extend(site.plazas().map(|plaza| site.plot(plaza).root_tile()));
    }
    if tiles.is_empty() {
        tiles.push(Vec2::zero());
    }
    tiles
}

fn rtsim_site_roster(
    guards: usize,
    adventurers: usize,
    merchants: usize,
    town_professions: usize,
) -> Vec<Profession> {
    let mut roster = Vec::with_capacity(guards + adventurers + merchants + town_professions);
    roster.extend(std::iter::repeat_n(Profession::Guard, guards));
    roster.extend((0..adventurers).map(|rank| Profession::Adventurer((rank % 4) as u32)));
    roster.extend(std::iter::repeat_n(Profession::Merchant, merchants));
    let cycle = [
        Profession::Farmer,
        Profession::Herbalist,
        Profession::Blacksmith,
        Profession::Chef,
        Profession::Alchemist,
        Profession::Hunter,
    ];
    roster.extend((0..town_professions).map(|index| cycle[index % cycle.len()]));
    roster
}

fn marker_from_rtsim_profession(profession: Profession) -> (OriginalSiteMarkerKind, &'static str) {
    match profession {
        Profession::Merchant => (OriginalSiteMarkerKind::Trader, "merchant"),
        Profession::Farmer => (OriginalSiteMarkerKind::Trader, "farmer"),
        Profession::Herbalist => (OriginalSiteMarkerKind::Trader, "herbalist"),
        Profession::Blacksmith => (OriginalSiteMarkerKind::Trader, "blacksmith"),
        Profession::Chef => (OriginalSiteMarkerKind::Trader, "chef"),
        Profession::Alchemist => (OriginalSiteMarkerKind::Trader, "alchemist"),
        Profession::Hunter => (OriginalSiteMarkerKind::Trader, "hunter"),
        Profession::Guard => (OriginalSiteMarkerKind::Guard, "rtsim guard"),
        Profession::Captain => (OriginalSiteMarkerKind::Captain, "rtsim captain"),
        Profession::Adventurer(_) => (OriginalSiteMarkerKind::Adventurer, "adventurer"),
        Profession::Pirate(false) => (OriginalSiteMarkerKind::Guard, "pirate"),
        Profession::Pirate(true) => (OriginalSiteMarkerKind::Guard, "pirate captain"),
        Profession::Cultist => (OriginalSiteMarkerKind::Guard, "cultist"),
    }
}

fn marker_offset(index: usize) -> Vec2<f32> {
    const OFFSETS: [Vec2<f32>; 8] = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.5, 0.5),
        Vec2::new(-1.5, 0.5),
        Vec2::new(0.5, 1.5),
        Vec2::new(0.5, -1.5),
        Vec2::new(2.2, -0.6),
        Vec2::new(-2.2, -0.6),
        Vec2::new(-0.6, 2.2),
    ];
    OFFSETS[index % OFFSETS.len()]
}

fn marker_from_service_plot(
    plot: &veloren_world::site::Plot,
) -> Option<(OriginalSiteMarkerKind, &'static str, Vec2<i32>)> {
    let fallback_tile = plot.root_tile();
    let marker = match plot.kind() {
        PlotKind::AirshipDock(_)
        | PlotKind::CoastalAirshipDock(_)
        | PlotKind::CliffTownAirshipDock(_)
        | PlotKind::DesertCityAirshipDock(_)
        | PlotKind::SavannahAirshipDock(_) => (OriginalSiteMarkerKind::Captain, "airship dock"),
        PlotKind::Workshop(_)
        | PlotKind::CoastalWorkshop(_)
        | PlotKind::SavannahWorkshop(_)
        | PlotKind::DesertCityMultiPlot(_) => (OriginalSiteMarkerKind::Trader, "workshop"),
        PlotKind::Tavern(_) => (OriginalSiteMarkerKind::Trader, "tavern"),
        PlotKind::Plaza(_) => (OriginalSiteMarkerKind::Market, "board"),
        _ => return None,
    };

    let tile = match plot.meta() {
        Some(PlotKindMeta::AirshipDock { door_tile, .. })
        | Some(PlotKindMeta::House { door_tile })
        | Some(PlotKindMeta::Other { door_tile }) => door_tile,
        Some(PlotKindMeta::Workshop { door_tile }) => door_tile.unwrap_or(fallback_tile),
        Some(PlotKindMeta::Dungeon) | None => fallback_tile,
    };

    Some((marker.0, marker.1, tile))
}

struct PreviewStartCandidate {
    center_chunk_pos: Vec2<i32>,
    player_wpos: Vec2<f32>,
    score: f32,
    plot_count: usize,
    site_kind: SiteKind,
    site_id: u64,
    name: String,
}

fn site_start_wpos(site: &veloren_world::site::Site) -> Option<Vec2<i32>> {
    site.plazas()
        .next()
        .map(|plaza| site.tile_center_wpos(site.plot(plaza).root_tile()))
        .or_else(|| {
            site.plots()
                .filter_map(|plot| plot.meta()?.door_tile())
                .map(|door_tile| site.tile_center_wpos(door_tile))
                .next()
        })
}

fn chunk_pos_for_wpos(wpos: Vec2<i32>) -> Vec2<i32> {
    let rect_size = TerrainChunkSize::RECT_SIZE.as_::<i32>();
    Vec2::new(
        wpos.x.div_euclid(rect_size.x),
        wpos.y.div_euclid(rect_size.y),
    )
}

fn clamp_center_chunk_pos_to_dimensions(chunk_pos: Vec2<i32>, dimensions: Vec2<u32>) -> Vec2<i32> {
    let max_world_x = dimensions.x.saturating_sub(1) as i32;
    let max_world_y = dimensions.y.saturating_sub(1) as i32;
    let min_x = CHUNK_RADIUS.min(max_world_x);
    let min_y = CHUNK_RADIUS.min(max_world_y);
    let max_x = (max_world_x - CHUNK_RADIUS).max(min_x);
    let max_y = (max_world_y - CHUNK_RADIUS).max(min_y);

    Vec2::new(
        chunk_pos.x.clamp(min_x, max_x),
        chunk_pos.y.clamp(min_y, max_y),
    )
}

fn clamp_player_wpos_to_dimensions(wpos: Vec2<f32>, dimensions: Vec2<u32>) -> Vec2<f32> {
    let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
    let max = dimensions.as_::<f32>() * rect_size - Vec2::broadcast(1.0);
    Vec2::new(wpos.x.clamp(0.0, max.x), wpos.y.clamp(0.0, max.y))
}

fn chunk_key(chunk_pos: Vec2<i32>) -> (i32, i32) { (chunk_pos.x, chunk_pos.y) }

fn player_sample_wpos(player_wpos: Vec2<f32>) -> Vec2<i32> {
    Vec2::new(player_wpos.x.floor() as i32, player_wpos.y.floor() as i32)
}

impl OriginalWorldPreview {
    fn cached_accessible_player_pos(&self, sample_wpos: Vec2<i32>) -> Option<Vec3<f32>> {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<i32>();
        let chunk_pos = Vec2::new(
            sample_wpos.x.div_euclid(rect_size.x),
            sample_wpos.y.div_euclid(rect_size.y),
        );
        self.chunk_cache
            .get(&chunk_key(chunk_pos))
            .map(|preview_chunk| preview_chunk.chunk.find_accessible_pos(sample_wpos, false))
    }

    fn nearest_terrain_prop(
        &self,
        player_wpos: Vec2<f32>,
        radius_blocks: f32,
    ) -> Option<TerrainPropFocus<'_>> {
        let radius_squared = radius_blocks * radius_blocks;
        self.chunk_cache
            .values()
            .flat_map(|preview_chunk| preview_chunk.mesh.terrain_props.iter())
            .filter_map(|prop| {
                let prop_wpos = Vec2::new(prop.wpos.x as f32 + 0.5, prop.wpos.y as f32 + 0.5);
                let distance_squared = vec2_distance_squared(player_wpos, prop_wpos);
                (distance_squared <= radius_squared).then_some((prop, distance_squared))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(prop, distance_squared)| TerrainPropFocus {
                prop,
                distance: distance_squared.sqrt(),
            })
    }

    fn nearest_entity(
        &self,
        player_wpos: Vec2<f32>,
        radius_blocks: f32,
    ) -> Option<EntityFocus<'_>> {
        let radius_squared = radius_blocks * radius_blocks;
        self.chunk_cache
            .values()
            .flat_map(|preview_chunk| preview_chunk.entity_spawns.iter())
            .flat_map(entity_spawn_entities)
            .filter_map(|entity| {
                let entity_wpos = Vec2::new(entity.pos.x, entity.pos.y);
                let distance_squared = vec2_distance_squared(player_wpos, entity_wpos);
                (distance_squared <= radius_squared).then_some((entity, distance_squared))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(entity, distance_squared)| EntityFocus {
                entity,
                distance: distance_squared.sqrt(),
            })
    }

    fn nearest_site_marker(
        &self,
        player_wpos: Vec2<f32>,
        radius_blocks: f32,
    ) -> Option<SiteMarkerFocus<'_>> {
        let radius_squared = radius_blocks * radius_blocks;
        self.site_markers
            .iter()
            .filter_map(|marker| {
                let marker_wpos = marker.wpos.xy();
                let distance_squared = vec2_distance_squared(player_wpos, marker_wpos);
                (distance_squared <= radius_squared).then_some((marker, distance_squared))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(marker, distance_squared)| SiteMarkerFocus {
                marker,
                distance: distance_squared.sqrt(),
            })
    }

    fn nearest_entity_target(
        &self,
        player_wpos: Vec2<f32>,
        radius_blocks: f32,
    ) -> Option<EntityTargetFocus<'_>> {
        let spawned = self
            .nearest_entity(player_wpos, radius_blocks)
            .map(EntityTargetFocus::Generated);
        let site = self
            .nearest_site_marker(player_wpos, radius_blocks)
            .map(EntityTargetFocus::Site);

        match (spawned, site) {
            (Some(spawned), Some(site)) if spawned.distance() <= site.distance() => Some(spawned),
            (Some(_), Some(site)) => Some(site),
            (Some(spawned), None) => Some(spawned),
            (None, Some(site)) => Some(site),
            (None, None) => None,
        }
    }
}

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
    mesh: ChunkMeshFragment,
}

struct ChunkMeshFragment {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    face_count: usize,
    filled_blocks: usize,
    liquid_blocks: usize,
    sprite_props: usize,
    terrain_props: Vec<TerrainSpriteProp>,
}

struct TerrainSpriteProp {
    wpos: Vec3<i32>,
    sprite: SpriteKind,
    category: Category,
    interaction: &'static str,
}

impl TerrainSpriteProp {
    fn new(chunk_pos: Vec2<i32>, local_pos: Vec3<i32>, sprite: SpriteKind) -> Self {
        let rect_size = TerrainChunkSize::RECT_SIZE.as_::<i32>();
        let chunk_origin = chunk_pos * rect_size;
        Self {
            wpos: Vec3::new(
                chunk_origin.x + local_pos.x,
                chunk_origin.y + local_pos.y,
                local_pos.z,
            ),
            sprite,
            category: sprite.category(),
            interaction: sprite_interaction_label(sprite),
        }
    }
}

struct TerrainPropFocus<'a> {
    prop: &'a TerrainSpriteProp,
    distance: f32,
}

impl TerrainPropFocus<'_> {
    fn summary(&self) -> String {
        format!(
            "{} {:?} {:?} at {:.1}m",
            self.prop.interaction, self.prop.category, self.prop.sprite, self.distance
        )
    }

    fn action_summary(&self) -> String {
        format!(
            "{} {:?} {:?} at {:.1}m",
            self.prop.interaction, self.prop.category, self.prop.sprite, self.distance
        )
    }
}

struct EntityFocus<'a> {
    entity: &'a EntityInfo,
    distance: f32,
}

impl EntityFocus<'_> {
    fn summary(&self) -> String {
        let role = entity_role_label(self.entity);
        let body = entity_body_label(&self.entity.body);
        format!(
            "{} {} {:?} at {:.1}m",
            role, body, self.entity.alignment, self.distance
        )
    }

    fn action_summary(&self) -> String {
        let role = entity_role_label(self.entity);
        let verb = match role {
            "trader" => "open trade preview",
            "waypoint" => "focus waypoint",
            "teleporter" => "focus teleporter",
            "enemy" => "target enemy",
            "npc" => "talk preview",
            "ally" | "passive" | "wild" => "inspect creature",
            _ => "inspect entity",
        };
        format!("{verb} {}", self.summary())
    }
}

struct SiteMarkerFocus<'a> {
    marker: &'a OriginalSiteMarker,
    distance: f32,
}

impl SiteMarkerFocus<'_> {
    fn summary(&self) -> String {
        let mut summary = format!(
            "{} {} in {} at {:.1}m",
            site_marker_role_label(self.marker.kind),
            self.marker.label,
            self.marker.site_name,
            self.distance
        );
        if let Some(trade_preview) = &self.marker.trade_preview {
            summary.push_str(&format!(
                " (trade data: {} wares)",
                trade_preview.wares.len()
            ));
        }
        summary
    }

    fn action_summary(&self) -> String {
        let verb = match self.marker.kind {
            OriginalSiteMarkerKind::Trader | OriginalSiteMarkerKind::Market => "open trade preview",
            OriginalSiteMarkerKind::Captain | OriginalSiteMarkerKind::Adventurer => "talk preview",
            OriginalSiteMarkerKind::Guard => "talk guard",
        };
        let mut summary = format!("{verb} {}", self.summary());
        if let Some(trade_preview) = &self.marker.trade_preview {
            summary.push_str(&format!("; {}", trade_preview.summary()));
        }
        summary
    }

    fn trade_panel(&self) -> Option<TradePanelPreview> {
        self.marker.trade_preview.as_ref().map(|preview| {
            preview.panel(format!(
                "{} {} - {}",
                site_marker_role_label(self.marker.kind),
                self.marker.label,
                self.marker.site_name
            ))
        })
    }
}

enum EntityTargetFocus<'a> {
    Generated(EntityFocus<'a>),
    Site(SiteMarkerFocus<'a>),
}

impl EntityTargetFocus<'_> {
    fn distance(&self) -> f32 {
        match self {
            EntityTargetFocus::Generated(focus) => focus.distance,
            EntityTargetFocus::Site(focus) => focus.distance,
        }
    }

    fn summary(&self) -> String {
        match self {
            EntityTargetFocus::Generated(focus) => focus.summary(),
            EntityTargetFocus::Site(focus) => focus.summary(),
        }
    }

    fn action_summary(&self) -> String {
        match self {
            EntityTargetFocus::Generated(focus) => focus.action_summary(),
            EntityTargetFocus::Site(focus) => focus.action_summary(),
        }
    }

    fn trade_panel(&self) -> Option<TradePanelPreview> {
        match self {
            EntityTargetFocus::Generated(_) => None,
            EntityTargetFocus::Site(focus) => focus.trade_panel(),
        }
    }
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
    original_sites: usize,
    original_settlements: usize,
    original_pois: usize,
    rtsim_sites: usize,
    rtsim_existing_npcs: usize,
    rtsim_wanted_population: usize,
    rtsim_wanted_merchants: usize,
    rtsim_wanted_guards: usize,
    site_markers: &[OriginalSiteMarker],
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

    let vertical_origin = (min_z + max_z) as f32 * 0.5;
    let mut builder = PatchMeshBuilder::new(vertical_origin);
    let mut filled_blocks = 0usize;
    let mut liquid_blocks = 0usize;
    let mut terrain_sprite_props = 0usize;
    let mut generated_entity_spawns = 0usize;
    let mut entity_markers = Vec::new();
    let mut terrain_chunks = Vec::new();

    for preview_chunk in chunks {
        generated_entity_spawns += count_entity_spawns(&preview_chunk.entity_spawns);
        append_entity_markers(
            &preview_chunk.entity_spawns,
            center_chunk_pos,
            vertical_origin,
            &mut entity_markers,
        );
        let chunk_origin = relative_chunk_origin(preview_chunk.pos, center_chunk_pos);
        let mut chunk_builder = PatchMeshBuilder::new(vertical_origin);
        chunk_builder.append_chunk_mesh(&preview_chunk.mesh, chunk_origin);
        builder.append_chunk_mesh(&preview_chunk.mesh, chunk_origin);
        terrain_chunks.push(OriginalTerrainChunkMesh {
            chunk_pos: chunk_key(preview_chunk.pos),
            vertices: chunk_builder.vertices,
            indices: chunk_builder.indices,
        });
        filled_blocks += preview_chunk.mesh.filled_blocks;
        liquid_blocks += preview_chunk.mesh.liquid_blocks;
        terrain_sprite_props += preview_chunk.mesh.sprite_props;
    }

    let (site_npc_markers, site_trader_markers, site_market_markers) = append_site_markers(
        site_markers,
        center_chunk_pos,
        chunk_radius,
        vertical_origin,
        &mut entity_markers,
    );

    if builder.indices.is_empty() {
        return Err(format!(
            "original TerrainChunk patch around {center_chunk_pos:?} produced no visible block \
             faces"
        ));
    }

    let patch_side = (chunk_radius * 2 + 1).max(1) as u32;
    Ok(OriginalWorldMesh {
        terrain_chunks,
        entity_markers,
        chunk_dimensions: (dimensions.x, dimensions.y),
        center_chunk_pos: (center_chunk_pos.x, center_chunk_pos.y),
        chunk_patch: (patch_side, patch_side),
        vertical_origin,
        generated_chunks: chunks.len(),
        newly_generated_chunks,
        cached_chunks,
        newly_meshed_chunks: newly_generated_chunks,
        cached_mesh_chunks: cached_chunks,
        terrain_faces: builder.face_count,
        filled_blocks,
        liquid_blocks,
        terrain_sprite_props,
        generated_entity_spawns,
        enabled_world_features,
        wildlife_spawn_manifests,
        original_sites,
        original_settlements,
        original_pois,
        rtsim_sites,
        rtsim_existing_npcs,
        rtsim_wanted_population,
        rtsim_wanted_merchants,
        rtsim_wanted_guards,
        site_npc_markers,
        site_trader_markers,
        site_market_markers,
        seed,
    })
}

fn append_site_markers(
    site_markers: &[OriginalSiteMarker],
    center_chunk_pos: Vec2<i32>,
    chunk_radius: i32,
    vertical_origin: f32,
    entity_markers: &mut Vec<OriginalEntityMarker>,
) -> (usize, usize, usize) {
    let rect_size = TerrainChunkSize::RECT_SIZE.as_::<f32>();
    let min_chunk = center_chunk_pos - Vec2::broadcast(chunk_radius);
    let max_chunk = center_chunk_pos + Vec2::broadcast(chunk_radius + 1);
    let min_wpos = min_chunk.as_::<f32>() * rect_size;
    let max_wpos = max_chunk.as_::<f32>() * rect_size;
    let mut npc_count = 0usize;
    let mut trader_count = 0usize;
    let mut market_count = 0usize;

    for marker in site_markers {
        if marker.wpos.x < min_wpos.x
            || marker.wpos.y < min_wpos.y
            || marker.wpos.x >= max_wpos.x
            || marker.wpos.y >= max_wpos.y
        {
            continue;
        }

        match marker.kind {
            OriginalSiteMarkerKind::Trader | OriginalSiteMarkerKind::Captain => trader_count += 1,
            OriginalSiteMarkerKind::Market => market_count += 1,
            OriginalSiteMarkerKind::Guard | OriginalSiteMarkerKind::Adventurer => npc_count += 1,
        }
        entity_markers.push(site_marker_to_entity_marker(
            marker,
            center_chunk_pos,
            vertical_origin,
        ));
    }

    (npc_count, trader_count, market_count)
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

fn entity_spawn_entities(entity_spawn: &EntitySpawn) -> Box<dyn Iterator<Item = &EntityInfo> + '_> {
    match entity_spawn {
        EntitySpawn::Entity(entity) => Box::new(std::iter::once(entity.as_ref())),
        EntitySpawn::Group(group) => Box::new(group.iter()),
    }
}

fn vec2_distance_squared(a: Vec2<f32>, b: Vec2<f32>) -> f32 {
    let delta = a - b;
    delta.x * delta.x + delta.y * delta.y
}

fn sprite_interaction_label(sprite: SpriteKind) -> &'static str {
    if sprite.is_defined_as_container() {
        "container"
    } else if sprite_is_unlock(sprite) {
        "unlock"
    } else if sprite.mine_tool().is_some() {
        "mine"
    } else if sprite.collectible_info().is_some() {
        match sprite.category() {
            Category::Plant | Category::Resource => "collect",
            _ => "interact",
        }
    } else {
        match sprite.category() {
            Category::Lamp => "light",
            Category::Furniture | Category::Structural | Category::Modular => "inspect",
            _ => "prop",
        }
    }
}

fn sprite_is_unlock(sprite: SpriteKind) -> bool {
    matches!(
        sprite,
        SpriteKind::Keyhole
            | SpriteKind::BoneKeyhole
            | SpriteKind::HaniwaKeyhole
            | SpriteKind::SahaginKeyhole
            | SpriteKind::VampireKeyhole
            | SpriteKind::GlassKeyhole
            | SpriteKind::KeyholeBars
            | SpriteKind::TerracottaKeyhole
            | SpriteKind::MyrmidonKeyhole
            | SpriteKind::MinotaurKeyhole
    )
}

fn entity_role_label(entity: &EntityInfo) -> &'static str {
    if entity.trading_information.is_some() {
        "trader"
    } else if let Some(special_entity) = &entity.special_entity {
        match special_entity {
            SpecialEntity::Waypoint => "waypoint",
            SpecialEntity::Teleporter(_) => "teleporter",
            SpecialEntity::ArenaTotem { .. } => "arena totem",
        }
    } else if entity.has_agency {
        match entity.alignment {
            Alignment::Enemy => "enemy",
            Alignment::Npc => "npc",
            Alignment::Tame | Alignment::Owned(_) => "ally",
            Alignment::Passive => "passive",
            Alignment::Wild => "wild",
        }
    } else {
        "static"
    }
}

fn site_marker_role_label(kind: OriginalSiteMarkerKind) -> &'static str {
    match kind {
        OriginalSiteMarkerKind::Trader => "trader",
        OriginalSiteMarkerKind::Market => "market",
        OriginalSiteMarkerKind::Captain => "captain",
        OriginalSiteMarkerKind::Guard => "guard",
        OriginalSiteMarkerKind::Adventurer => "adventurer",
    }
}

fn entity_body_label(body: &Body) -> &'static str {
    match body {
        Body::Humanoid(_) => "humanoid",
        Body::BipedSmall(_) => "small biped",
        Body::BipedLarge(_) => "large biped",
        Body::QuadrupedSmall(_) => "small quadruped",
        Body::QuadrupedMedium(_) => "quadruped",
        Body::QuadrupedLow(_) => "low quadruped",
        Body::Theropod(_) => "theropod",
        Body::BirdMedium(_) => "bird",
        Body::BirdLarge(_) => "large bird",
        Body::Dragon(_) => "dragon",
        Body::FishSmall(_) => "small fish",
        Body::FishMedium(_) => "fish",
        Body::Golem(_) => "golem",
        Body::Object(_) => "object",
        Body::Item(_) => "item",
        Body::Ship(_) => "ship",
        Body::Crustacean(_) => "crustacean",
        Body::Arthropod(_) => "arthropod",
        Body::Plugin(_) => "plugin",
    }
}

fn build_chunk_mesh_fragment(chunk: &TerrainChunk, chunk_pos: Vec2<i32>) -> ChunkMeshFragment {
    let mut builder = BlockMeshBuilder::new();
    let mut filled_blocks = 0usize;
    let mut liquid_blocks = 0usize;
    let mut terrain_props = Vec::new();

    for (pos, block) in chunk.iter_changed() {
        if let Some(sprite) = block
            .get_sprite()
            .filter(|sprite| *sprite != SpriteKind::Empty)
        {
            builder.add_sprite_prop(pos, block, sprite);
            terrain_props.push(TerrainSpriteProp::new(chunk_pos, pos, sprite));
        }

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

    ChunkMeshFragment {
        vertices: builder.vertices,
        indices: builder.indices,
        face_count: builder.face_count,
        filled_blocks,
        liquid_blocks,
        sprite_props: terrain_props.len(),
        terrain_props,
    }
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
        yaw_radians: 0.0,
    }
}

fn site_marker_to_entity_marker(
    marker: &OriginalSiteMarker,
    center_chunk_pos: Vec2<i32>,
    vertical_origin: f32,
) -> OriginalEntityMarker {
    let center = chunk_center_wpos(center_chunk_pos);
    let render_pos = [
        (marker.wpos.x - center.x) * TERRAIN_HORIZONTAL_SCALE,
        (marker.wpos.z - vertical_origin) * 0.28 + 0.25,
        (marker.wpos.y - center.y) * TERRAIN_HORIZONTAL_SCALE,
    ];
    let (radius, height, shape) = match marker.kind {
        OriginalSiteMarkerKind::Market => (0.42, 0.7, OriginalEntityMarkerShape::Object),
        OriginalSiteMarkerKind::Guard => (0.38, 1.55, OriginalEntityMarkerShape::Humanoid),
        _ => (0.34, 1.45, OriginalEntityMarkerShape::Humanoid),
    };
    OriginalEntityMarker {
        render_pos,
        radius,
        height,
        color: site_marker_color(marker.kind),
        shape,
        yaw_radians: 0.0,
    }
}

fn site_marker_color(kind: OriginalSiteMarkerKind) -> [f32; 3] {
    match kind {
        OriginalSiteMarkerKind::Trader => [1.0, 0.78, 0.18],
        OriginalSiteMarkerKind::Market => [1.0, 0.62, 0.14],
        OriginalSiteMarkerKind::Captain => [0.38, 0.78, 1.0],
        OriginalSiteMarkerKind::Guard => [0.92, 0.28, 0.28],
        OriginalSiteMarkerKind::Adventurer => [0.62, 0.52, 1.0],
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

struct PatchMeshBuilder {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    face_count: usize,
    vertical_origin: f32,
}

impl PatchMeshBuilder {
    fn new(vertical_origin: f32) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            face_count: 0,
            vertical_origin,
        }
    }

    fn append_chunk_mesh(&mut self, mesh: &ChunkMeshFragment, chunk_origin: Vec2<i32>) {
        let base = (self.vertices.len() / FLOATS_PER_VERTEX) as u32;
        for vertex in mesh.vertices.chunks_exact(FLOATS_PER_VERTEX) {
            let [x, y, z] = self.render_point([vertex[0], vertex[1], vertex[2]], chunk_origin);
            self.vertices
                .extend_from_slice(&[x, y, z, vertex[3], vertex[4], vertex[5]]);
        }
        self.indices
            .extend(mesh.indices.iter().map(|index| base + *index));
        self.face_count += mesh.face_count;
    }

    fn render_point(&self, point: [f32; 3], chunk_origin: Vec2<i32>) -> [f32; 3] {
        let chunk_center = TerrainChunkSize::RECT_SIZE.x as f32 * 0.5;
        [
            (point[0] + chunk_origin.x as f32 - chunk_center) * TERRAIN_HORIZONTAL_SCALE,
            (point[1] - self.vertical_origin) * 0.28,
            (point[2] + chunk_origin.y as f32 - chunk_center) * TERRAIN_HORIZONTAL_SCALE,
        ]
    }
}

struct BlockMeshBuilder {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    face_count: usize,
}

impl BlockMeshBuilder {
    fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            face_count: 0,
        }
    }

    fn add_face(&mut self, pos: Vec3<i32>, block: &Block, face: Face) {
        let base = (self.vertices.len() / FLOATS_PER_VERTEX) as u32;
        let color = face.shade_color(block_color(block));
        for corner in face.corners(pos) {
            let [x, y, z] = Self::render_point(corner);
            self.vertices
                .extend_from_slice(&[x, y, z, color[0], color[1], color[2]]);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.face_count += 1;
    }

    fn add_sprite_prop(&mut self, pos: Vec3<i32>, block: &Block, sprite: SpriteKind) {
        let style = sprite_prop_style(block, sprite);
        match style.shape {
            SpritePropShape::Cross => self.add_sprite_cross(pos, style),
            SpritePropShape::Cuboid => self.add_sprite_cuboid(pos, style),
            SpritePropShape::Pillar => {
                self.add_sprite_cuboid(pos, SpritePropStyle {
                    width: style.width * 0.52,
                    depth: style.depth * 0.52,
                    ..style
                });
                self.add_sprite_cuboid(pos + Vec3::unit_z(), SpritePropStyle {
                    height: 0.18,
                    width: style.width,
                    depth: style.depth,
                    color: scale_color(style.color, 1.26),
                    shape: SpritePropShape::Cuboid,
                });
            },
        }
    }

    fn add_sprite_cross(&mut self, pos: Vec3<i32>, style: SpritePropStyle) {
        let x = pos.x as f32 + 0.5;
        let y = pos.y as f32 + 0.5;
        let z0 = pos.z as f32;
        let z1 = z0 + style.height;
        let half_width = style.width * 0.5;
        let color = scale_color(style.color, 1.08);

        self.add_sprite_quad(
            [
                [x - half_width, y, z0],
                [x + half_width, y, z0],
                [x + half_width, y, z1],
                [x - half_width, y, z1],
            ],
            color,
        );
        self.add_sprite_quad(
            [
                [x, y - half_width, z0],
                [x, y + half_width, z0],
                [x, y + half_width, z1],
                [x, y - half_width, z1],
            ],
            scale_color(color, 0.90),
        );
    }

    fn add_sprite_cuboid(&mut self, pos: Vec3<i32>, style: SpritePropStyle) {
        let x = pos.x as f32 + 0.5;
        let y = pos.y as f32 + 0.5;
        let z0 = pos.z as f32;
        let z1 = z0 + style.height.max(0.05);
        let hx = (style.width * 0.5).max(0.03);
        let hy = (style.depth * 0.5).max(0.03);
        let corners = [
            [x - hx, y - hy, z0],
            [x + hx, y - hy, z0],
            [x + hx, y + hy, z0],
            [x - hx, y + hy, z0],
            [x - hx, y - hy, z1],
            [x + hx, y - hy, z1],
            [x + hx, y + hy, z1],
            [x - hx, y + hy, z1],
        ];
        const FACES: [[usize; 4]; 6] = [
            [0, 1, 2, 3],
            [4, 7, 6, 5],
            [0, 4, 5, 1],
            [1, 5, 6, 2],
            [2, 6, 7, 3],
            [3, 7, 4, 0],
        ];
        for (face_index, face) in FACES.iter().enumerate() {
            self.add_sprite_quad(
                [
                    corners[face[0]],
                    corners[face[1]],
                    corners[face[2]],
                    corners[face[3]],
                ],
                sprite_face_color(style.color, face_index),
            );
        }
    }

    fn add_sprite_quad(&mut self, corners: [[f32; 3]; 4], color: [f32; 3]) {
        let base = (self.vertices.len() / FLOATS_PER_VERTEX) as u32;
        for corner in corners {
            let [x, y, z] = Self::render_point(corner);
            self.vertices
                .extend_from_slice(&[x, y, z, color[0], color[1], color[2]]);
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.face_count += 1;
    }

    fn render_point(point: [f32; 3]) -> [f32; 3] { [point[0], point[2], point[1]] }
}

#[derive(Clone, Copy)]
struct SpritePropStyle {
    shape: SpritePropShape,
    width: f32,
    depth: f32,
    height: f32,
    color: [f32; 3],
}

#[derive(Clone, Copy)]
enum SpritePropShape {
    Cross,
    Cuboid,
    Pillar,
}

fn sprite_prop_style(block: &Block, sprite: SpriteKind) -> SpritePropStyle {
    let category = sprite.category();
    let height = sprite_prop_height(block, sprite, category);
    let (shape, width, depth) = match category {
        Category::Plant => (SpritePropShape::Cross, 0.72, 0.08),
        Category::Resource => (SpritePropShape::Cuboid, 0.44, 0.44),
        Category::MineableResource => (SpritePropShape::Cuboid, 0.58, 0.58),
        Category::Furniture | Category::Container | Category::Structural | Category::Modular => {
            (SpritePropShape::Cuboid, 0.78, 0.78)
        },
        Category::Lamp => (SpritePropShape::Pillar, 0.62, 0.62),
        Category::Decor => (SpritePropShape::Cuboid, 0.54, 0.54),
        Category::Misc => (SpritePropShape::Cuboid, 0.42, 0.42),
        Category::Void => (SpritePropShape::Cuboid, 0.0, 0.0),
    };

    let mut color = sprite_prop_color(sprite);
    if sprite.collectible_info().is_some() {
        color = blend_color(color, [1.0, 0.88, 0.34], 0.10);
    }

    SpritePropStyle {
        shape,
        width,
        depth,
        height,
        color,
    }
}

fn sprite_prop_height(block: &Block, sprite: SpriteKind, category: Category) -> f32 {
    let category_height = match category {
        Category::Void => 0.0,
        Category::Plant => 0.74,
        Category::Resource => 0.38,
        Category::MineableResource => 0.52,
        Category::Furniture => 1.05,
        Category::Structural => 1.18,
        Category::Decor => 0.72,
        Category::Lamp => 1.55,
        Category::Container => 0.86,
        Category::Modular => 1.0,
        Category::Misc => 0.38,
    };
    let sprite_height = sprite
        .solid_height()
        .filter(|height| *height > 0.0)
        .unwrap_or_else(|| block.solid_height().max(category_height));
    sprite_height.clamp(0.12, 2.8)
}

fn sprite_prop_color(sprite: SpriteKind) -> [f32; 3] {
    match sprite {
        SpriteKind::BlueFlower => [0.28, 0.48, 1.0],
        SpriteKind::PinkFlower => [1.0, 0.46, 0.76],
        SpriteKind::PurpleFlower | SpriteKind::Moonbell => [0.70, 0.42, 1.0],
        SpriteKind::RedFlower | SpriteKind::Pyrebloom => [1.0, 0.22, 0.18],
        SpriteKind::WhiteFlower => [0.94, 0.94, 0.86],
        SpriteKind::YellowFlower | SpriteKind::Sunflower => [1.0, 0.82, 0.18],
        SpriteKind::LanternFlower | SpriteKind::LanternPlant => [0.72, 1.0, 0.68],
        SpriteKind::LushFlower => [0.52, 0.94, 0.42],
        SpriteKind::LongGrass
        | SpriteKind::MediumGrass
        | SpriteKind::ShortGrass
        | SpriteKind::LargeGrass
        | SpriteKind::GrassBlue
        | SpriteKind::GrassBlueShort
        | SpriteKind::GrassBlueMedium
        | SpriteKind::GrassBlueLong
        | SpriteKind::SavannaGrass
        | SpriteKind::TallSavannaGrass
        | SpriteKind::RedSavannaGrass
        | SpriteKind::TaigaGrass => [0.22, 0.54, 0.20],
        SpriteKind::Fern
        | SpriteKind::JungleFern
        | SpriteKind::LeafyPlant
        | SpriteKind::JungleLeafyPlant => [0.12, 0.46, 0.18],
        SpriteKind::DeadBush | SpriteKind::DeadPlant | SpriteKind::Twigs => [0.48, 0.34, 0.18],
        SpriteKind::Mushroom
        | SpriteKind::CaveMushroom
        | SpriteKind::SewerMushroom
        | SpriteKind::LushMushroom
        | SpriteKind::RockyMushroom => [0.78, 0.42, 0.30],
        SpriteKind::GlowMushroom | SpriteKind::CeilingMushroom => [0.58, 0.38, 1.0],
        SpriteKind::Reed | SpriteKind::SporeReed | SpriteKind::WildFlax | SpriteKind::Flax => {
            [0.38, 0.58, 0.24]
        },
        SpriteKind::Pumpkin => [0.95, 0.44, 0.08],
        SpriteKind::Apple => [0.92, 0.12, 0.08],
        SpriteKind::Coconut => [0.45, 0.30, 0.13],
        SpriteKind::Beehive => [0.92, 0.66, 0.22],
        SpriteKind::Stones | SpriteKind::Stones2 | SpriteKind::Mud | SpriteKind::Grave => {
            [0.48, 0.48, 0.48]
        },
        SpriteKind::Wood
        | SpriteKind::Bamboo
        | SpriteKind::Hardwood
        | SpriteKind::Ironwood
        | SpriteKind::Frostwood
        | SpriteKind::Eldwood => [0.42, 0.24, 0.10],
        SpriteKind::Amethyst => [0.58, 0.28, 0.84],
        SpriteKind::Ruby | SpriteKind::Bloodstone => [0.80, 0.12, 0.10],
        SpriteKind::Sapphire | SpriteKind::Cobalt => [0.15, 0.32, 0.92],
        SpriteKind::Emerald => [0.14, 0.72, 0.32],
        SpriteKind::Topaz | SpriteKind::Gold => [1.0, 0.74, 0.16],
        SpriteKind::Diamond | SpriteKind::Velorite | SpriteKind::VeloriteFrag => [0.52, 0.92, 1.0],
        SpriteKind::Coal => [0.08, 0.08, 0.08],
        SpriteKind::Copper => [0.80, 0.42, 0.18],
        SpriteKind::Iron | SpriteKind::Tin | SpriteKind::Silver | SpriteKind::Lodestone => {
            [0.62, 0.66, 0.70]
        },
        SpriteKind::Lantern
        | SpriteKind::StreetLamp
        | SpriteKind::StreetLampTall
        | SpriteKind::MesaLantern
        | SpriteKind::SeashellLantern
        | SpriteKind::BonfireMLit => [1.0, 0.70, 0.24],
        sprite if sprite.is_defined_as_container() => [0.54, 0.30, 0.12],
        _ => match sprite.category() {
            Category::Plant => [0.20, 0.48, 0.18],
            Category::Resource => [0.56, 0.42, 0.24],
            Category::MineableResource => [0.50, 0.54, 0.58],
            Category::Furniture => [0.46, 0.28, 0.16],
            Category::Structural => [0.42, 0.44, 0.48],
            Category::Decor => [0.64, 0.52, 0.38],
            Category::Lamp => [0.98, 0.66, 0.25],
            Category::Container => [0.54, 0.30, 0.12],
            Category::Modular => [0.45, 0.42, 0.36],
            Category::Misc => [0.58, 0.58, 0.60],
            Category::Void => [0.0, 0.0, 0.0],
        },
    }
}

fn sprite_face_color(color: [f32; 3], face_index: usize) -> [f32; 3] {
    let shade = match face_index {
        1 => 1.12,
        0 => 0.62,
        2 | 3 => 0.86,
        _ => 0.74,
    };
    scale_color(color, shade)
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
    let kind = block.kind();
    let base_color = block
        .get_color()
        .map(|color| rgb8_to_unit([color.r, color.g, color.b]))
        .unwrap_or_else(|| fallback_block_color(kind));

    match kind {
        BlockKind::Air => base_color,
        BlockKind::Water => blend_color(base_color, [0.09, 0.34, 0.82], 0.86),
        BlockKind::Lava => scale_color(blend_color(base_color, [1.0, 0.24, 0.02], 0.72), 1.32),
        BlockKind::GlowingRock | BlockKind::GlowingWeakRock | BlockKind::GlowingMushroom => {
            scale_color(blend_color(base_color, [0.78, 0.95, 1.0], 0.18), 1.22)
        },
        BlockKind::Grass => blend_color(base_color, [0.18, 0.48, 0.18], 0.26),
        BlockKind::Snow | BlockKind::ArtSnow => {
            scale_color(blend_color(base_color, [0.84, 0.90, 1.0], 0.34), 1.08)
        },
        BlockKind::Ice => scale_color(blend_color(base_color, [0.48, 0.78, 1.0], 0.40), 1.08),
        BlockKind::Leaves | BlockKind::ArtLeaves => {
            blend_color(base_color, [0.10, 0.40, 0.12], 0.22)
        },
        BlockKind::Wood => blend_color(base_color, [0.40, 0.20, 0.08], 0.22),
        BlockKind::Sand => blend_color(base_color, [0.86, 0.70, 0.42], 0.18),
        BlockKind::Earth => blend_color(base_color, [0.42, 0.28, 0.16], 0.18),
        BlockKind::Rock | BlockKind::WeakRock => blend_color(base_color, [0.50, 0.52, 0.56], 0.10),
        BlockKind::Misc => base_color,
    }
}

fn fallback_block_color(kind: BlockKind) -> [f32; 3] {
    match kind {
        BlockKind::Air => [0.0, 0.0, 0.0],
        BlockKind::Water => [0.08, 0.30, 0.76],
        BlockKind::Rock | BlockKind::WeakRock => [0.48, 0.48, 0.52],
        BlockKind::Lava => [1.0, 0.26, 0.02],
        BlockKind::GlowingRock | BlockKind::GlowingWeakRock => [0.62, 0.82, 0.98],
        BlockKind::Grass => [0.20, 0.48, 0.18],
        BlockKind::Snow | BlockKind::ArtSnow => [0.86, 0.90, 1.0],
        BlockKind::Earth => [0.38, 0.25, 0.14],
        BlockKind::Sand => [0.78, 0.64, 0.38],
        BlockKind::Wood => [0.42, 0.24, 0.10],
        BlockKind::Leaves | BlockKind::ArtLeaves => [0.10, 0.36, 0.12],
        BlockKind::GlowingMushroom => [0.68, 0.36, 1.0],
        BlockKind::Ice => [0.48, 0.74, 1.0],
        BlockKind::Misc => [0.58, 0.58, 0.62],
    }
}

fn rgb8_to_unit(color: [u8; 3]) -> [f32; 3] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ]
}

fn blend_color(a: [f32; 3], b: [f32; 3], amount: f32) -> [f32; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * amount,
        a[1] + (b[1] - a[1]) * amount,
        a[2] + (b[2] - a[2]) * amount,
    ]
}

fn scale_color(color: [f32; 3], scale: f32) -> [f32; 3] {
    color.map(|channel| (channel * scale).min(1.0))
}
