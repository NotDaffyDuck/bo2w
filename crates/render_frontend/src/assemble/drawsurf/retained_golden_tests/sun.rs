//! Golden test for the sun-shadow caster bake: seeded fixtures, one per seed, baked for
//! both partitions and merged with the dynamic models the way a frame does it, then
//! hashed against `sun_golden.txt`. A rewrite of the bake has to keep every hash.
//!
//! Lives under `retained_golden_tests` so it can reuse that file's fixture builders.
//! `GOLDEN_BLESS=1` rewrites the golden file; `GOLDEN_DUMP=<dir>` writes each seed's
//! three texts as `sun<seed>-<near|far|merged>.txt` for diffing.

use std::fmt::Write as _;

use super::*;

use crate::assemble::drawsurf::retained::{
    SunBakeInputs, SunScratch, SunShadowCasterPlan, SunVisibility, bake_partition,
    fill_smodel_draw_inst_shadow, merge_partitions,
};
use crate::assemble::drawsurf::tess::smodel::LodRampArgs;
use crate::assemble::drawsurf::tess::xmodel::XModelDrawPlan;
use crate::prepare::scene::world::{WorldCull, WorldDpvs};
use render_frame::RetainedDrawItem;
use render_material::{RuntimeTechniqueSet, TECHNIQUE_SLOT_COUNT};
use render_scene::{XModelCasterBound, XModelSurfaceDraw};

const SUN_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/drawsurf/sun_golden.txt"
);
const SUN_SEEDS: u64 = 8;
const XMODEL_MATERIALS: usize = 8;
const XMODEL_DRAWS: usize = 24;

struct Fixture {
    cull: WorldCull,
    world_plan: WorldDrawGpuPlan,
    smodel_plan: SmodelGpuPlan,
    catalog: RuntimeMaterialCatalog,
    xmodel: Option<XModelDrawPlan>,
    cache: Option<WorldStaticModelCache>,
    lighting: Option<WorldSmodelLighting>,
    pretess: bool,
    eye: Option<Vec3>,
    ramp: LodRampArgs,
    surface_vis: [Vec<u8>; 2],
    smodel_vis: [Vec<u8>; 2],
    planes: [Vec<[f32; 4]>; 2],
}

fn vis_bytes(rng: &mut Rng, len: usize, pct: u64) -> Vec<u8> {
    (0..len).map(|_| u8::from(rng.chance(pct))).collect()
}

/// Which surfaces the BSP part of the bake looks at, by seed: all four ways the range
/// can be chosen (emissive end, lit-opaque end, whole list, an end past the list).
fn world_dpvs(seed: u64) -> WorldDpvs {
    let (begin, lit_end, emissive_end) = match seed % 4 {
        0 => (10, 250, 330),
        1 => (0, 300, 0),
        2 => (20, 0, 0),
        _ => (5, 100, 900),
    };
    WorldDpvs {
        planes: Vec::new(),
        nodes: Vec::new(),
        cell_count: 0,
        aabb_trees: Vec::new(),
        aabb_smodel_indices: Vec::new(),
        sorted_surf_index: Vec::new(),
        static_surface_count: 0,
        static_surface_count_no_decal: 0,
        surface_bounds: Vec::new(),
        smodel_bounds: Vec::new(),
        portal_verts: Vec::new(),
        portals_per_cell: Vec::new(),
        cell_reflection_probes: Vec::new(),
        cell_caster_bits: Vec::new(),
        lit_opaque_begin: begin,
        lit_opaque_end: lit_end,
        camera_ranges: asset_world::CameraSurfRanges::new(
            asset_world::CameraSurfRange {
                kind: asset_world::CameraRangeKind::LitOpaque,
                begin,
                end: lit_end,
            },
            Vec::new(),
        ),
        emissive_surfs_begin: 0,
        emissive_surfs_end: emissive_end,
    }
}

fn world_cull(rng: &mut Rng, seed: u64, cull_dists: Vec<u16>) -> WorldCull {
    let surfs = WORLD_SURFS as usize;
    let mut capture = asset_world::WorldCapture::default();
    capture.casters = asset_world::SurfaceCastsSunShadow::with_len(surfs);
    for surf in 0..surfs {
        if rng.chance(70) {
            capture.casters.set(surf);
        }
    }
    // Seeds 2 and 5 have no packed keys, so the bake walks the surfaces itself.
    if !matches!(seed, 2 | 5) {
        capture.packed_draw_surfs = (0..surfs)
            .map(|_| {
                let key = if rng.chance(80) { world_key(rng) } else { 0 };
                dpvs_t6::GfxDrawSurf::from_packed(key)
            })
            .collect();
    }
    let model = |start_surf, surface_count| asset_world::GfxBrushModelSurfs {
        start_surf,
        surface_count,
        surface_count_no_decal: surface_count,
    };
    let mut brush_models = vec![asset_world::GfxBrushModelSurfs::default()];
    if seed != 4 {
        brush_models.push(model(300, 12));
        brush_models.push(model(360, 30));
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
    WorldCull {
        packed_indices: Vec::new(),
        surface_index_ranges: Vec::new(),
        surface_draw_fields: Vec::new(),
        surface_batch_ranges: Vec::new(),
        surface_materials: (0..surfs).map(|_| authored(rng)).collect(),
        surface_lightmap_indices: Vec::new(),
        surface_reflection_probes: Vec::new(),
        surface_primary_lights: (0..surfs).map(|_| rng.below(4) as u8).collect(),
        sort_key_distortion: None,
        surface_sort_keys: Vec::new(),
        capture,
        brush_models,
        brush_model_bounds: Vec::new(),
        bmodel_world_from_local: poses,
        t6_brush_surfaces: Vec::new(),
        dpvs: world_dpvs(seed),
        batch_count: 0,
        batch_lightmapped: Vec::new(),
        surface_vis: Vec::new(),
        draw_items: Vec::new(),
        item_surfs: Vec::new(),
        bsp_run_scratch: Vec::new(),
        draw_items_id: 0,
        g0_surfs: Vec::new(),
        static_model_entities: Vec::new(),
        static_model_cull_dists: cull_dists,
        smodel_vis: Vec::new(),
        cell_vis: Vec::new(),
        cell_vis_all: false,
    }
}

/// The parent's catalog, given what the sun bake reads: baked keys, a few cut-out names,
/// and technique sets that pass or fail the caster check.
fn sun_catalog(rng: &mut Rng, shape: Shape, generation: u64) -> RuntimeMaterialCatalog {
    let mut base = catalog(rng, shape, generation);
    for (i, row) in base.materials.iter_mut().enumerate() {
        row.baked_draw_surf = rng.chance(80).then(|| world_key(rng));
        if rng.chance(15) {
            row.name = format!("fence_{i}");
        }
        // Set 0 exists and has no caster technique; set 1 is past the end and passes.
        row.local_technique_set = if rng.chance(30) {
            RuntimeTechniqueSetId(0)
        } else {
            RuntimeTechniqueSetId(1)
        };
        row.remap = if rng.chance(10) {
            RemapResolution::Resolved(RuntimeTechniqueSetId(0))
        } else {
            RemapResolution::SelfSet
        };
    }
    let slots = (0..TECHNIQUE_SLOT_COUNT).map(|_| None).collect();
    base.technique_sets = vec![RuntimeTechniqueSet::new(slots).unwrap()];
    base
}

fn sun_smodels(rng: &mut Rng, shape: Shape, n: usize) -> SmodelGpuPlan {
    let mut plan = smodel_plan(rng, shape, n);
    for placement in &mut plan.placements {
        placement.primary_light_index = if rng.chance(85) { 1 } else { rng.below(4) as u8 };
        placement.flags = match rng.below(10) {
            6 => 0x1,
            7 => 0x2,
            8 => 0x3,
            _ => 0,
        };
    }
    // Vis slots map onto placements through a permutation, with holes and strays.
    plan.authored_placement_indices = (0..n)
        .map(|slot| match rng.below(100) {
            0..85 => Some((slot * 11 + 3) % n),
            85..93 => None,
            _ => Some(n + 5),
        })
        .collect();
    plan
}

fn xmodel_draw(rng: &mut Rng, i: usize) -> XModelSurfaceDraw {
    let bound = match rng.below(3) {
        0 => None,
        1 => Some(XModelCasterBound {
            origin: [rng.span(-800.0, 800.0), rng.span(-800.0, 800.0), rng.span(0.0, 200.0)],
            radius: rng.span(20.0, 200.0),
        }),
        _ => Some(XModelCasterBound {
            origin: [rng.span(-4000.0, 4000.0), rng.span(-4000.0, 4000.0), 0.0],
            radius: rng.span(10.0, 150.0),
        }),
    };
    XModelSurfaceDraw {
        surface: rng.below(100) as u32,
        // Two draws in ten name a material past the plan's list.
        material: rng.below(XMODEL_MATERIALS as u64 + 2) as u32,
        world_from_local: Mat4::from_translation(eye_at(rng)),
        lighting_handle: rng.below(300) as u32,
        pending_lighting: None,
        colour_refusal: None,
        object_id: if i == 3 { 1 } else { 2 + i as u16 * 13 + rng.below(5) as u16 },
        scene_light_index: rng.below(4) as u8,
        reflection_probe_index: rng.below(3) as u8,
        packed_lighting: rng.chance(50).then(|| [rng.below(256) as u8, 1, 2, 3]),
        is_scope: i == 5 || rng.chance(5),
        scene_entnum: rng.chance(60).then(|| rng.below(1000) as u32),
        caster_bound: bound,
    }
}

fn sun_xmodels(rng: &mut Rng) -> XModelDrawPlan {
    // The plan keeps private bookkeeping, so it is filled in rather than built field by field.
    let mut plan = XModelDrawPlan::default();
    plan.materials = (0..XMODEL_MATERIALS).map(|i| pass_material(i, rng)).collect();
    plan.draws = (0..XMODEL_DRAWS).map(|i| xmodel_draw(rng, i)).collect();
    plan
}

fn fixture(seed: u64) -> Fixture {
    let shape = shape(seed);
    let mut rng = Rng::new(0x5100 + seed);
    let n = if shape.dense_skinned { 420 } else { 240 };
    let cat = sun_catalog(&mut rng, shape, 2000 + seed);
    let mut smodel_plan = sun_smodels(&mut rng, shape, n);
    // Seed 3's cull distances stop halfway: the rest read as zero.
    let dist_len = if seed == 3 { n / 2 } else { n };
    let cull_dists: Vec<u16> = (0..dist_len)
        .map(|_| if rng.chance(50) { 0 } else { 500 + rng.below(5500) as u16 })
        .collect();
    let mut insts = Vec::new();
    fill_smodel_draw_inst_shadow(&mut insts, &cull_dists, &smodel_plan);
    smodel_plan.shadow_draw_insts = insts;
    let surfs = WORLD_SURFS as usize;
    // Seed 5 has visibility lists shorter than what they describe.
    let cut = |len: usize, by: usize| if seed == 5 { len - by } else { len };
    let surface_vis = [
        vis_bytes(&mut rng, cut(surfs, 60), 75),
        vis_bytes(&mut rng, cut(surfs, 60), 50),
    ];
    let smodel_vis = [
        vis_bytes(&mut rng, cut(n, 10), 75),
        vis_bytes(&mut rng, cut(n, 10), 50),
    ];
    let cull = world_cull(&mut rng, seed, cull_dists);
    let world_plan = if seed == 3 { WorldDrawGpuPlan::default() } else { world_plan(&mut rng) };
    let cache = shape.cache.then(|| cache(&mut rng, &smodel_plan, shape, n));
    let lighting = shape.lighting.then(|| lighting(&mut rng, n));
    let eye = (shape.eye != Eye::Missing).then(|| eye_at(&mut rng));
    let late = seed % 2 == 1;
    let ramp = LodRampDvar {
        t5: LodParmsAxis {
            applied_inv_scale: 1.0,
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
    Fixture {
        cull,
        world_plan,
        smodel_plan,
        catalog: cat,
        xmodel: (seed != 1 && seed != 6).then(|| sun_xmodels(&mut rng)),
        cache,
        lighting,
        pretess: shape.pretess,
        eye,
        ramp,
        surface_vis,
        smodel_vis,
        planes: [near, far],
    }
}

fn bake(fix: &Fixture, partition: usize) -> SunShadowCasterPlan {
    let inputs = SunBakeInputs {
        cull: &fix.cull,
        world_plan: &fix.world_plan,
        smodel_plan: &fix.smodel_plan,
        catalog: &fix.catalog,
        cache: fix.cache.as_ref(),
        lighting: fix.lighting.as_ref(),
        pretess_on: fix.pretess,
        eye: fix.eye,
        ramp: fix.ramp,
    };
    let vis = SunVisibility {
        surfaces: &fix.surface_vis[partition],
        smodels: &fix.smodel_vis[partition],
    };
    bake_partition(inputs, vis, &mut SunScratch::default())
}

fn partition_text(plan: &SunShadowCasterPlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "generation {:?}", plan.generation_id);
    let _ = writeln!(
        out,
        "world eligible {} missing_key {}",
        plan.world_eligible, plan.world_missing_key
    );
    let _ = writeln!(
        out,
        "smodel eligible {} missing_key {}",
        plan.smodel_eligible, plan.smodel_missing_key
    );
    let _ = writeln!(out, "pretess {:?}", plan.smodel_pretess_indices);
    let _ = writeln!(out, "items {}", plan.items.len());
    for item in &plan.items {
        let _ = writeln!(out, "  {item:?}");
    }
    out
}

/// The merged list a frame hands the sun pass: near partition first, then far.
fn merged_text(merged: &SunShadowCasterPlan) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "near {} of {}", merged.sun_near_n, merged.items.len());
    let _ = writeln!(
        out,
        "world eligible {} missing_key {}",
        merged.world_eligible, merged.world_missing_key
    );
    let _ = writeln!(
        out,
        "smodel eligible {} missing_key {}",
        merged.smodel_eligible, merged.smodel_missing_key
    );
    let _ = writeln!(out, "pretess {:?}", merged.smodel_pretess_indices);
    for item in &merged.items {
        let _ = writeln!(out, "  {item:?}");
    }
    out
}

#[derive(Default)]
struct SunCoverage {
    world: bool,
    world_posed: bool,
    smodel: bool,
    streams: [bool; 4],
    xmodel: bool,
    pretess_indices: bool,
    world_missing_key: bool,
    smodel_missing_key: bool,
    near_differs_from_far: bool,
    eye_missing: bool,
    eye_present: bool,
}

impl SunCoverage {
    fn note(&mut self, fix: &Fixture, items: &[RetainedDrawItem], near: &str, far: &str) {
        self.near_differs_from_far |= near != far;
        self.eye_missing |= fix.eye.is_none();
        self.eye_present |= fix.eye.is_some();
        for item in items {
            match item.kind {
                RetainedDrawKind::World { world_from_local, .. } => {
                    self.world = true;
                    self.world_posed |= world_from_local != Mat4::IDENTITY;
                }
                RetainedDrawKind::Smodel { stream, .. } => {
                    self.smodel = true;
                    let at = match stream {
                        Some(SmodelSurfPath::Rigid) => Some(0),
                        Some(SmodelSurfPath::Skinned) => Some(1),
                        Some(SmodelSurfPath::Cached) => Some(2),
                        Some(SmodelSurfPath::Pretess) => Some(3),
                        None => None,
                    };
                    if let Some(at) = at {
                        self.streams[at] = true;
                    }
                }
                RetainedDrawKind::XModel { .. } => self.xmodel = true,
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
            .filter(|(seen, _)| *seen)
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
            (self.world_missing_key, "world surface without a key"),
            (self.smodel_missing_key, "static model without a key"),
            (self.near_differs_from_far, "partitions that differ"),
            (self.eye_missing, "bake without an eye"),
            (self.eye_present, "bake with an eye"),
        ];
        checks
            .iter()
            .filter(|(seen, _)| !seen)
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
            !near.smodel_pretess_indices.is_empty() || !far.smodel_pretess_indices.is_empty();
        coverage.world_missing_key |= near.world_missing_key + far.world_missing_key > 0;
        coverage.smodel_missing_key |= near.smodel_missing_key + far.smodel_missing_key > 0;
        let merged_plan = merge_partitions(
            near,
            far,
            fix.xmodel.as_ref(),
            &fix.catalog,
            [fix.planes[0].as_slice(), fix.planes[1].as_slice()],
        );
        coverage.note(&fix, &merged_plan.items, &near_text, &far_text);
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
                .map(|(name, text)| format!("{seed} {name} {:016x}", fnv1a(text)))
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
