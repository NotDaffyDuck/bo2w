//! Golden tests for `rebuild_static_draw_lane`: seeded fixtures run through
//! eight frames each (full build, reuse, LOD hold, world-only and
//! smodel-only rebuilds, new material generation, new world generation,
//! eye moved), and every frame's lane output is hashed against
//! `retained_golden.txt`. A rewrite of the static lane has to keep every hash.
//!
//! `GOLDEN_BLESS=1` rewrites the golden file; `GOLDEN_DUMP=<dir>` writes each
//! frame's snapshot text as `s<seed>-<step>.txt` for diffing.

use std::fmt::Write as _;
use std::sync::Arc;

use asset_core::AssetNamespace;
use asset_model::{LodParmsAxis, ModelLodSelector};
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use dpvs_t6::{GfxDrawSurfFields, pack};
use frame::WorldGeneration;
use lighting_t6::SmodelSurfPath;

use super::list::{DrawSurfItem, DrawSurfList};
use super::retained::{RetainedDrawKind, StaticDrawLane, rebuild_static_draw_lane};
use super::tess::smodel::{SmodelGpuPlan, SmodelMeshSurfaces, SmodelPassMaterial, SmodelPlacement};
use super::{
    MaterialAssetId, MaterialGeneration, MaterialGenerationId, RemapResolution, RuntimeMaterial,
    RuntimeMaterialCatalog, RuntimeSortedMaterialTable, RuntimeTechniqueSetId,
    SortedMaterialOrdinal, SurfaceLightmapId, SurfaceReflectionProbeId, SurfaceSamplerInputs,
    WorldDrawGpuPlan,
};
use crate::prepare::scene::cull::DpvsFrameStats;
use crate::prepare::scene::smodel_geom_cache::{
    LodRampDvar, PretessDvar, SmcEnableDvar, WorldStaticModelCache,
};
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;
use crate::prepare::scene::view_parms::PreparedSceneView;
use crate::prepare::scene::world::WorldDrawItemKind;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/drawsurf/retained_golden.txt"
);
const SEEDS: u64 = 8;
const MATERIALS: usize = 24;
const RANKS: u64 = 32;
const PASS_MATERIALS: usize = 12;
const MESHES: usize = 10;
const WORLD_SURFS: u64 = 400;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }

    fn span(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.below(1 << 20) as f32 / (1 << 20) as f32)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Eye {
    Missing,
    NotReady,
    Ready,
}

/// What one seed's world has and lacks.
#[derive(Clone, Copy)]
struct Shape {
    eye: Eye,
    dpvs: bool,
    vis: bool,
    pretess: bool,
    smc: Option<bool>,
    lighting: bool,
    cache: bool,
    sorted: bool,
    world_plan: bool,
    smodel_plan: bool,
    world_generation: bool,
    dense_skinned: bool,
    all_cached: bool,
}

fn shape(seed: u64) -> Shape {
    let base = Shape {
        eye: Eye::Ready,
        dpvs: true,
        vis: true,
        pretess: true,
        smc: None,
        lighting: true,
        cache: true,
        sorted: true,
        world_plan: true,
        smodel_plan: true,
        world_generation: true,
        dense_skinned: false,
        all_cached: false,
    };
    match seed {
        0 => Shape {
            eye: Eye::Missing,
            vis: false,
            ..base
        },
        1 => Shape {
            smc: Some(true),
            ..base
        },
        2 => Shape {
            pretess: false,
            smc: Some(false),
            ..base
        },
        3 => Shape {
            eye: Eye::NotReady,
            lighting: false,
            cache: false,
            ..base
        },
        4 => Shape {
            sorted: false,
            world_plan: false,
            smodel_plan: false,
            ..base
        },
        5 => Shape {
            dense_skinned: true,
            ..base
        },
        6 => Shape {
            eye: Eye::Missing,
            dpvs: false,
            smc: Some(true),
            world_generation: false,
            ..base
        },
        _ => Shape {
            smc: Some(true),
            all_cached: true,
            ..base
        },
    }
}

fn material(i: usize, rng: &mut Rng) -> RuntimeMaterial {
    RuntimeMaterial {
        // One row whose id does not match its slot: lookups through it miss.
        asset_id: MaterialAssetId(if i == 7 { 99 } else { i as u16 }),
        name: format!("m{i}"),
        // BO1 materials never reach a BO2 draw lane.
        namespace: AssetNamespace::T6,
        technique_set: String::new(),
        baked_draw_surf: None,
        local_technique_set: RuntimeTechniqueSetId(0),
        remap: RemapResolution::SelfSet,
        state_bits_entry: None,
        state_bits_table: Vec::new(),
        camera_region: rng.below(5) as u8,
        sort_key: rng.below(64) as u8,
        info_game_flags: rng.below(256) as u8,
        state_flags: 0,
        surface_type_bits: None,
        unlit: false,
        uses_model_lighting_const: false,
        square_color_map: false,
        shadow_only: false,
        t6_draw: None,
        cull_mode: None,
        uv_anim_bits: [0; 4],
        falloff_parms_bits: [0; 4],
        falloff_begin_bits: [0; 4],
        falloff_end_bits: [0; 4],
        env_map_parms_bits: [0; 4],
        textures: Vec::new(),
        constants: Vec::new(),
    }
}

fn catalog(rng: &mut Rng, shape: Shape, generation: u64) -> RuntimeMaterialCatalog {
    let materials = (0..MATERIALS).map(|i| material(i, rng)).collect();
    let sorted_materials = if shape.sorted {
        let mut order: Vec<usize> = (0..MATERIALS).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let mut asset_ids_by_ordinal: Vec<MaterialAssetId> =
            order.iter().map(|&i| MaterialAssetId(i as u16)).collect();
        // Ordinals past the catalog's end.
        asset_ids_by_ordinal.push(MaterialAssetId(200));
        asset_ids_by_ordinal.push(MaterialAssetId(201));
        let mut ordinals_by_asset_id = vec![None; MATERIALS];
        for (ordinal, &asset) in order.iter().enumerate() {
            ordinals_by_asset_id[asset] = Some(SortedMaterialOrdinal::new(ordinal as u32).unwrap());
        }
        RuntimeSortedMaterialTable::Ready {
            asset_ids_by_ordinal,
            ordinals_by_asset_id,
            skipped_n: 0,
            slot_gap_n: 0,
        }
    } else {
        RuntimeSortedMaterialTable::default()
    };
    RuntimeMaterialCatalog {
        generation_id: MaterialGenerationId(generation),
        materials,
        sorted_materials,
        ..Default::default()
    }
}

fn world_key(rng: &mut Rng) -> u64 {
    pack(GfxDrawSurfFields {
        material_sorted_index: rng.below(RANKS) as u16,
        primary_sort_key: rng.below(16) as u8,
        surf_type: rng.below(4) as u8,
        custom_index: rng.below(8) as u16,
        ..Default::default()
    })
    .packed
}

fn world_list(rng: &mut Rng, seed: u64) -> DrawSurfList {
    let kinds = [
        asset_world::CameraRangeKind::LitOpaque,
        asset_world::CameraRangeKind::LitTrans,
        asset_world::CameraRangeKind::Decal,
        asset_world::CameraRangeKind::Emissive,
    ];
    let mut list = DrawSurfList {
        draw_items_id: 0x1000 + seed,
        ..Default::default()
    };
    for _ in 0..60 {
        let key = world_key(rng);
        if rng.chance(20) {
            list.items.push(DrawSurfItem {
                key,
                surf: rng.below(WORLD_SURFS) as u16,
                run: 1,
                list_start: 0,
                entry_first: 0,
                kind: WorldDrawItemKind::BModel,
            });
            continue;
        }
        let run = 1 + rng.below(6);
        let first = rng.below(WORLD_SURFS - run);
        let list_start = list.surfs.len() as u32;
        let mut surf = first;
        for _ in 0..run {
            list.surfs.push(surf as u16);
            surf += 1 + u64::from(rng.chance(15)) * rng.below(9);
            surf = surf.min(WORLD_SURFS - 1);
        }
        list.items.push(DrawSurfItem {
            key,
            surf: first as u16,
            run: run as u16,
            list_start,
            entry_first: rng.below(8) as u16,
            kind: WorldDrawItemKind::Bsp(kinds[rng.below(4) as usize]),
        });
    }
    list.keys = list.items.len() as u32;
    list
}

fn world_plan(rng: &mut Rng) -> WorldDrawGpuPlan {
    let mut blocks = Vec::new();
    for _ in 0..WORLD_SURFS / 8 {
        blocks.push(SurfaceSamplerInputs {
            reflection_probe: match rng.below(3) {
                0 => None,
                n => Some(SurfaceReflectionProbeId(n as u8)),
            },
            primary_lightmap: rng
                .chance(70)
                .then(|| SurfaceLightmapId(rng.below(3) as u8)),
            secondary_lightmap: None,
        });
    }
    WorldDrawGpuPlan {
        surface_sampler_inputs: (0..WORLD_SURFS as usize).map(|s| blocks[s / 8]).collect(),
        ..Default::default()
    }
}

fn authored(rng: &mut Rng) -> Option<assets::MaterialIndex> {
    rng.chance(85)
        .then(|| assets::MaterialIndex::from_order(rng.below(MATERIALS as u64) as usize))
}

/// Mesh kinds by `i % 4`: rigid, skinned, cache-capable rigid, and one whose
/// odd LODs carry an unread XSurface byte.
fn mesh(i: usize, rng: &mut Rng) -> SmodelMeshSurfaces {
    let kind = i % 4;
    let mut mesh = SmodelMeshSurfaces::default();
    for lod in 0..4 {
        let n = 1 + rng.below(3) as usize;
        let surfs: Vec<_> = (0..n)
            .map(|s| ((i * 16 + lod * 4 + s) as u32, authored(rng)))
            .collect();
        mesh.xsurface_plus_1_by_lod[lod] = (0..n)
            .map(|s| match kind {
                1 if s == 0 => Some(1 + rng.below(3) as u8),
                3 if lod % 2 == 1 && s == n - 1 => None,
                _ => Some(0),
            })
            .collect();
        mesh.surfaces_by_lod[lod] = surfs;
    }
    if kind == 2 {
        mesh.lod_smc = Some([0, 1, 0, 0]);
        if i > 5 {
            mesh.lod_smc_rows = Some([[0, 1, 0, 0], [0, 1, 0, 0], [0, 0, 0, 0], [0, 1, 0, 0]]);
        }
    }
    if i % 3 != 0 {
        mesh.lod = Some(ModelLodSelector::T5 {
            num_lods: if i % 5 == 0 { 3 } else { 4 },
            lod_dist: [300.0, 900.0, 2000.0, 5000.0],
        });
    }
    mesh
}

fn pass_material(i: usize, rng: &mut Rng) -> SmodelPassMaterial {
    SmodelPassMaterial {
        model_lighting_required: i % 2 == 0,
        color: None,
        specular: None,
        probe: None,
        atlas: None,
        alpha_mode: AlphaMode::Opaque,
        draw_mode: None,
        cull_mode: None,
        env_map_parms: [0.0; 4],
        lighting_lookup_scale: [0.0; 4],
        atlas_lookup: [0.0; 4],
        sort_key: rng.below(64) as u8,
        material_sorted_index: (i % 6 != 5).then(|| rng.below(RANKS) as u32),
        t6_material: None,
    }
}

fn placement_mesh(rng: &mut Rng, shape: Shape) -> usize {
    if shape.all_cached {
        return [2, 6][rng.below(2) as usize];
    }
    if shape.dense_skinned && rng.chance(80) {
        return [1, 5, 9][rng.below(3) as usize];
    }
    rng.below(MESHES as u64) as usize
}

fn smodel_plan(rng: &mut Rng, shape: Shape, n: usize) -> SmodelGpuPlan {
    let mut plan = SmodelGpuPlan {
        materials: (0..PASS_MATERIALS).map(|i| pass_material(i, rng)).collect(),
        meshes: (0..MESHES).map(|i| mesh(i, rng)).collect(),
        ..Default::default()
    };
    let authored_all = std::iter::once(None)
        .chain((0..MATERIALS).map(|m| Some(assets::MaterialIndex::from_order(m))));
    for authored in authored_all {
        for probe in 0..3u8 {
            if rng.chance(90) {
                // Mostly a lit pass material (even), sometimes a mismatch.
                let even = 2 * rng.below(PASS_MATERIALS as u64 / 2) as u32;
                let material = if rng.chance(90) { even } else { even + 1 };
                plan.lit_material_key.insert((authored, probe), material);
            }
        }
        if rng.chance(90) {
            let odd = 2 * rng.below(PASS_MATERIALS as u64 / 2) as u32 + 1;
            let material = if rng.chance(90) { odd } else { odd - 1 };
            plan.unlit_material_key.insert(authored, material);
        }
    }
    for i in 0..n {
        let origin = [
            rng.span(-3000.0, 3000.0),
            rng.span(-3000.0, 3000.0),
            rng.span(-200.0, 400.0),
        ];
        let lighting_slot = match rng.below(100) {
            _ if shape.all_cached => Some(i),
            0..85 => Some(i),
            85..95 => None,
            _ => Some(70_000),
        };
        let packed_lighting = match rng.below(10) {
            _ if shape.all_cached => Some([1 + rng.below(255) as u8, 2, 3, 4]),
            0..4 => Some([rng.below(256) as u8, rng.below(256) as u8, 7, 1]),
            4 => Some([0; 4]),
            _ => None,
        };
        plan.placements.push(SmodelPlacement {
            mesh: placement_mesh(rng, shape),
            lit: shape.all_cached || rng.chance(60),
            reflection_probe_index: rng.below(3) as u8,
            primary_light_index: rng.below(4) as u8,
            flags: 0,
            lighting_slot,
            packed_lighting,
            entity: Entity::PLACEHOLDER,
            world_from_local: Mat4::from_translation(Vec3::from_array(origin)),
            origin,
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: rng.span(0.5, 2.0),
            t6_primary: [0; 2],
            t6_light: 0,
            t6_cull_dist: 0.0,
        });
    }
    plan
}

fn lighting(rng: &mut Rng, n: usize) -> WorldSmodelLighting {
    let dims = lighting_t6::model_lighting_atlas_dims(1).unwrap();
    let mut lighting = WorldSmodelLighting::new(dims, n, &[]);
    lighting.handles = (0..n)
        .map(|_| {
            if rng.chance(80) {
                1 + rng.below(500) as u16
            } else {
                0
            }
        })
        .collect();
    lighting.packed_lighting = (0..n)
        .map(|_| rng.chance(50).then(|| [rng.below(256) as u8, 9, 9, 1]))
        .collect();
    lighting
}

fn cache(rng: &mut Rng, plan: &SmodelGpuPlan, shape: Shape, n: usize) -> WorldStaticModelCache {
    let mut cache = WorldStaticModelCache::new(n);
    for row in cache.cache_index.iter_mut() {
        for index in row.iter_mut() {
            if shape.all_cached || rng.chance(70) {
                *index = 1 + rng.below(0x1fff) as u16;
            }
        }
    }
    for placement in &plan.placements {
        let Some(slot) = placement.lighting_slot.filter(|&slot| slot < n) else {
            continue;
        };
        for &(surface, _) in plan.meshes[placement.mesh].surfaces_by_lod.iter().flatten() {
            let roll = rng.below(100);
            if shape.all_cached || roll < 85 {
                let tris = 1 + rng.below(4) as usize;
                let run = (0..tris * 3).map(|_| rng.below(0x1000) as u16).collect();
                cache.index_runs.insert((slot as u32, surface), run);
            } else if roll < 90 {
                cache.index_runs.insert((slot as u32, surface), vec![1, 2, 3, 4]);
            }
        }
    }
    cache
}

fn eye_at(rng: &mut Rng) -> Vec3 {
    Vec3::new(
        rng.span(-1500.0, 1500.0),
        rng.span(-1500.0, 1500.0),
        rng.span(0.0, 200.0),
    )
}

fn build_world(seed: u64) -> World {
    let shape = shape(seed);
    let mut rng = Rng::new(seed + 1);
    let n = if shape.dense_skinned { 420 } else { 240 };
    let mut world = World::new();
    world.insert_resource(StaticDrawLane::default());
    world.insert_resource(world_list(&mut rng, seed));
    if shape.world_plan {
        world.insert_resource(world_plan(&mut rng));
    }
    let plan = smodel_plan(&mut rng, shape, n);
    if shape.lighting {
        world.insert_resource(lighting(&mut rng, n));
    }
    if shape.cache {
        world.insert_resource(cache(&mut rng, &plan, shape, n));
    }
    if shape.dpvs {
        world.insert_resource(DpvsFrameStats {
            smodel_vis: if shape.vis {
                (0..n).map(|_| u8::from(rng.chance(80))).collect()
            } else {
                Vec::new()
            },
            smodel_vis_id: 0x50 + seed,
            ..Default::default()
        });
    }
    if shape.smodel_plan {
        world.insert_resource(plan);
    }
    world.insert_resource(SmcEnableDvar { enabled: shape.smc });
    world.insert_resource(PretessDvar {
        enabled: shape.pretess,
    });
    world.insert_resource(LodRampDvar {
        t5: LodParmsAxis {
            applied_inv_scale: 1.0,
            scale: 1.0,
            bias: 0.0,
        },
        ..Default::default()
    });
    if shape.eye != Eye::Missing {
        world.insert_resource(PreparedSceneView {
            ready: shape.eye == Eye::Ready,
            eye: eye_at(&mut rng),
            ..Default::default()
        });
    }
    world.insert_resource(MaterialGeneration {
        catalog: Arc::new(catalog(&mut rng, shape, 1000 + seed)),
        ..Default::default()
    });
    if shape.world_generation {
        world.insert_resource(WorldGeneration(Some(3)));
    }
    world
}

/// Changes the world before frame `step` (1-based; frame 1 is the first build).
fn advance(world: &mut World, step: u32, rng: &mut Rng) {
    match step {
        // Same inputs: the lane is reused.
        2 => {}
        // New visibility id, same bytes: world and smodel picks held.
        3 => {
            if let Some(mut dpvs) = world.get_resource_mut::<DpvsFrameStats>() {
                dpvs.smodel_vis_id += 1;
            }
        }
        // The world list changed: world rebuilt, smodels held.
        4 => {
            let mut list = world.resource_mut::<DrawSurfList>();
            list.draw_items_id += 1;
            list.items.pop();
        }
        // Visibility bytes changed: smodels rebuilt, world held.
        5 => {
            if let Some(mut dpvs) = world.get_resource_mut::<DpvsFrameStats>() {
                dpvs.smodel_vis_id += 1;
                let n = dpvs.smodel_vis.len();
                for _ in 0..n.min(24) {
                    let i = rng.below(n as u64) as usize;
                    dpvs.smodel_vis[i] ^= 1;
                }
            }
        }
        // A new material generation: everything rebuilt.
        6 => {
            let mut runtime = world.resource_mut::<MaterialGeneration>();
            let mut next = (*runtime.catalog).clone();
            next.generation_id = MaterialGenerationId(next.generation_id.0 + 1);
            runtime.catalog = Arc::new(next);
        }
        // A new world generation: the old one sees an empty lane.
        7 => {
            world.insert_resource(WorldGeneration(Some(8)));
        }
        // The eye moved: LODs change.
        _ => {
            if let Some(mut view) = world.get_resource_mut::<PreparedSceneView>() {
                view.eye += Vec3::new(700.0, -350.0, 0.0);
            }
        }
    }
}

fn item_line(out: &mut String, item: &super::retained::RetainedDrawItem) {
    let _ = writeln!(out, "  {item:?}");
}

fn snapshot(lane: &StaticDrawLane, current: WorldGeneration) -> String {
    let mut out = String::new();
    let counts = lane.counts;
    let buckets = counts.buckets;
    let _ = writeln!(
        out,
        "generation {:?} world {:?}",
        lane.generation_id, lane.world_generation
    );
    let _ = writeln!(
        out,
        "census skip {} hold {} query {} buckets {} {} {} {} flush {} consume {}",
        u8::from(counts.reused),
        u8::from(counts.lod_held),
        counts.models_queried,
        buckets.rigid,
        buckets.skinned,
        buckets.cached,
        buckets.unread,
        buckets.flushes,
        buckets.consumed
    );
    let _ = writeln!(out, "membership {}", lane.membership_revision);
    let _ = writeln!(
        out,
        "layout {} pretess {:?}",
        lane.smodel_index_layout_revision, lane.smodel_pretess_indices
    );
    let _ = writeln!(out, "bucket counts {:?}", lane.smodel_surf_lists.count);
    for (bucket, ids) in lane.smodel_surf_lists.ids.iter().enumerate() {
        let _ = writeln!(out, "bucket {bucket} {ids:?}");
    }
    let _ = writeln!(out, "run surfs {:?}", lane.world_run_surfs);
    for (name, items) in [
        ("colour", &lane.colour),
        ("emissive", &lane.emissive),
        ("distortion", &lane.distortion),
    ] {
        let _ = writeln!(out, "{name} {}", items.len());
        for item in items {
            item_line(&mut out, item);
        }
    }
    let (live, colour, emissive, distortion, runs) = lane.live_for(current);
    let _ = writeln!(
        out,
        "live {} {} {} {} {}",
        live.len(),
        colour.len(),
        emissive.len(),
        distortion.len(),
        runs.len()
    );
    for item in live {
        item_line(&mut out, item);
    }
    let other = WorldGeneration(Some(current.0.map_or(1, |g| g + 1)));
    let _ = writeln!(out, "live other {}", lane.live_for(other).0.len());
    out
}

fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[derive(Default)]
struct Coverage {
    rigid: bool,
    skinned: bool,
    cached: bool,
    unread: bool,
    flush: bool,
    pretess_indices: bool,
    world_runs: bool,
    colour: bool,
    emissive: bool,
    distortion: bool,
    rebuild_skip: bool,
    lod_hold: bool,
    streams: [bool; 4],
    live_other_empty: bool,
}

impl Coverage {
    fn note(&mut self, lane: &StaticDrawLane, current: WorldGeneration) {
        let counts = lane.counts;
        self.rigid |= counts.buckets.rigid > 0;
        self.skinned |= counts.buckets.skinned > 0;
        self.cached |= counts.buckets.cached > 0;
        self.unread |= counts.buckets.unread > 0;
        self.flush |= counts.buckets.flushes > 0;
        self.pretess_indices |= !lane.smodel_pretess_indices.is_empty();
        self.world_runs |= !lane.world_run_surfs.is_empty();
        self.colour |= !lane.colour.is_empty();
        self.emissive |= !lane.emissive.is_empty();
        self.distortion |= !lane.distortion.is_empty();
        self.rebuild_skip |= counts.reused;
        self.lod_hold |= counts.lod_held;
        let live = lane.live_for(current).0;
        for item in live {
            if let RetainedDrawKind::Smodel {
                stream: Some(stream),
                ..
            } = item.kind
            {
                let at = match stream {
                    SmodelSurfPath::Rigid => 0,
                    SmodelSurfPath::Skinned => 1,
                    SmodelSurfPath::Cached => 2,
                    SmodelSurfPath::Pretess => 3,
                };
                self.streams[at] = true;
            }
        }
        let other = WorldGeneration(Some(current.0.map_or(1, |g| g + 1)));
        self.live_other_empty |= !live.is_empty() && lane.live_for(other).0.is_empty();
    }

    fn missing(&self) -> Vec<&'static str> {
        let checks = [
            (self.rigid, "rigid bucket"),
            (self.skinned, "skinned bucket"),
            (self.cached, "cached bucket"),
            (self.unread, "unread XSurface"),
            (self.flush, "full bucket flush"),
            (self.pretess_indices, "pretess indices"),
            (self.world_runs, "world runs"),
            (self.colour, "colour items"),
            (self.emissive, "emissive items"),
            (self.distortion, "distortion items"),
            (self.rebuild_skip, "rebuild skip"),
            (self.lod_hold, "LOD hold"),
            (self.streams[0], "rigid stream"),
            (self.streams[1], "skinned stream"),
            (self.streams[2], "cached stream"),
            (self.streams[3], "pretess stream"),
            (self.live_other_empty, "other world generation empty"),
        ];
        checks
            .iter()
            .filter(|(seen, _)| !seen)
            .map(|&(_, name)| name)
            .collect()
    }
}

/// Runs every seed's eight frames: `(seed, step, snapshot)` per frame.
fn run_all(coverage: &mut Coverage) -> Vec<(u64, u32, String)> {
    let mut frames = Vec::new();
    for seed in 0..SEEDS {
        let mut world = build_world(seed);
        let mut rng = Rng::new(0xabc0 + seed);
        for step in 1..=8 {
            if step > 1 {
                advance(&mut world, step, &mut rng);
            }
            world
                .run_system_once(rebuild_static_draw_lane)
                .expect("rebuild_static_draw_lane runs");
            let current = world
                .get_resource::<WorldGeneration>()
                .copied()
                .unwrap_or_default();
            let lane = world.resource::<StaticDrawLane>();
            coverage.note(lane, current);
            frames.push((seed, step, snapshot(lane, current)));
        }
    }
    frames
}

#[test]
fn retained_golden_static_lane() {
    let mut coverage = Coverage::default();
    let frames = run_all(&mut coverage);
    if let Ok(dir) = std::env::var("GOLDEN_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for (seed, step, text) in &frames {
            std::fs::write(format!("{dir}/s{seed}-{step}.txt"), text).unwrap();
        }
    }
    let lines: Vec<String> = frames
        .iter()
        .map(|(seed, step, text)| format!("{seed} {step} {:016x}", fnv1a(text)))
        .collect();
    if std::env::var("GOLDEN_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(GOLDEN, lines.join("\n") + "\n").unwrap();
    }
    let missing = coverage.missing();
    assert!(missing.is_empty(), "fixtures never reach: {missing:?}");
    let golden = std::fs::read_to_string(GOLDEN).expect("retained_golden.txt (bless with GOLDEN_BLESS=1)");
    let want: Vec<&str> = golden.lines().collect();
    let changed: Vec<String> = lines
        .iter()
        .zip(want.iter())
        .filter(|(got, want)| got.as_str() != **want)
        .map(|(got, want)| format!("want {want} got {got}"))
        .collect();
    assert_eq!(lines.len(), want.len(), "frame count changed");
    assert!(changed.is_empty(), "static lane changed:\n{}", changed.join("\n"));
}

#[test]
fn retained_golden_frames_are_deterministic() {
    let first = run_all(&mut Coverage::default());
    let second = run_all(&mut Coverage::default());
    for ((seed, step, a), (_, _, b)) in first.iter().zip(second.iter()) {
        assert!(a == b, "seed {seed} step {step} differs between runs");
    }
}


mod lanes;
mod sun;
