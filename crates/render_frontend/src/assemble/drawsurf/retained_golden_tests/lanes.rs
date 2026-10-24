//! Golden tests for the dynamic-model lane and the effect lane: seeded fixtures run through
//! a series of frames each, and every frame's lane output is hashed against
//! `xmodel_lane_golden.txt` / `fx_lane_golden.txt`. A rewrite of either lane has to keep
//! every hash.
//!
//! The dynamic-model lane is tested from the merged plan onwards (`refresh_xmodel_lane`):
//! merging the plans the game keeps is a different step, with its own tests. The effect lane
//! is run as the system it is.
//!
//! Lives under `retained_golden_tests` so it can reuse that file's fixture builders.
//! `GOLDEN_BLESS=1` rewrites the golden files; `GOLDEN_DUMP=<dir>` writes each frame's text
//! as `xm<seed>-<step>.txt` and `fx<seed>-<step>.txt` for diffing.

use std::fmt::Write as _;

use fx_t6::runtime::GfxParticleCloud;

use super::*;

use crate::assemble::drawsurf::{
    FxDrawLane, XModelDrawLane, rebuild_fx_draw_lane, refresh_xmodel_lane,
};
use crate::assemble::drawsurf::retained::RetainedDrawItem;
use crate::assemble::drawsurf::tess::fx::{FxCodeMeshPlan, FxPassMaterial, FxSurfaceDraw};
use crate::assemble::drawsurf::tess::mark::{GfxMarkMeshDraw, GfxMarkMeshPlan, GfxMarkSubKey};
use crate::assemble::drawsurf::tess::particle_cloud::{FxParticleCloudDraw, FxParticleCloudPlan};
use crate::assemble::drawsurf::tess::xmodel::{
    XModelColourRefusal, XModelDrawPlan, XModelSurfaceDraw,
};
use crate::prepare::scene::model_lighting_cache::{
    ModelLightingOwner, ResolvedModelLighting, ResolvedModelLightingTable,
};

const XMODEL_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/drawsurf/xmodel_lane_golden.txt"
);
const FX_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/drawsurf/fx_lane_golden.txt"
);
const LANE_SEEDS: u64 = 8;
const XMODEL_FRAMES: u32 = 9;
const FX_FRAMES: u32 = 10;
const XMODEL_MATERIALS: usize = 8;
const XMODEL_DRAWS: usize = 40;
const FX_MATERIALS: usize = 9;
const FX_DRAWS: usize = 16;
const GLASS_PIECES: u16 = 6;
const SMODEL_SLOTS: usize = 30;

/// The parent's catalog, given baked keys so the dynamic models have something to key from.
fn lane_catalog(rng: &mut Rng, shape: Shape, generation: u64) -> RuntimeMaterialCatalog {
    let mut base = catalog(rng, shape, generation);
    for row in &mut base.materials {
        row.baked_draw_surf = rng.chance(80).then(|| world_key(rng));
    }
    base
}

fn lane_xmodel_draw(rng: &mut Rng, i: usize) -> XModelSurfaceDraw {
    XModelSurfaceDraw {
        surface: rng.below(100) as u32,
        // Two draws in ten name a material past the plan's list.
        material: rng.below(XMODEL_MATERIALS as u64 + 2) as u32,
        world_from_local: Mat4::from_translation(eye_at(rng)),
        lighting_handle: if rng.chance(75) { 1 + rng.below(300) as u32 } else { 0 },
        pending_lighting: None,
        colour_refusal: rng.chance(8).then_some(XModelColourRefusal::CameraFrustum),
        // Ids differ from draw to draw, apart from the zeros.
        object_id: if i % 7 == 3 { 0 } else { 1 + i as u16 * 17 + rng.below(16) as u16 },
        scene_light_index: rng.below(4) as u8,
        reflection_probe_index: rng.below(3) as u8,
        packed_lighting: rng.chance(50).then(|| [rng.below(256) as u8, 1, 2, 3]),
        is_scope: rng.chance(10),
        scene_entnum: rng.chance(60).then(|| rng.below(1000) as u32),
        caster_bound: None,
    }
}

fn lane_xmodel_plan(rng: &mut Rng) -> XModelDrawPlan {
    // The plan keeps private bookkeeping, so it is filled in rather than built field by field.
    let mut plan = XModelDrawPlan::default();
    plan.materials = (0..XMODEL_MATERIALS).map(|i| pass_material(i, rng)).collect();
    plan.draws = (0..XMODEL_DRAWS).map(|i| lane_xmodel_draw(rng, i)).collect();
    plan.revision = 1;
    plan
}

/// Moves every draw and changes what it carries but not what it is: a draw lit before is lit
/// after and an unlit one stays unlit, so the lane's layout is the same.
fn xmodel_payload_change(plan: &mut XModelDrawPlan, rng: &mut Rng) {
    for draw in &mut plan.draws {
        draw.world_from_local = Mat4::from_translation(eye_at(rng));
        if draw.lighting_handle != 0 {
            draw.lighting_handle = 1 + rng.below(300) as u32;
        }
        draw.packed_lighting = rng.chance(50).then(|| [rng.below(256) as u8, 4, 5, 6]);
        draw.is_scope = rng.chance(10);
        draw.scene_entnum = rng.chance(60).then(|| rng.below(1000) as u32);
    }
    plan.revision += 1;
}

fn lane_items(out: &mut String, products: [(&str, &[RetainedDrawItem]); 3]) {
    for (name, items) in products {
        let _ = writeln!(out, "{name} {}", items.len());
        for item in items {
            item_line(out, item);
        }
    }
}

fn xmodel_text(lane: &XModelDrawLane) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "revisions membership {} payload {}",
        lane.membership_revision, lane.payload_revision
    );
    let _ = writeln!(
        out,
        "merge_packed_n {:?} sorted {} standin {} exhausted {}",
        lane.merge_packed_n, lane.sorted, lane.object_id_standin, lane.fx_object_id_exhausted
    );
    let _ = writeln!(
        out,
        "skipped no_ordinal {} no_baked_key {} camera_frustum {} no_lighting {}",
        lane.skipped_no_ordinal,
        lane.skipped_no_baked_key,
        lane.skipped_camera_frustum,
        lane.skipped_no_lighting
    );
    lane_items(
        &mut out,
        [
            ("colour", &lane.colour),
            ("emissive", &lane.emissive),
            ("distortion", &lane.distortion),
        ],
    );
    out
}

/// What the dynamic-model frames reached; a golden over fixtures that never get here would
/// pass a rewrite that breaks it.
#[derive(Default)]
struct XmCoverage {
    products: [bool; 3],
    overlay: bool,
    rebuild: bool,
    skips: [bool; 4],
    standin: bool,
    object_id_zero: bool,
    packed_rows: bool,
    exhausted: bool,
}

impl XmCoverage {
    fn note(&mut self, rebuilt: bool, lane: &XModelDrawLane) {
        self.products[0] |= !lane.colour.is_empty();
        self.products[1] |= !lane.emissive.is_empty();
        self.products[2] |= !lane.distortion.is_empty();
        if rebuilt {
            self.rebuild = true;
        } else {
            self.overlay = true;
        }
        self.skips[0] |= lane.skipped_no_ordinal > 0;
        self.skips[1] |= lane.skipped_no_baked_key > 0;
        self.skips[2] |= lane.skipped_camera_frustum > 0;
        self.skips[3] |= lane.skipped_no_lighting > 0;
        self.standin |= lane.object_id_standin > 0;
        self.object_id_zero |= lane.sorted > lane.object_id_standin;
        self.packed_rows |= lane.merge_packed_n.is_some();
        self.exhausted |= lane.fx_object_id_exhausted > 0;
    }

    fn missing(&self) -> Vec<&'static str> {
        [
            (self.products[0], "colour items"),
            (self.products[1], "emissive items"),
            (self.products[2], "distortion items"),
            (self.overlay, "a frame that only overlays the payload"),
            (self.rebuild, "a frame that rebuilds the lane"),
            (self.skips[0], "a draw without an ordinal"),
            (self.skips[1], "a draw without a baked key"),
            (self.skips[2], "a draw refused by the camera frustum"),
            (self.skips[3], "a draw without lighting"),
            (self.standin, "a draw with an object id"),
            (self.object_id_zero, "a draw without an object id"),
            (self.packed_rows, "a plan with packed rows"),
            (self.exhausted, "a plan out of fx object ids"),
        ]
        .into_iter()
        .filter(|&(seen, _)| !seen)
        .map(|(_, name)| name)
        .collect()
    }
}

/// `(seed, step, text)` for every dynamic-model frame.
fn xmodel_frames(coverage: &mut XmCoverage) -> Vec<(u64, u32, String)> {
    let mut frames = Vec::new();
    for seed in 0..LANE_SEEDS {
        let shape = shape(seed);
        let mut rng = Rng::new(0x4c00 + seed);
        let mut catalog = lane_catalog(&mut rng, shape, 3000 + seed);
        let mut plan = lane_xmodel_plan(&mut rng);
        let mut lane = XModelDrawLane::default();
        for step in 1..=XMODEL_FRAMES {
            match step {
                // Frame 2 changes nothing.
                3 => {
                    xmodel_payload_change(&mut plan, &mut rng);
                    plan.packed_share =
                        Some(Arc::new(vec![[0u8; asset_t6::T6_PACKED_VERTEX]; 7]));
                }
                4 => {
                    for draw in plan.draws.iter_mut().step_by(3) {
                        draw.lighting_handle = 0;
                    }
                    plan.revision += 1;
                }
                5 => {
                    plan.draws.pop();
                }
                6 => plan.topology_revision += 1,
                7 => catalog = lane_catalog(&mut rng, shape, 3100 + seed),
                8 => {
                    for draw in plan.draws.iter_mut().skip(1).step_by(4) {
                        draw.colour_refusal = match draw.colour_refusal {
                            None => Some(XModelColourRefusal::CameraFrustum),
                            Some(_) => None,
                        };
                    }
                    plan.fx_object_id_exhausted = 2;
                }
                9 => xmodel_payload_change(&mut plan, &mut rng),
                _ => {}
            }
            let before = lane.membership_revision;
            refresh_xmodel_lane(&plan, &catalog, &mut lane);
            coverage.note(lane.membership_revision != before, &lane);
            frames.push((seed, step, xmodel_text(&lane)));
        }
    }
    frames
}

fn fx_material(i: usize, rng: &mut Rng) -> FxPassMaterial {
    FxPassMaterial {
        color: None,
        sort_key: rng.below(64) as u8,
        // One material in five has no sorted ordinal; ranks past the catalog's table miss it.
        material_sorted_index: (i % 5 != 4).then(|| rng.below(RANKS) as u32),
    }
}

fn index_count(rng: &mut Rng) -> u32 {
    if rng.chance(15) { 0 } else { 3 * (1 + rng.below(20) as u32) }
}

fn fx_code_plan(rng: &mut Rng) -> FxCodeMeshPlan {
    let args = FX_DRAWS * 2 + 4;
    let mut plan = FxCodeMeshPlan::default();
    plan.args = (0..args)
        .map(|_| [rng.span(-2.0, 2.0), rng.span(-2.0, 2.0), rng.span(0.0, 9.0), 1.0])
        .collect();
    plan.materials = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    plan.draws = (0..FX_DRAWS)
        .map(|i| FxSurfaceDraw {
            viewmodel: rng.chance(20),
            // Two draws in eleven name a material past the plan's list.
            material: rng.below(FX_MATERIALS as u64 + 2) as u32,
            index_start: i as u32 * 60,
            index_count: index_count(rng),
            // Argument ranges may run off the end of the list; counts may exceed two.
            arg_start: rng.below(args as u64 + 3) as u32,
            arg_count: rng.below(4) as u16,
        })
        .collect();
    plan.revision = 1;
    plan
}

fn cloud(rng: &mut Rng) -> GfxParticleCloud {
    GfxParticleCloud {
        quat: [0.0, 0.0, rng.span(-1.0, 1.0), 1.0],
        pos: [rng.span(-500.0, 500.0), rng.span(-500.0, 500.0), rng.span(0.0, 100.0)],
        placement_scale: rng.span(0.5, 2.0),
        axis_or_vel: [rng.span(-1.0, 1.0), 0.0, rng.span(0.0, 1.0)],
        color: rng.below(1 << 32) as u32,
        size0: rng.span(1.0, 8.0),
        size1: rng.span(1.0, 8.0),
        flags: rng.below(8) as u32,
        scale: rng.span(0.1, 3.0),
    }
}

fn fx_cloud_plan(rng: &mut Rng) -> FxParticleCloudPlan {
    let mut plan = FxParticleCloudPlan::default();
    plan.materials = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    plan.draws = (0..FX_DRAWS / 2)
        .map(|i| FxParticleCloudDraw {
            material: rng.below(FX_MATERIALS as u64 + 2) as u32,
            index_start: i as u32 * 60,
            index_count: index_count(rng),
            clouds: [cloud(rng), cloud(rng), cloud(rng)],
        })
        .collect();
    plan.revision = 1;
    plan
}

fn mark_sub_key(rng: &mut Rng) -> GfxMarkSubKey {
    let kind = rng.below(10);
    GfxMarkSubKey {
        lmap: if rng.chance(30) {
            marks_t6::GFX_SURFACE_LIGHTMAP_NONE
        } else {
            rng.below(4) as u8
        },
        packed: rng.chance(50),
        // No entity stands behind a mark here, so these never find their lighting.
        entity: (kind == 0).then(|| rng.below(60) as u16),
        // Some marks name a static model past the lighting's last slot.
        smodel: (1..=4).contains(&kind).then(|| rng.below(SMODEL_SLOTS as u64 + 6) as u16),
        glass: None,
        primary_light: rng.below(4) as u8,
        probe: rng.below(3) as u8,
    }
}

fn fx_mark_plan(rng: &mut Rng) -> GfxMarkMeshPlan {
    let mut plan = GfxMarkMeshPlan::default();
    plan.materials = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    plan.draws = (0..FX_DRAWS)
        .map(|i| GfxMarkMeshDraw {
            material: rng.below(FX_MATERIALS as u64 + 2) as u32,
            index_start: i as u32 * 60,
            index_count: index_count(rng),
            sub_key: mark_sub_key(rng),
        })
        .collect();
    plan.revision = 1;
    plan
}

/// Glass pieces 0, 1 and 3 are seated, 2 failed, 4 and 5 have no entry. `flipped` swaps
/// which of the first four are seated and seats piece 4 as well.
fn glass_lighting(flipped: bool) -> ResolvedModelLightingTable {
    let mut table = ResolvedModelLightingTable::default();
    let seat = |piece: u16| ResolvedModelLighting::Seated {
        handle: 10 + u32::from(piece),
        scene_light_index: piece as u8 % 4,
        reflection_probe_index: piece as u8 % 3,
        packed_lighting: None,
    };
    for piece in 0..GLASS_PIECES - 2 {
        let seated = matches!(piece, 0 | 1 | 3) != flipped;
        let value = if seated { seat(piece) } else { ResolvedModelLighting::Failed };
        table.insert_if_absent(ModelLightingOwner::Glass(piece), value);
    }
    if flipped {
        table.insert_if_absent(ModelLightingOwner::Glass(4), seat(4));
    }
    table
}

fn fx_text(lane: &FxDrawLane) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "revisions membership {} payload {}",
        lane.membership_revision, lane.payload_revision
    );
    let _ = writeln!(
        out,
        "sorted code {} cloud {} mark {}",
        lane.code_sorted, lane.particle_cloud_sorted, lane.mark_mesh_sorted
    );
    let _ = writeln!(
        out,
        "skipped code {} cloud {} mark {} mark_no_lighting {}",
        lane.code_skipped_no_ordinal,
        lane.particle_cloud_skipped_no_ordinal,
        lane.mark_mesh_skipped_no_ordinal,
        lane.mark_mesh_skipped_no_lighting
    );
    lane_items(
        &mut out,
        [
            ("colour", &lane.colour),
            ("emissive", &lane.emissive),
            ("distortion", &lane.distortion),
        ],
    );
    out
}

/// What the effect frames reached.
#[derive(Default)]
struct FxCoverage {
    products: [bool; 3],
    overlay: bool,
    rebuild: bool,
    sorted: [bool; 3],
    skips: [bool; 4],
    glass: bool,
    glass_rebuilds: bool,
    code_plan_missing: bool,
}

impl FxCoverage {
    fn note(&mut self, rebuilt: bool, lane: &FxDrawLane) {
        self.products[0] |= !lane.colour.is_empty();
        self.products[1] |= !lane.emissive.is_empty();
        self.products[2] |= !lane.distortion.is_empty();
        if rebuilt {
            self.rebuild = true;
        } else {
            self.overlay = true;
        }
        self.sorted[0] |= lane.code_sorted > 0;
        self.sorted[1] |= lane.particle_cloud_sorted > 0;
        self.sorted[2] |= lane.mark_mesh_sorted > 0;
        self.skips[0] |= lane.code_skipped_no_ordinal > 0;
        self.skips[1] |= lane.particle_cloud_skipped_no_ordinal > 0;
        self.skips[2] |= lane.mark_mesh_skipped_no_ordinal > 0;
        self.skips[3] |= lane.mark_mesh_skipped_no_lighting > 0;
        self.glass |= lane
            .colour
            .iter()
            .chain(&lane.emissive)
            .any(|item| matches!(item.kind, RetainedDrawKind::MarkMesh { glass: true, .. }));
    }

    fn missing(&self) -> Vec<&'static str> {
        [
            (self.products[0], "colour items"),
            (self.products[1], "emissive items"),
            (self.products[2], "distortion items"),
            (self.overlay, "a frame that only overlays the payload"),
            (self.rebuild, "a frame that rebuilds the lane"),
            (self.sorted[0], "code mesh items"),
            (self.sorted[1], "particle cloud items"),
            (self.sorted[2], "mark mesh items"),
            (self.skips[0], "a code mesh without an ordinal"),
            (self.skips[1], "a particle cloud without an ordinal"),
            (self.skips[2], "a mark mesh without an ordinal"),
            (self.skips[3], "a mark mesh without lighting"),
            (self.glass, "glass mark items"),
            (self.glass_rebuilds, "glass marks rebuilt on an unchanged frame"),
            (self.code_plan_missing, "a frame without the code mesh plan"),
        ]
        .into_iter()
        .filter(|&(seen, _)| !seen)
        .map(|(_, name)| name)
        .collect()
    }
}

/// `(seed, step, text)` for every effect frame. The static-model lighting and the material
/// catalog stay the same all the way through a seed.
fn fx_frames(coverage: &mut FxCoverage) -> Vec<(u64, u32, String)> {
    let mut frames = Vec::new();
    for seed in 0..LANE_SEEDS {
        let shape = shape(seed);
        let mut rng = Rng::new(0x2b00 + seed);
        let mut world = World::new();
        world.insert_resource(FxDrawLane::default());
        world.insert_resource(MaterialGeneration {
            catalog: Arc::new(lane_catalog(&mut rng, shape, 3200 + seed)),
            ..Default::default()
        });
        world.insert_resource(glass_lighting(false));
        if shape.lighting {
            world.insert_resource(lighting(&mut rng, SMODEL_SLOTS));
        }
        let code = fx_code_plan(&mut rng);
        world.insert_resource(code.clone());
        world.insert_resource(fx_cloud_plan(&mut rng));
        world.insert_resource(fx_mark_plan(&mut rng));
        for step in 1..=FX_FRAMES {
            match step {
                // Frame 2 changes nothing.
                3 => {
                    {
                        let mut code = world.resource_mut::<FxCodeMeshPlan>();
                        for row in &mut code.args {
                            row[0] += 1.5;
                        }
                        for draw in &mut code.draws {
                            draw.arg_start = rng.below(FX_DRAWS as u64 * 2) as u32;
                            draw.arg_count = rng.below(4) as u16;
                        }
                        code.bump();
                    }
                    {
                        let mut clouds = world.resource_mut::<FxParticleCloudPlan>();
                        for draw in &mut clouds.draws {
                            draw.clouds = [cloud(&mut rng), cloud(&mut rng), cloud(&mut rng)];
                        }
                        clouds.bump();
                    }
                    let mut marks = world.resource_mut::<GfxMarkMeshPlan>();
                    for draw in &mut marks.draws {
                        if rng.chance(50) {
                            draw.sub_key.packed = !draw.sub_key.packed;
                        }
                    }
                    marks.bump();
                }
                4 => {
                    {
                        let mut code = world.resource_mut::<FxCodeMeshPlan>();
                        code.draws[0].material = (code.draws[0].material + 3) % FX_MATERIALS as u32;
                        code.draws[1].viewmodel = !code.draws[1].viewmodel;
                        code.bump();
                    }
                    {
                        let mut clouds = world.resource_mut::<FxParticleCloudPlan>();
                        clouds.draws[0].material =
                            (clouds.draws[0].material + 2) % FX_MATERIALS as u32;
                        clouds.bump();
                    }
                    let mut marks = world.resource_mut::<GfxMarkMeshPlan>();
                    marks.draws[1].sub_key.lmap = (marks.draws[1].sub_key.lmap + 1) % 4;
                    marks.bump();
                }
                // Glass marks appear; from here until frame 8 the lane rebuilds every frame.
                5 => {
                    let mut marks = world.resource_mut::<GfxMarkMeshPlan>();
                    for draw in marks.draws.iter_mut().skip(2).step_by(3) {
                        draw.sub_key.glass = Some(rng.below(u64::from(GLASS_PIECES)) as u16);
                        draw.sub_key.entity = None;
                        draw.sub_key.smodel = None;
                    }
                    marks.bump();
                }
                7 => world.insert_resource(glass_lighting(true)),
                8 => {
                    world.remove_resource::<FxCodeMeshPlan>();
                    coverage.code_plan_missing = true;
                }
                9 => world.insert_resource(code.clone()),
                10 => world.resource_mut::<GfxMarkMeshPlan>().bump(),
                _ => {}
            }
            let before = world.resource::<FxDrawLane>().membership_revision;
            world
                .run_system_once(rebuild_fx_draw_lane)
                .expect("rebuild_fx_draw_lane runs");
            let lane = world.resource::<FxDrawLane>();
            let rebuilt = lane.membership_revision != before;
            coverage.note(rebuilt, lane);
            if step == 6 {
                coverage.glass_rebuilds |= rebuilt;
            }
            frames.push((seed, step, fx_text(lane)));
        }
    }
    frames
}

fn check_golden(path: &str, what: &str, frames: &[(u64, u32, String)], dump: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        for (seed, step, text) in frames {
            std::fs::write(format!("{dir}/{dump}{seed}-{step}.txt"), text).unwrap();
        }
    }
    let lines: Vec<String> = frames
        .iter()
        .map(|(seed, step, text)| format!("{seed} {step} {:016x}", fnv1a(text)))
        .collect();
    if std::env::var("GOLDEN_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }
    let golden = std::fs::read_to_string(path).expect("lane golden (bless with GOLDEN_BLESS=1)");
    let want: Vec<&str> = golden.lines().collect();
    let changed: Vec<String> = lines
        .iter()
        .zip(want.iter())
        .filter(|(got, want)| got.as_str() != **want)
        .map(|(got, want)| format!("want {want} got {got}"))
        .collect();
    assert_eq!(lines.len(), want.len(), "{what}: frame count changed");
    assert!(changed.is_empty(), "{what} changed:\n{}", changed.join("\n"));
}

#[test]
fn lanes_golden_xmodel_lane() {
    let mut coverage = XmCoverage::default();
    let frames = xmodel_frames(&mut coverage);
    assert!(
        coverage.missing().is_empty(),
        "xmodel fixtures never reach: {:?}",
        coverage.missing()
    );
    check_golden(XMODEL_GOLDEN, "xmodel lane", &frames, "xm");
}

#[test]
fn lanes_golden_fx_lane() {
    let mut coverage = FxCoverage::default();
    let frames = fx_frames(&mut coverage);
    assert!(
        coverage.missing().is_empty(),
        "fx fixtures never reach: {:?}",
        coverage.missing()
    );
    check_golden(FX_GOLDEN, "fx lane", &frames, "fx");
}

#[test]
fn lanes_golden_is_deterministic() {
    let first = xmodel_frames(&mut XmCoverage::default());
    let second = xmodel_frames(&mut XmCoverage::default());
    for (a, b) in first.iter().zip(second.iter()) {
        assert!(a == b, "xmodel seed {} step {} differs between runs", a.0, a.1);
    }
    let first = fx_frames(&mut FxCoverage::default());
    let second = fx_frames(&mut FxCoverage::default());
    for (a, b) in first.iter().zip(second.iter()) {
        assert!(a == b, "fx seed {} step {} differs between runs", a.0, a.1);
    }
}
