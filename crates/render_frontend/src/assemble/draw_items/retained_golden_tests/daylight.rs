//! Golden test for the sun-shadow caster bake: seeded fixtures, one per seed, baked for
//! both partitions and merged with the dynamic models the way a frame does it, then
//! hashed against `sun_golden.txt`. A rewrite of the bake has to keep every hash.
//!
//! Lives under `retained_golden_tests` so it can reuse that file's fixture builders.
//! `GOLDEN_BLESS=1` rewrites the golden file; `GOLDEN_DUMP=<dir>` writes each seed's
//! three texts as `sun<seed>-<near|far|merged>.txt` for diffing.

use std::fmt::Write as _;

use super::*;

use crate::assemble::draw_items::kept_lanes::{
    SunBakeInputs, SunScratch, DaylightShadowCasterPlan, SunVisibility, bake_partition,
    fill_prop_draw_inst_shadow, merge_partitions,
};
use crate::assemble::draw_items::mesh_build::prop_mesh::DetailRampArgs;
use crate::assemble::draw_items::mesh_build::figure_mesh::FigureDrawPlan;
use crate::prepare::scene::world::LevelHide;
use crate::prepare::scene::world::LevelSight;
use render_frame::HeldDrawItem;
use render_material::{LiveTechniqueBundle, RENDER_RECIPE_SLOT_TOTAL};
use render_scene::{XModelCasterBound, FigureFacetDraw};

const SUN_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/draw_items/sun_golden.txt"
);
const SUN_SEEDS: u64 = 8;
const XMODEL_MATERIALS: usize = 8;
const XMODEL_DRAWS: usize = 24;

struct ScriptedSetup {
    cull: LevelHide,
    level_plan: WorldGpuSubmission,
    prop_plan: PropGpuPlan,
    catalog: LiveMaterialLedger,
    xmodel: Option<FigureDrawPlan>,
    cache: Option<LevelPropCache>,
    lighting: Option<LevelPropIllum>,
    pretess: bool,
    eye_pos: Option<Vec3>,
    ramp: DetailRampArgs,
    facet_vis: [Vec<u8>; 2],
    prop_vis: [Vec<u8>; 2],
    planes: [Vec<[f32; 4]>; 2],
}

fn vis_bytes(rng: &mut Rng, len: usize, pct: u64) -> Vec<u8> {
    (0..len).map(|_| u8::from(rng.chance(pct))).collect()
}

/// Which surfaces the BSP part of the bake looks at, by seed: all four ways the range
/// can be chosen (emissive end, lit-opaque end, whole list, an end past the list).
fn world_dpvs(seed: u64) -> LevelSight {
    let (begin, lit_end, emissive_end) = match seed % 4 {
        0 => (10, 250, 330),
        1 => (0, 300, 0),
        2 => (20, 0, 0),
        _ => (5, 100, 900),
    };
    LevelSight {
        plane_set: Vec::new(),
        node_list: Vec::new(),
        cells_n: 0,
        box_trees: Vec::new(),
        aabb_prop_indices: Vec::new(),
        sorted_facet_index: Vec::new(),
        static_facet_tally: 0,
        static_facet_tally_no_stain: 0,
        facet_bounds: Vec::new(),
        prop_bounds: Vec::new(),
        portal_vertices: Vec::new(),
        portals_per_room: Vec::new(),
        room_env_probes: Vec::new(),
        room_caster_bits: Vec::new(),
        lit_solid_begin: begin,
        lit_solid_end: lit_end,
        camera_spans: asset_world::ViewSurfaceSpans::new(
            asset_world::ViewSurfaceSpan {
                kind: asset_world::CameraSpanKind::LitOpaque,
                from: begin,
                until: lit_end,
            },
            Vec::new(),
        ),
        emissive_facets_begin: 0,
        emissive_facets_end: emissive_end,
    }
}

fn world_cull(rng: &mut Rng, seed: u64, hide_dists: Vec<u16>) -> LevelHide {
    let surfs = WORLD_SURFS as usize;
    let mut capture = asset_world::LevelSnapshot::default();
    capture.casters = asset_world::FaceBlocksSunlight::with_len(surfs);
    for surf in 0..surfs {
        if rng.chance(70) {
            capture.casters.set(surf);
        }
    }
    // Seeds 2 and 5 have no packed keys, so the bake walks the surfaces itself.
    if !matches!(seed, 2 | 5) {
        capture.compact_facet_draws = (0..surfs)
            .map(|_| {
                let key = if rng.chance(80) { world_key(rng) } else { 0 };
                dpvs_t6::SortedDraw::unpack_from_bits(key)
            })
            .collect();
    }
    let model = |start_facet, facet_tally| asset_world::SolidModelFaces {
        start_facet,
        facet_tally,
        plain_face_total: facet_tally,
    };
    let mut solid_models = vec![asset_world::SolidModelFaces::default()];
    if seed != 4 {
        solid_models.push(model(300, 12));
        solid_models.push(model(360, 30));
    }
    let mut poses = vec![
        Mat4::IDENTITY,
        Mat4::from_translation(Vec3::new(120.0, -40.0, 8.0)),
        Mat4::from_translation(Vec3::new(-60.0, 300.0, 0.0)) * Mat4::from_rotation_z(0.7),
    ];
    // Seed 7 has one pose too few for its brush models.
    if seed == 7 {
        poses.pop();
    }
    LevelHide {
        compact_indices: Vec::new(),
        facet_index_ranges: Vec::new(),
        facet_draw_fields: Vec::new(),
        facet_batch_ranges: Vec::new(),
        facet_materials: (0..surfs).map(|_| authored(rng)).collect(),
        facet_lightbake_indices: Vec::new(),
        facet_env_probes: Vec::new(),
        facet_key_lights: (0..surfs).map(|_| rng.below(4) as u8).collect(),
        warp_sort_key: None,
        facet_sort_keys: Vec::new(),
        capture,
        solid_models,
        solid_model_bounds: Vec::new(),
        solid_model_level_from_local: poses,
        t6_brush_surfaces: Vec::new(),
        dpvs: world_dpvs(seed),
        batches_n: 0,
        batch_lightbaked: Vec::new(),
        facet_vis: Vec::new(),
        draw_batch_list: Vec::new(),
        item_surfs: Vec::new(),
        level_tree_run_scratch: Vec::new(),
        draw_batch_list_id: 0,
        g0_facets: Vec::new(),
        prop_entities: Vec::new(),
        prop_hide_dists: hide_dists,
        prop_vis: Vec::new(),
        room_vis: Vec::new(),
        room_vis_all: false,
    }
}

/// The parent's catalog, given what the sun bake reads: baked keys, a few cut-out names,
/// and technique sets that pass or fail the caster check.
fn sun_catalog(rng: &mut Rng, shape: Shape, generation: u64) -> LiveMaterialLedger {
    let mut base = catalog(rng, shape, generation);
    for (i, row) in base.materials.iter_mut().enumerate() {
        row.baked_facet_draw = rng.chance(80).then(|| world_key(rng));
        if rng.chance(15) {
            row.name = format!("fence_{i}");
        }
        // Set 0 exists and has no caster technique; set 1 is past the end and passes.
        row.local_technique_set = if rng.chance(30) {
            LiveTechniqueBundleKey(0)
        } else {
            LiveTechniqueBundleKey(1)
        };
        row.remap = if rng.chance(10) {
            MaterialRemapOutcome::Resolved(LiveTechniqueBundleKey(0))
        } else {
            MaterialRemapOutcome::SelfSet
        };
    }
    let slots = (0..RENDER_RECIPE_SLOT_TOTAL).map(|_| None).collect();
    base.technique_sets = vec![LiveTechniqueBundle::new(slots).unwrap()];
    base
}

fn sun_smodels(rng: &mut Rng, shape: Shape, n: usize) -> PropGpuPlan {
    let mut plan = prop_plan(rng, shape, n);
    for placement in &mut plan.placements {
        placement.key_light_index = if rng.chance(85) { 1 } else { rng.below(4) as u8 };
        placement.mode_bits = match rng.below(10) {
            6 => 0x1,
            7 => 0x2,
            8 => 0x3,
            _ => 0,
        };
    }
    // Vis slots map onto placements through a permutation, with holes and strays.
    plan.designer_placement_indices = (0..n)
        .map(|slot| match rng.below(100) {
            0..85 => Some((slot * 11 + 3) % n),
            85..93 => None,
            _ => Some(n + 5),
        })
        .collect();
    plan
}

fn xmodel_draw(rng: &mut Rng, i: usize) -> FigureFacetDraw {
    let bound = match rng.below(3) {
        0 => None,
        1 => Some(XModelCasterBound {
            origin: [rng.span(-800.0, 800.0), rng.span(-800.0, 800.0), rng.span(0.0, 200.0)],
            reach: rng.span(20.0, 200.0),
        }),
        _ => Some(XModelCasterBound {
            origin: [rng.span(-4000.0, 4000.0), rng.span(-4000.0, 4000.0), 0.0],
            reach: rng.span(10.0, 150.0),
        }),
    };
    let colour_refusal = None;
    FigureFacetDraw {
        surface: rng.below(100) as u32,
        // Two draws in ten name a material past the plan's list.
        material: rng.below(XMODEL_MATERIALS as u64 + 2) as u32,
        map_from_self: Mat4::from_translation(eye_at(rng)),
        illum_handle: rng.below(300) as u32,
        queued_illum: None,
        colour_refusal,
        object_tag: if i == 3 { 1 } else { 2 + i as u16 * 13 + rng.below(5) as u16 },
        stage_light_index: rng.below(4) as u8,
        env_probe_index: rng.below(3) as u8,
        compact_illum: rng.chance(50).then(|| [rng.below(256) as u8, 1, 2, 3]),
        is_scoped_view: i == 5 || rng.chance(5),
        stage_actor_num: rng.chance(60).then(|| rng.below(1000) as u32),
        caster_bound: bound,
    }
}

fn sun_xmodels(rng: &mut Rng) -> FigureDrawPlan {
    // The plan keeps private bookkeeping, so it is filled in rather than built field by field.
    let mut plan = FigureDrawPlan::default();
    plan.materials = (0..XMODEL_MATERIALS).map(|i| pass_material(i, rng)).collect();
    plan.draws = (0..XMODEL_DRAWS).map(|i| xmodel_draw(rng, i)).collect();
    plan
}

fn fixture(seed: u64) -> ScriptedSetup {
    let shape = shape(seed);
    let mut rng = Rng::new(0x5100 + seed);
    let n = if shape.dense_skinned { 420 } else { 240 };
    let cat = sun_catalog(&mut rng, shape, 2000 + seed);
    let mut prop_plan = sun_smodels(&mut rng, shape, n);
    // Seed 3's cull distances stop halfway: the rest read as zero.
    let dist_len = if seed == 3 { n / 2 } else { n };
    let hide_dists: Vec<u16> = (0..dist_len)
        .map(|_| if rng.chance(50) { 0 } else { 500 + rng.below(5500) as u16 })
        .collect();
    let mut insts = Vec::new();
    fill_prop_draw_inst_shadow(&mut insts, &hide_dists, &prop_plan);
    prop_plan.shadow_draw_instances = insts;
    let surfs = WORLD_SURFS as usize;
    // Seed 5 has visibility lists shorter than what they describe.
    let cut = |len: usize, by: usize| if seed == 5 { len - by } else { len };
    let facet_vis = [
        vis_bytes(&mut rng, cut(surfs, 60), 75),
        vis_bytes(&mut rng, cut(surfs, 60), 50),
    ];
    let prop_vis = [
        vis_bytes(&mut rng, cut(n, 10), 75),
        vis_bytes(&mut rng, cut(n, 10), 50),
    ];
    let cull = world_cull(&mut rng, seed, hide_dists);
    let level_plan = if seed == 3 { WorldGpuSubmission::default() } else { level_plan(&mut rng) };
    let cache = shape.cache.then(|| cache(&mut rng, &prop_plan, shape, n));
    let lighting = shape.lighting.then(|| light_data(&mut rng, n));
    let eye = (shape.eye != Eye::Missing).then(|| eye_at(&mut rng));
    let late = seed % 2 == 1;
    let ramp = DetailRampTunable {
        t5: DetailParamsDirection {
            used_reciprocal_factor: 1.0,
            scale: if late { 1.3 } else { 1.0 },
            bias: if late { 40.0 } else { 0.0 },
        },
        ..Default::default()
    }
    .args();
    let near = vec![
        [1.0, 0.0, 0.0, 1500.0],
        [-1.0, 0.0, 0.0, 1500.0],
        [0.0, 1.0, 0.0, 1500.0],
        [0.0, -1.0, 0.0, 1500.0],
    ];
    // Odd seeds give the far partition a wider box, even seeds none at all.
    let far = if late {
        vec![
            [1.0, 0.0, 0.0, 2500.0],
            [-1.0, 0.0, 0.0, 3500.0],
            [0.0, 1.0, 0.0, 3000.0],
            [0.0, -1.0, 0.0, 3000.0],
        ]
    } else {
        Vec::new()
    };
    ScriptedSetup {
        cull,
        level_plan,
        prop_plan,
        catalog: cat,
        xmodel: (seed != 1 && seed != 6).then(|| sun_xmodels(&mut rng)),
        cache,
        lighting,
        pretess: shape.pretess,
        eye_pos: eye,
        ramp,
        facet_vis,
        prop_vis,
        planes: [near, far],
    }
}

fn bake(fix: &ScriptedSetup, partition: usize) -> DaylightShadowCasterPlan {
    let inputs = SunBakeInputs {
        cull: &fix.cull,
        level_plan: &fix.level_plan,
        prop_plan: &fix.prop_plan,
        catalog: &fix.catalog,
        cache: fix.cache.as_ref(),
        lighting: fix.lighting.as_ref(),
        pretess_on: fix.pretess,
        eye: fix.eye_pos,
        ramp: fix.ramp,
    };
    let vis = SunVisibility {
        surfaces: &fix.facet_vis[partition],
        fixed_props: &fix.prop_vis[partition],
    };
    bake_partition(inputs, vis, &mut SunScratch::default())
}

fn partition_text(plan: &DaylightShadowCasterPlan) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "generation {:?}", plan.revision_stamp);
    let _ = writeln!(
        text,
        "world eligible {} missing_key {}",
        plan.level_eligible, plan.level_missing_key
    );
    let _ = writeln!(
        text,
        "smodel eligible {} missing_key {}",
        plan.prop_eligible, plan.prop_missing_key
    );
    let _ = writeln!(text, "pretess {:?}", plan.prop_prebuilt_indices);
    let _ = writeln!(text, "items {}", plan.item_list.len());
    for item in &plan.item_list {
        let _ = writeln!(text, "  {item:?}");
    }
    text
}

/// The merged list a frame hands the sun pass: near partition first, then far.
fn merged_text(merged: &DaylightShadowCasterPlan) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "near {} of {}", merged.daylight_near_tally, merged.item_list.len());
    let _ = writeln!(
        text,
        "world eligible {} missing_key {}",
        merged.level_eligible, merged.level_missing_key
    );
    let _ = writeln!(
        text,
        "smodel eligible {} missing_key {}",
        merged.prop_eligible, merged.prop_missing_key
    );
    let _ = writeln!(text, "pretess {:?}", merged.prop_prebuilt_indices);
    for item in &merged.item_list {
        let _ = writeln!(text, "  {item:?}");
    }
    text
}

#[derive(Default)]
struct SunCoverage {
    world: bool,
    world_posed: bool,
    smodel: bool,
    streams: [bool; 4],
    xmodel: bool,
    pretess_indices: bool,
    level_missing_key: bool,
    prop_missing_key: bool,
    near_differs_from_far: bool,
    eye_missing: bool,
    eye_present: bool,
}

impl SunCoverage {
    fn record(&mut self, fix: &ScriptedSetup, items: &[HeldDrawItem], near: &str, far: &str) {
        self.near_differs_from_far |= near != far;
        self.eye_missing |= fix.eye_pos.is_none();
        self.eye_present |= fix.eye_pos.is_some();
        for draw_entry in items {
            match draw_entry.kind {
                HeldDrawKind::World { model_to_world: local_to_world, .. } => {
                    self.world = true;
                    self.world_posed |= local_to_world != Mat4::IDENTITY;
                }
                HeldDrawKind::Smodel { stream, .. } => {
                    self.smodel = true;
                    let at = match stream {
                        Some(PropFacetPath::Rigid) => Some(0),
                        Some(PropFacetPath::Skinned) => Some(1),
                        Some(PropFacetPath::Cached) => Some(2),
                        Some(PropFacetPath::Pretess) => Some(3),
                        None => None,
                    };
                    if let Some(at_pos) = at {
                        self.streams[at_pos] = true;
                    }
                }
                HeldDrawKind::XModel { .. } => self.xmodel = true,
                _ => {}
            }
        }
    }

    /// The sun bake files every static model with the static-model-cache switch off, so
    /// the bucket search can only pick the rigid or skinned stream. The cached and pretess
    /// streams, and the pretess index list, are out of its reach; the golden pins that.
    fn unexpected(&self) -> Vec<&'static str> {
        let reached = [
            (self.streams[2], "cached stream"),
            (self.streams[3], "pretess stream"),
            (self.pretess_indices, "pretess indices"),
        ];
        reached
            .iter()
            .filter(|(visited, _)| *visited)
            .map(|&(_, name)| name)
            .collect()
    }

    fn missing(&self) -> Vec<&'static str> {
        let checks = [
            (self.world, "world items"),
            (self.world_posed, "world item with a brush-model pose"),
            (self.smodel, "static model items"),
            (self.streams[0], "rigid stream"),
            (self.streams[1], "skinned stream"),
            (self.xmodel, "dynamic model items"),
            (self.level_missing_key, "world surface without a key"),
            (self.prop_missing_key, "static model without a key"),
            (self.near_differs_from_far, "partitions that differ"),
            (self.eye_missing, "bake without an eye"),
            (self.eye_present, "bake with an eye"),
        ];
        checks
            .iter()
            .filter(|(visited, _)| !visited)
            .map(|&(_, name)| name)
            .collect()
    }
}

/// `(seed, near text, far text, merged text)` for every seed.
fn sun_frames(coverage: &mut SunCoverage) -> Vec<(u64, String, String, String)> {
    let mut frames = Vec::new();
    for seed in 0..SUN_SEEDS {
        let fix = fixture(seed);
        let near = bake(&fix, 0);
        let far = bake(&fix, 1);
        let near_text = partition_text(&near);
        let far_text = partition_text(&far);
        coverage.pretess_indices |=
            !near.prop_prebuilt_indices.is_empty() || !far.prop_prebuilt_indices.is_empty();
        coverage.level_missing_key |= near.level_missing_key + far.level_missing_key > 0;
        coverage.prop_missing_key |= near.prop_missing_key + far.prop_missing_key > 0;
        let merged_plan = merge_partitions(
            near,
            far,
            fix.xmodel.as_ref(),
            &fix.catalog,
            [fix.planes[0].as_slice(), fix.planes[1].as_slice()],
        );
        coverage.record(&fix, &merged_plan.item_list, &near_text, &far_text);
        let merged = merged_text(&merged_plan);
        frames.push((seed, near_text, far_text, merged));
    }
    frames
}

#[test]
fn sun_golden_casters() {
    let mut coverage = SunCoverage::default();
    let frames = sun_frames(&mut coverage);
    if let Ok(dir) = std::env::var("GOLDEN_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for (seed, near, far, merged) in &frames {
            for (name, text) in [("near", near), ("far", far), ("merged", merged)] {
                std::fs::write(format!("{dir}/sun{seed}-{name}.txt"), text).unwrap();
            }
        }
    }
    let lines: Vec<String> = frames
        .iter()
        .flat_map(|(seed, near, far, merged)| {
            [("near", near), ("far", far), ("merged", merged)]
                .map(|(part, text)| format!("{seed} {part} {:016x}", fnv1a(text)))
        })
        .collect();
    if std::env::var("GOLDEN_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(SUN_GOLDEN, lines.join("\n") + "\n").unwrap();
    }
    let missing = coverage.missing();
    assert!(missing.is_empty(), "fixtures never reach: {missing:?}");
    let unexpected = coverage.unexpected();
    assert!(unexpected.is_empty(), "sun bake now reaches: {unexpected:?}");
    let golden =
        std::fs::read_to_string(SUN_GOLDEN).expect("sun_golden.txt (bless with GOLDEN_BLESS=1)");
    let want: Vec<&str> = golden.lines().collect();
    let changed: Vec<String> = lines
        .iter()
        .zip(want.iter())
        .filter(|(got, want)| got.as_str() != **want)
        .map(|(got, want)| format!("want {want} got {got}"))
        .collect();
    assert_eq!(lines.len(), want.len(), "line count changed");
    assert!(changed.is_empty(), "sun casters changed:\n{}", changed.join("\n"));
}

#[test]
fn sun_golden_is_deterministic() {
    let first = sun_frames(&mut SunCoverage::default());
    let second = sun_frames(&mut SunCoverage::default());
    for (a, b) in first.iter().zip(second.iter()) {
        assert!(a == b, "seed {} differs between runs", a.0);
    }
}
