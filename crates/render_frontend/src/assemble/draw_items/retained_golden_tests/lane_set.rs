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

use fx_t6::runtime::VisualParticleCloud;

use super::*;

use crate::{
    EffectDrawTrack, FigureDrawTrack, refill_fx_draw_lane, refresh_xmodel_lane,
};
use crate::assemble::draw_items::kept_lanes::HeldDrawItem;
use crate::assemble::draw_items::mesh_build::fx::{FxCodeDrawPlan, FxDrawRun, FxMaterialSlot, RunSpan};
use crate::assemble::draw_items::mesh_build::stain::{DecalMeshDrawCall, VisualStainMeshPlan, DecalSubmeshKey};
use crate::assemble::draw_items::mesh_build::cloud_lattice::{FxCloudDrawRun, FxCloudDrawPlan};
use crate::assemble::draw_items::mesh_build::figure_mesh::{
    ModelTintRejection, FigureDrawPlan, FigureFacetDraw,
};
use crate::prepare::scene::model_illum_cache::{
    ModelIllumOwner, ResolvedModelIllum, ResolvedModelIllumTable,
};

const XMODEL_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/draw_items/xmodel_lane_golden.txt"
);
const FX_GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/assemble/draw_items/fx_lane_golden.txt"
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
fn lane_catalog(rng: &mut Rng, shape: Shape, generation: u64) -> LiveMaterialLedger {
    let mut base = catalog(rng, shape, generation);
    for row in &mut base.materials {
        row.baked_facet_draw = rng.chance(80).then(|| world_key(rng));
    }
    base
}

fn lane_xmodel_draw(rng: &mut Rng, i: usize) -> FigureFacetDraw {
    FigureFacetDraw {
        surface: rng.below(100) as u32,
        // Two draws in ten name a material past the plan's list.
        material: rng.below(XMODEL_MATERIALS as u64 + 2) as u32,
        map_from_self: Mat4::from_translation(eye_at(rng)),
        illum_handle: if rng.chance(75) { 1 + rng.below(300) as u32 } else { 0 },
        queued_illum: None,
        colour_refusal: rng.chance(8).then_some(ModelTintRejection::CameraViewVolume),
        // Ids differ from draw to draw, apart from the zeros.
        object_tag: if i % 7 == 3 { 0 } else { 1 + i as u16 * 17 + rng.below(16) as u16 },
        stage_light_index: rng.below(4) as u8,
        env_probe_index: rng.below(3) as u8,
        compact_illum: rng.chance(50).then(|| [rng.below(256) as u8, 1, 2, 3]),
        is_scoped_view: rng.chance(10),
        stage_actor_num: rng.chance(60).then(|| rng.below(1000) as u32),
        caster_bound: None,
    }
}

fn lane_xmodel_plan(rng: &mut Rng) -> FigureDrawPlan {
    // The plan keeps private bookkeeping, so it is filled in rather than built field by field.
    let mut plan = FigureDrawPlan::default();
    plan.materials = (0..XMODEL_MATERIALS).map(|i| pass_material(i, rng)).collect();
    plan.draws = (0..XMODEL_DRAWS).map(|i| lane_xmodel_draw(rng, i)).collect();
    plan.revision = 1;
    plan
}

/// Moves every draw and changes what it carries but not what it is: a draw lit before is lit
/// after and an unlit one stays unlit, so the lane's layout is the same.
fn xmodel_payload_change(plan: &mut FigureDrawPlan, rng: &mut Rng) {
    for draw_item in &mut plan.draws {
        draw_item.map_from_self = Mat4::from_translation(eye_at(rng));
        if draw_item.illum_handle != 0 {
            draw_item.illum_handle = 1 + rng.below(300) as u32;
        }
        draw_item.compact_illum = rng.chance(50).then(|| [rng.below(256) as u8, 4, 5, 6]);
        draw_item.is_scoped_view = rng.chance(10);
        draw_item.stage_actor_num = rng.chance(60).then(|| rng.below(1000) as u32);
    }
    plan.revision += 1;
}

fn lane_items(text: &mut String, products: [(&str, &[HeldDrawItem]); 3]) {
    for (name, items) in products {
        let _ = writeln!(text, "{name} {}", items.len());
        for draw_entry in items {
            item_line(text, draw_entry);
        }
    }
}

fn xmodel_text(lane: &FigureDrawTrack) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "revisions membership {} payload {}",
        lane.membership_edition, lane.payload_edition
    );
    let _ = writeln!(
        out,
        "merge_packed_n {:?} sorted {} standin {} exhausted {}",
        lane.packed_row_count, lane.items_filed, lane.object_tag_standin, lane.effect_object_id_exhausted
    );
    let _ = writeln!(
        out,
        "skipped no_ordinal {} no_baked_key {} camera_frustum {} no_lighting {}",
        lane.skipped_unranked,
        lane.skipped_unbaked,
        lane.skipped_camera_view_volume,
        lane.skipped_no_illum
    );
    lane_items(
        &mut out,
        [
            ("colour", &lane.tint_rgb),
            ("emissive", &lane.glow),
            ("distortion", &lane.warp),
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
    fn record(&mut self, rebuilt: bool, lane: &FigureDrawTrack) {
        self.products[0] |= !lane.tint_rgb.is_empty();
        self.products[1] |= !lane.glow.is_empty();
        self.products[2] |= !lane.warp.is_empty();
        if rebuilt {
            self.rebuild = true;
        } else {
            self.overlay = true;
        }
        self.skips[0] |= lane.skipped_unranked > 0;
        self.skips[1] |= lane.skipped_unbaked > 0;
        self.skips[2] |= lane.skipped_camera_view_volume > 0;
        self.skips[3] |= lane.skipped_no_illum > 0;
        self.standin |= lane.object_tag_standin > 0;
        self.object_id_zero |= lane.items_filed > lane.object_tag_standin;
        self.packed_rows |= lane.packed_row_count.is_some();
        self.exhausted |= lane.effect_object_id_exhausted > 0;
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
        let mut lane = FigureDrawTrack::default();
        for step in 1..=XMODEL_FRAMES {
            match step {
                // Frame 2 changes nothing.
                3 => {
                    xmodel_payload_change(&mut plan, &mut rng);
                    plan.compact_portion =
                        Some(Arc::new(vec![[0u8; asset_t6::T6_PACKED_VERTEX]; 7]));
                }
                4 => {
                    for draw in plan.draws.iter_mut().step_by(3) {
                        draw.illum_handle = 0;
                    }
                    plan.revision += 1;
                }
                5 => {
                    plan.draws.pop();
                }
                6 => plan.topology_edition += 1,
                7 => catalog = lane_catalog(&mut rng, shape, 3100 + seed),
                8 => {
                    for draw in plan.draws.iter_mut().skip(1).step_by(4) {
                        draw.colour_refusal = match draw.colour_refusal {
                            None => Some(ModelTintRejection::CameraViewVolume),
                            Some(_) => None,
                        };
                    }
                    plan.effect_object_id_exhausted = 2;
                }
                9 => xmodel_payload_change(&mut plan, &mut rng),
                _ => {}
            }
            let before = lane.membership_edition;
            refresh_xmodel_lane(&plan, &catalog, &mut lane);
            coverage.record(lane.membership_edition != before, &lane);
            frames.push((seed, step, xmodel_text(&lane)));
        }
    }
    frames
}

fn fx_material(i: usize, rng: &mut Rng) -> FxMaterialSlot {
    let bucket = rng.below(64) as u8;
    // One material in five has no sorted ordinal; ranks past the catalog's table miss it.
    let sorted_rank = (i % 5 != 4).then(|| rng.below(RANKS) as u32);
    FxMaterialSlot { texture: None, bin: bucket, sorted_rank }
}

/// Two draws in eleven name a material past the plan's list.
fn pick_slot(rng: &mut Rng) -> u32 {
    rng.below(FX_MATERIALS as u64 + 2) as u32
}

fn index_count(rng: &mut Rng) -> u32 {
    if rng.chance(15) { 0 } else { 3 * (1 + rng.below(20) as u32) }
}

fn fx_code_plan(rng: &mut Rng) -> FxCodeDrawPlan {
    let args = FX_DRAWS * 2 + 4;
    let rows: Vec<[f32; 4]> = (0..args)
        .map(|_| [rng.span(-2.0, 2.0), rng.span(-2.0, 2.0), rng.span(0.0, 9.0), 1.0])
        .collect();
    let slots: Vec<_> = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    let runs = (0..FX_DRAWS)
        .map(|i| {
            let in_viewmodel = rng.chance(20);
            let surface_material_index = pick_slot(rng);
            let indices = RunSpan { first: i as u32 * 60, count: index_count(rng) };
            // Argument ranges may run off the end of the list; counts may exceed two.
            let first = rng.below(args as u64 + 3) as u32;
            let args = RunSpan { first, count: rng.below(4) as u32 };
            FxDrawRun { surface_material_index, in_viewmodel, indices, args }
        })
        .collect();
    let mut plan = FxCodeDrawPlan::default();
    (plan.shader_args, plan.slots, plan.runs) = (rows, slots, runs);
    plan.revision = 1;
    plan
}

fn cloud(rng: &mut Rng) -> VisualParticleCloud {
    VisualParticleCloud {
        turn: [0.0, 0.0, rng.span(-1.0, 1.0), 1.0],
        spot: [rng.span(-500.0, 500.0), rng.span(-500.0, 500.0), rng.span(0.0, 100.0)],
        layout_size_ratio: rng.span(0.5, 2.0),
        basis_or_speed: [rng.span(-1.0, 1.0), 0.0, rng.span(0.0, 1.0)],
        color: rng.below(1 << 32) as u32,
        start_extent: rng.span(1.0, 8.0),
        second_extent: rng.span(1.0, 8.0),
        flags: rng.below(8) as u32,
        scale: rng.span(0.1, 3.0),
    }
}

fn triple_cloud(rng: &mut Rng) -> [VisualParticleCloud; 3] {
    [cloud(rng), cloud(rng), cloud(rng)]
}

fn fx_cloud_plan(rng: &mut Rng) -> FxCloudDrawPlan {
    let slots: Vec<_> = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    let runs = (0..FX_DRAWS / 2)
        .map(|i| {
            let surface_material_index = pick_slot(rng);
            let (first, num_indices) = (i as u32 * 60, index_count(rng));
            let triple = triple_cloud(rng);
            FxCloudDrawRun { surface_material_index, first_index: first, num_indices, descriptors: triple }
        })
        .collect();
    let mut plan = FxCloudDrawPlan::default();
    (plan.cell_list, plan.runs) = (slots, runs);
    plan.revision = 1;
    plan
}

fn mark_sub_key(rng: &mut Rng) -> DecalSubmeshKey {
    let kind = rng.below(10);
    DecalSubmeshKey {
        baked_light_slot: if rng.chance(30) {
            marks_t6::SURFACE_BAKED_LIGHT_ABSENT
        } else {
            rng.below(4) as u8
        },
        packed: rng.chance(50),
        // No entity stands behind a mark here, so these never find their lighting.
        ent_id: (kind == 0).then(|| rng.below(60) as u16),
        // Some marks name a static model past the lighting's last slot.
        smodel: (1..=4).contains(&kind).then(|| rng.below(SMODEL_SLOTS as u64 + 6) as u16),
        glass: None,
        dominant_light: rng.below(4) as u8,
        probe: rng.below(3) as u8,
    }
}

fn fx_mark_plan(rng: &mut Rng) -> VisualStainMeshPlan {
    let mut plan = VisualStainMeshPlan::default();
    plan.mat_list = (0..FX_MATERIALS).map(|i| fx_material(i, rng)).collect();
    plan.draws = (0..FX_DRAWS)
        .map(|i| DecalMeshDrawCall {
            shader: rng.below(FX_MATERIALS as u64 + 2) as u32,
            first_triangle_slot: i as u32 * 60,
            index_count: index_count(rng),
            minor_key: mark_sub_key(rng),
        })
        .collect();
    plan.revision = 1;
    plan
}

/// Glass pieces 0, 1 and 3 are seated, 2 failed, 4 and 5 have no entry. `flipped` swaps
/// which of the first four are seated and seats piece 4 as well.
fn glass_lighting(flipped: bool) -> ResolvedModelIllumTable {
    let mut table = ResolvedModelIllumTable::default();
    let seat = |piece: u16| ResolvedModelIllum::Seated {
        handle: 10 + u32::from(piece),
        stage_light_index: piece as u8 % 4,
        env_probe_index: piece as u8 % 3,
        compact_illum: None,
    };
    for piece in 0..GLASS_PIECES - 2 {
        let seated = matches!(piece, 0 | 1 | 3) != flipped;
        let value = if seated { seat(piece) } else { ResolvedModelIllum::Failed };
        table.keep_first(ModelIllumOwner::Pane(piece), value);
    }
    if flipped {
        table.keep_first(ModelIllumOwner::Pane(4), seat(4));
    }
    table
}

fn fx_text(lane: &EffectDrawTrack) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "revisions membership {} payload {}",
        lane.membership_edition, lane.payload_edition
    );
    let _ = writeln!(
        out,
        "sorted code {} cloud {} mark {}",
        lane.engine_sorted, lane.particle_cloud_ordered, lane.stain_mesh_sorted
    );
    let _ = writeln!(
        out,
        "skipped code {} cloud {} mark {} mark_no_lighting {}",
        lane.engine_skipped_no_ordinal,
        lane.cloud_skipped_without_ordinal,
        lane.stain_mesh_skipped_no_ordinal,
        lane.stain_mesh_skipped_no_illum
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
    fn record(&mut self, rebuilt: bool, lane: &EffectDrawTrack) {
        self.products[0] |= !lane.colour.is_empty();
        self.products[1] |= !lane.emissive.is_empty();
        self.products[2] |= !lane.distortion.is_empty();
        if rebuilt {
            self.rebuild = true;
        } else {
            self.overlay = true;
        }
        self.sorted[0] |= lane.engine_sorted > 0;
        self.sorted[1] |= lane.particle_cloud_ordered > 0;
        self.sorted[2] |= lane.stain_mesh_sorted > 0;
        self.skips[0] |= lane.engine_skipped_no_ordinal > 0;
        self.skips[1] |= lane.cloud_skipped_without_ordinal > 0;
        self.skips[2] |= lane.stain_mesh_skipped_no_ordinal > 0;
        self.skips[3] |= lane.stain_mesh_skipped_no_illum > 0;
        self.glass |= lane
            .colour
            .iter()
            .chain(&lane.emissive)
            .any(|item| matches!(item.kind, HeldDrawKind::MarkMesh { glass: true, .. }));
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
        world.insert_resource(EffectDrawTrack::default());
        world.insert_resource(MaterialEpoch {
            catalog: Arc::new(lane_catalog(&mut rng, shape, 3200 + seed)),
            ..Default::default()
        });
        world.insert_resource(glass_lighting(false));
        if shape.lighting {
            world.insert_resource(light_data(&mut rng, SMODEL_SLOTS));
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
                        let mut code = world.resource_mut::<FxCodeDrawPlan>();
                        for row in &mut code.shader_args {
                            row[0] += 1.5;
                        }
                        for run in &mut code.runs {
                            run.args.first = rng.below(FX_DRAWS as u64 * 2) as u32;
                            run.args.count = rng.below(4) as u32;
                        }
                        code.advance_revision();
                    }
                    {
                        let mut clouds = world.resource_mut::<FxCloudDrawPlan>();
                        clouds.runs.iter_mut().for_each(|run| run.descriptors = triple_cloud(&mut rng));
                        clouds.advance_revision();
                    }
                    let mut marks = world.resource_mut::<VisualStainMeshPlan>();
                    for draw in &mut marks.draws {
                        if rng.chance(50) {
                            draw.minor_key.packed = !draw.minor_key.packed;
                        }
                    }
                    marks.next_edit();
                }
                4 => {
                    {
                        let mut code = world.resource_mut::<FxCodeDrawPlan>();
                        let moved = (code.runs[0].surface_material_index + 3) % FX_MATERIALS as u32;
                        code.runs[0].surface_material_index = moved;
                        code.runs[1].in_viewmodel ^= true;
                        code.advance_revision();
                    }
                    {
                        let mut clouds = world.resource_mut::<FxCloudDrawPlan>();
                        let moved = (clouds.runs[0].surface_material_index + 2) % FX_MATERIALS as u32;
                        clouds.runs[0].surface_material_index = moved;
                        clouds.advance_revision();
                    }
                    let mut marks = world.resource_mut::<VisualStainMeshPlan>();
                    marks.draws[1].minor_key.baked_light_slot = (marks.draws[1].minor_key.baked_light_slot + 1) % 4;
                    marks.next_edit();
                }
                // Glass marks appear; from here until frame 8 the lane rebuilds every frame.
                5 => {
                    let mut marks = world.resource_mut::<VisualStainMeshPlan>();
                    for draw in marks.draws.iter_mut().skip(2).step_by(3) {
                        draw.minor_key.glass = Some(rng.below(u64::from(GLASS_PIECES)) as u16);
                        draw.minor_key.ent_id = None;
                        draw.minor_key.smodel = None;
                    }
                    marks.next_edit();
                }
                7 => world.insert_resource(glass_lighting(true)),
                8 => {
                    world.remove_resource::<FxCodeDrawPlan>();
                    coverage.code_plan_missing = true;
                }
                9 => world.insert_resource(code.clone()),
                10 => world.resource_mut::<VisualStainMeshPlan>().next_edit(),
                _ => {}
            }
            let before = world.resource::<EffectDrawTrack>().membership_edition;
            world
                .run_system_once(refill_fx_draw_lane)
                .expect("rebuild_fx_draw_lane runs");
            let lane = world.resource::<EffectDrawTrack>();
            let rebuilt = lane.membership_edition != before;
            coverage.record(rebuilt, lane);
            if step == 6 {
                coverage.glass_rebuilds |= rebuilt;
            }
            frames.push((seed, step, fx_text(lane)));
        }
    }
    frames
}

fn check_golden(golden_file: &str, what: &str, frames: &[(u64, u32, String)], dump: &str) {
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
        std::fs::write(golden_file, lines.join("\n") + "\n").unwrap();
    }
    let golden = std::fs::read_to_string(golden_file).expect("lane golden (bless with GOLDEN_BLESS=1)");
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
