//! Golden tests for `refill_static_draw_lane`: seeded fixtures run through
//! eight frames each (full build, reuse, LOD hold, world-only and
//! smodel-only rebuilds, new material generation, new world generation,
//! eye moved), and every frame's lane output is hashed against
//! `retained_golden.txt`. A rewrite of the static lane has to keep every hash.
//!
//! `GOLDEN_BLESS=1` rewrites the golden file; `GOLDEN_DUMP=<dir>` writes each
//! frame's snapshot text as `s<seed>-<step>.txt` for diffing.

use std::fmt::Write as _;
use std::sync::Arc;

use asset_core::AssetCategoryScope;
use asset_model::{DetailParamsDirection, ModelDetailSelector};
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use dpvs_t6::{SortedDrawParts, pack};
use frame::LevelGeneration;
use lighting_t6::PropFacetPath;

use super::list::{FacetDrawItem, FacetDrawList};
use super::kept_lanes::{HeldDrawKind, StaticDrawTrack, refill_static_draw_lane};
use super::mesh_build::prop_mesh::{PropGpuPlan, PropMeshFacets, PropPassMaterial, PropPlacement};
use super::{
    MaterialAssetKey, MaterialEpoch, MaterialEpochId, MaterialRemapOutcome, LiveMaterial,
    LiveMaterialLedger, LiveSortedMaterialTable, LiveTechniqueBundleKey, SortedMaterialRank,
    SurfaceBakedLightId, SurfaceCubeProbeId, FacetSamplerInputs, WorldGpuSubmission,
};
use crate::prepare::scene::cull::SightFrameStats;
use crate::prepare::scene::prop_geom_memo::{
    DetailRampTunable, PrebuiltTunable, PropCacheEnableTunable, LevelPropCache,
};
use crate::prepare::scene::prop_light_slots::LevelPropIllum;
use crate::prepare::scene::eye_parms::StagedStageEye;
use crate::prepare::scene::world::LevelDrawItemKind;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/draw_items/retained_golden.txt"
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
    in_order: bool,
    level_plan: bool,
    prop_plan: bool,
    level_generation: bool,
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
        in_order: true,
        level_plan: true,
        prop_plan: true,
        level_generation: true,
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
            in_order: false,
            level_plan: false,
            prop_plan: false,
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
            level_generation: false,
            ..base
        },
        _ => Shape {
            smc: Some(true),
            all_cached: true,
            ..base
        },
    }
}

fn material(i: usize, rng: &mut Rng) -> LiveMaterial {
    let cull_mode = None;
    LiveMaterial {
        // One row whose id does not match its slot: lookups through it miss.
        asset_id: MaterialAssetKey(if i == 7 { 99 } else { i as u16 }),
        name: format!("m{i}"),
        // BO1 materials never reach a BO2 draw lane.
        namespace: AssetCategoryScope::T6,
        method_set: String::new(),
        baked_facet_draw: None,
        local_technique_set: LiveTechniqueBundleKey(0),
        remap: MaterialRemapOutcome::SelfSet,
        state_row_for: None,
        state_rows: Vec::new(),
        camera_zone: rng.below(5) as u8,
        draw_sort_key: rng.below(64) as u8,
        game_flag_info: rng.below(256) as u8,
        condition_bits: 0,
        facet_type_bits: None,
        unlit: false,
        uses_model_illum_const: false,
        colour_map_is_square: false,
        occluder_exclusive: false,
        t6_draw: None,
        cull_mode,
        uv_scroll_bits: [0; 4],
        attenuation_terms_raw: [0; 4],
        attenuation_start_raw: [0; 4],
        attenuation_stop_raw: [0; 4],
        reflection_map_param_bits: [0; 4],
        sheet_bits: [0; 5],
        textures: Vec::new(),
        constants: Vec::new(),
    }
}

fn catalog(rng: &mut Rng, shape: Shape, generation: u64) -> LiveMaterialLedger {
    let materials = (0..MATERIALS).map(|i| material(i, rng)).collect();
    let ranked_materials = if shape.in_order {
        let mut order: Vec<usize> = (0..MATERIALS).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i as u64 + 1) as usize);
        }
        let mut asset_nos_by_ord: Vec<MaterialAssetKey> =
            order.iter().map(|&i| MaterialAssetKey(i as u16)).collect();
        // Ordinals past the catalog's end.
        asset_nos_by_ord.push(MaterialAssetKey(200));
        asset_nos_by_ord.push(MaterialAssetKey(201));
        let mut ranks_by_asset_id = vec![None; MATERIALS];
        for (ord, &asset) in order.iter().enumerate() {
            ranks_by_asset_id[asset] = Some(SortedMaterialRank::new(ord as u32).unwrap());
        }
        LiveSortedMaterialTable::Ready {
            asset_ids_by_ordinal: asset_nos_by_ord,
            ranks_by_asset_id,
            skipped_total: 0,
            gap_total: 0,
        }
    } else {
        LiveSortedMaterialTable::default()
    };
    LiveMaterialLedger {
        revision_stamp: MaterialEpochId(generation),
        materials,
        ranked_materials,
        ..Default::default()
    }
}

fn world_key(rng: &mut Rng) -> u64 {
    let fields = SortedDrawParts {
        material_draw_rank_slot: rng.below(RANKS) as u16,
        leading_sort_bucket: rng.below(16) as u8,
        kind: rng.below(4) as u8,
        extra_ix: rng.below(8) as u16,
        ..Default::default()
    };
    pack(fields).packed
}

fn world_list(rng: &mut Rng, seed: u64) -> FacetDrawList {
    let kinds = [
        asset_world::CameraSpanKind::LitOpaque,
        asset_world::CameraSpanKind::LitTrans,
        asset_world::CameraSpanKind::Decal,
        asset_world::CameraSpanKind::Emissive,
    ];
    let mut list = FacetDrawList {
        draw_batch_list_id: 0x1000 + seed,
        ..Default::default()
    };
    for _ in 0..60 {
        let key = world_key(rng);
        if rng.chance(20) {
            list.items.push(FacetDrawItem {
                key,
                facet_ix: rng.below(WORLD_SURFS) as u16,
                span_run: 1,
                list_start: 0,
                entry_first: 0,
                kind: LevelDrawItemKind::BModel,
            });
            continue;
        }
        let batch_run = 1 + rng.below(6);
        let first = rng.below(WORLD_SURFS - batch_run);
        let list_start = list.surfs.len() as u32;
        let mut surf = first;
        for _ in 0..batch_run {
            list.surfs.push(surf as u16);
            surf += 1 + u64::from(rng.chance(15)) * rng.below(9);
            surf = surf.min(WORLD_SURFS - 1);
        }
        list.items.push(FacetDrawItem {
            key,
            facet_ix: first as u16,
            span_run: batch_run as u16,
            list_start,
            entry_first: rng.below(8) as u16,
            kind: LevelDrawItemKind::Bsp(kinds[rng.below(4) as usize]),
        });
    }
    list.keys = list.items.len() as u32;
    list
}

fn level_plan(rng: &mut Rng) -> WorldGpuSubmission {
    let mut blocks = Vec::new();
    for _ in 0..WORLD_SURFS / 8 {
        blocks.push(FacetSamplerInputs {
            reflection_probe: match rng.below(3) {
                0 => None,
                n => Some(SurfaceCubeProbeId(n as u8)),
            },
            primary_lightmap: rng
                .chance(70)
                .then(|| SurfaceBakedLightId(rng.below(3) as u8)),
            secondary_lightmap: None,
        });
    }
    let plan = WorldGpuSubmission {
        facet_sampler_inputs: (0..WORLD_SURFS as usize).map(|s| blocks[s / 8]).collect(),
        ..Default::default()
    };
    plan
}

fn authored(rng: &mut Rng) -> Option<assets::SurfaceMaterialId> {
    rng.chance(85)
        .then(|| assets::SurfaceMaterialId::from_sequence(rng.below(MATERIALS as u64) as usize))
}

/// Mesh kinds by `i % 4`: rigid, skinned, cache-capable rigid, and one whose
/// odd LODs carry an unread XSurface byte.
fn mesh(i: usize, rng: &mut Rng) -> PropMeshFacets {
    let kind = i % 4;
    let mut mesh = PropMeshFacets::default();
    for detail in 0..4 {
        let n = 1 + rng.below(3) as usize;
        let surfs: Vec<_> = (0..n)
            .map(|s| ((i * 16 + detail * 4 + s) as u32, authored(rng)))
            .collect();
        mesh.figure_part_plus_1_by_detail[detail] = (0..n)
            .map(|s| match kind {
                1 if s == 0 => Some(1 + rng.below(3) as u8),
                3 if detail % 2 == 1 && s == n - 1 => None,
                _ => Some(0),
            })
            .collect();
        mesh.facets_by_detail[detail] = surfs;
    }
    if kind == 2 {
        mesh.detail_prop_cache = Some([0, 1, 0, 0]);
        if i > 5 {
            mesh.detail_prop_cache_rows = Some([[0, 1, 0, 0], [0, 1, 0, 0], [0, 0, 0, 0], [0, 1, 0, 0]]);
        }
    }
    if i % 3 != 0 {
        mesh.lod = Some(ModelDetailSelector::T5 {
            detail_tier_total: if i % 5 == 0 { 3 } else { 4 },
            tier_reach: [300.0, 900.0, 2000.0, 5000.0],
        });
    }
    mesh
}

fn pass_material(i: usize, rng: &mut Rng) -> PropPassMaterial {
    let cull_mode = None;
    let blend = AlphaMode::Opaque;
    let atlas = None;
    let specular = None;
    let color = None;
    PropPassMaterial {
        model_illum_required: i % 2 == 0,
        color,
        specular,
        probe: None,
        atlas,
        see_through: blend,
        paint_mode: None,
        face_cull: cull_mode,
        env_map_params: [0.0; 4],
        illum_lookup_scale: [0.0; 4],
        atlas_find: [0.0; 4],
        order_key: rng.below(64) as u8,
        material_draw_rank_slot: (i % 6 != 5).then(|| rng.below(RANKS) as u32),
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

fn prop_plan(rng: &mut Rng, shape: Shape, n: usize) -> PropGpuPlan {
    let mut draw_plan = PropGpuPlan {
        materials: (0..PASS_MATERIALS).map(|i| pass_material(i, rng)).collect(),
        meshes: (0..MESHES).map(|i| mesh(i, rng)).collect(),
        ..Default::default()
    };
    let authored_all = std::iter::once(None)
        .chain((0..MATERIALS).map(|m| Some(assets::SurfaceMaterialId::from_sequence(m))));
    for authored in authored_all {
        for probe in 0..3u8 {
            if rng.chance(90) {
                // Mostly a lit pass material (even), sometimes a mismatch.
                let even = 2 * rng.below(PASS_MATERIALS as u64 / 2) as u32;
                let material = if rng.chance(90) { even } else { even + 1 };
                draw_plan.shaded_material_key.insert((authored, probe), material);
            }
        }
        if rng.chance(90) {
            let odd = 2 * rng.below(PASS_MATERIALS as u64 / 2) as u32 + 1;
            let material = if rng.chance(90) { odd } else { odd - 1 };
            draw_plan.flat_material_key.insert(authored, material);
        }
    }
    for i in 0..n {
        let pos = [
            rng.span(-3000.0, 3000.0),
            rng.span(-3000.0, 3000.0),
            rng.span(-200.0, 400.0),
        ];
        let illum_slot = match rng.below(100) {
            _ if shape.all_cached => Some(i),
            0..85 => Some(i),
            85..95 => None,
            _ => Some(70_000),
        };
        let compact_illum = match rng.below(10) {
            _ if shape.all_cached => Some([1 + rng.below(255) as u8, 2, 3, 4]),
            0..4 => Some([rng.below(256) as u8, rng.below(256) as u8, 7, 1]),
            4 => Some([0; 4]),
            _ => None,
        };
        draw_plan.placements.push(PropPlacement {
            model_slot: placement_mesh(rng, shape),
            lit_list: shape.all_cached || rng.chance(60),
            env_probe_index: rng.below(3) as u8,
            key_light_index: rng.below(4) as u8,
            mode_bits: 0,
            illum_slot,
            compact_illum,
            ent: Entity::PLACEHOLDER,
            local_to_world: Mat4::from_translation(Vec3::from_array(pos)),
            at_pos: pos,
            axes_set: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            size_k: rng.span(0.5, 2.0),
            t6_primary: [0; 2],
            t6_light: 0,
            t6_cull_dist: 0.0,
        });
    }
    draw_plan
}

fn light_data(rng: &mut Rng, n: usize) -> LevelPropIllum {
    let dims = lighting_t6::illum_atlas_dims(1).unwrap();
    let mut lighting = LevelPropIllum::new(dims, n, &[]);
    lighting.handle_list = (0..n)
        .map(|_| {
            if rng.chance(80) {
                1 + rng.below(500) as u16
            } else {
                0
            }
        })
        .collect();
    lighting.compact_illum = (0..n)
        .map(|_| rng.chance(50).then(|| [rng.below(256) as u8, 9, 9, 1]))
        .collect();
    lighting
}

fn cache(rng: &mut Rng, plan: &PropGpuPlan, shape: Shape, n: usize) -> LevelPropCache {
    let mut cache = LevelPropCache::new(n);
    for row in cache.cache_slot_no.iter_mut() {
        for index in row.iter_mut() {
            if shape.all_cached || rng.chance(70) {
                *index = 1 + rng.below(0x1fff) as u16;
            }
        }
    }
    for placement in &plan.placements {
        let Some(slot) = placement.illum_slot.filter(|&slot| slot < n) else {
            continue;
        };
        for &(facet, _) in plan.meshes[placement.model_slot].facets_by_detail.iter().flatten() {
            let roll = rng.below(100);
            if shape.all_cached || roll < 85 {
                let triangle_list = 1 + rng.below(4) as usize;
                let run = (0..triangle_list * 3).map(|_| rng.below(0x1000) as u16).collect();
                cache.index_stretches.insert((slot as u32, facet), run);
            } else if roll < 90 {
                cache.index_stretches.insert((slot as u32, facet), vec![1, 2, 3, 4]);
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
    world.insert_resource(StaticDrawTrack::default());
    world.insert_resource(world_list(&mut rng, seed));
    if shape.level_plan {
        world.insert_resource(level_plan(&mut rng));
    }
    let plan = prop_plan(&mut rng, shape, n);
    if shape.lighting {
        world.insert_resource(light_data(&mut rng, n));
    }
    if shape.cache {
        world.insert_resource(cache(&mut rng, &plan, shape, n));
    }
    if shape.dpvs {
        world.insert_resource(SightFrameStats {
            prop_vis: if shape.vis {
                (0..n).map(|_| u8::from(rng.chance(80))).collect()
            } else {
                Vec::new()
            },
            prop_vis_id: 0x50 + seed,
            ..Default::default()
        });
    }
    if shape.prop_plan {
        world.insert_resource(plan);
    }
    world.insert_resource(PropCacheEnableTunable { enabled: shape.smc });
    world.insert_resource(PrebuiltTunable {
        enabled: shape.pretess,
    });
    world.insert_resource(DetailRampTunable {
        t5: DetailParamsDirection {
            used_reciprocal_factor: 1.0,
            scale: 1.0,
            bias: 0.0,
        },
        ..Default::default()
    });
    if shape.eye != Eye::Missing {
        world.insert_resource(StagedStageEye {
            primed: shape.eye == Eye::Ready,
            eye: eye_at(&mut rng),
            ..Default::default()
        });
    }
    world.insert_resource(MaterialEpoch {
        catalog: Arc::new(catalog(&mut rng, shape, 1000 + seed)),
        ..Default::default()
    });
    if shape.level_generation {
        world.insert_resource(LevelGeneration(Some(3)));
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
            if let Some(mut dpvs) = world.get_resource_mut::<SightFrameStats>() {
                dpvs.prop_vis_id += 1;
            }
        }
        // The world list changed: world rebuilt, fixed_props held.
        4 => {
            let mut list = world.resource_mut::<FacetDrawList>();
            list.draw_batch_list_id += 1;
            list.items.pop();
        }
        // Visibility bytes changed: fixed_props rebuilt, world held.
        5 => {
            if let Some(mut dpvs) = world.get_resource_mut::<SightFrameStats>() {
                dpvs.prop_vis_id += 1;
                let n = dpvs.prop_vis.len();
                for _ in 0..n.min(24) {
                    let i = rng.below(n as u64) as usize;
                    dpvs.prop_vis[i] ^= 1;
                }
            }
        }
        // A new material generation: everything rebuilt.
        6 => {
            let mut runtime = world.resource_mut::<MaterialEpoch>();
            let mut next = (*runtime.catalog).clone();
            next.revision_stamp = MaterialEpochId(next.revision_stamp.0 + 1);
            runtime.catalog = Arc::new(next);
        }
        // A new world generation: the old one sees an empty lane.
        7 => {
            world.insert_resource(LevelGeneration(Some(8)));
        }
        // The eye moved: LODs change.
        _ => {
            if let Some(mut view) = world.get_resource_mut::<StagedStageEye>() {
                view.eye += Vec3::new(700.0, -350.0, 0.0);
            }
        }
    }
}

fn item_line(out: &mut String, item: &super::kept_lanes::HeldDrawItem) {
    out.push_str(&format!("  {item:?}
"));
}

fn snapshot(lane: &StaticDrawTrack, current: LevelGeneration) -> String {
    let mut out = String::new();
    let counts = lane.counts;
    let buckets = counts.buckets;
    let _ = writeln!(
        out,
        "generation {:?} world {:?}",
        lane.revision_stamp, lane.level_generation
    );
    let _ = writeln!(
        out,
        "census skip {} hold {} query {} buckets {} {} {} {} flush {} consume {}",
        u8::from(counts.kept_over),
        u8::from(counts.lod_held),
        counts.models_queried,
        buckets.rigid,
        buckets.skinned,
        buckets.cached,
        buckets.unread,
        buckets.flushes,
        buckets.consumed
    );
    out.push_str(&format!("membership {}
", lane.membership_edition));
    let _ = writeln!(
        out,
        "layout {} pretess {:?}",
        lane.prop_index_layout_edition, lane.prop_prebuilt_indices
    );
    out.push_str(&format!("bucket counts {:?}
", lane.smodel_surf_lists.filled));
    for (bin, ids) in lane.smodel_surf_lists.members.iter().enumerate() {
        out.push_str(&format!("bucket {bin} {ids:?}
"));
    }
    out.push_str(&format!("run surfs {:?}
", lane.level_run_facets));
    for (name, items) in [
        ("colour", &lane.tint_rgb),
        ("emissive", &lane.glow),
        ("distortion", &lane.warp_pass),
    ] {
        out.push_str(&format!("{name} {}
", items.len()));
        for draw_entry in items {
            item_line(&mut out, draw_entry);
        }
    }
    let (live, colour, emissive, warp_pass, runs) = lane.alive_for(current);
    let _ = writeln!(
        out,
        "live {} {} {} {} {}",
        live.len(),
        colour.len(),
        emissive.len(),
        warp_pass.len(),
        runs.len()
    );
    for item in live {
        item_line(&mut out, item);
    }
    let other = LevelGeneration(Some(current.0.map_or(1, |g| g + 1)));
    out.push_str(&format!("live other {}
", lane.alive_for(other).0.len()));
    out
}

fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for octet in text.bytes() {
        hash ^= u64::from(octet);
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
    fn tally(&mut self, lane: &StaticDrawTrack, current: LevelGeneration) {
        let counts = lane.counts;
        self.rigid |= counts.buckets.rigid > 0;
        self.skinned |= counts.buckets.skinned > 0;
        self.cached |= counts.buckets.cached > 0;
        self.unread |= counts.buckets.unread > 0;
        self.flush |= counts.buckets.flushes > 0;
        self.pretess_indices |= !lane.prop_prebuilt_indices.is_empty();
        self.world_runs |= !lane.level_run_facets.is_empty();
        self.colour |= !lane.tint_rgb.is_empty();
        self.emissive |= !lane.glow.is_empty();
        self.distortion |= !lane.warp_pass.is_empty();
        self.rebuild_skip |= counts.kept_over;
        self.lod_hold |= counts.lod_held;
        let live = lane.alive_for(current).0;
        for draw_entry in live {
            if let HeldDrawKind::Smodel {
                stream: Some(path),
                ..
            } = draw_entry.kind
            {
                let at = match path {
                    PropFacetPath::Rigid => 0,
                    PropFacetPath::Skinned => 1,
                    PropFacetPath::Cached => 2,
                    PropFacetPath::Pretess => 3,
                };
                self.streams[at] = true;
            }
        }
        let other = LevelGeneration(Some(current.0.map_or(1, |g| g + 1)));
        self.live_other_empty |= !live.is_empty() && lane.alive_for(other).0.is_empty();
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
            .filter(|(visited, _)| !visited)
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
                .run_system_once(refill_static_draw_lane)
                .expect("refill_static_draw_lane runs");
            let current = world
                .get_resource::<LevelGeneration>()
                .copied()
                .unwrap_or_default();
            let lane = world.resource::<StaticDrawTrack>();
            coverage.tally(lane, current);
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


mod lane_set;
mod daylight;
