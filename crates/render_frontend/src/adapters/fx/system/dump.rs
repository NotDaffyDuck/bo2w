//! The report behind the `fx_dump [radius]` console command: what the
//! effects system produced this frame, what became of it, and the sprites
//! nearest the camera. It runs on the frame after the command and then
//! clears the request.

use std::collections::HashMap;

use bevy::prelude::*;
use fx::{EffectHole, CornerBuildOutput, EffectSpriteInstance, EffectSystemHost};
use render_fx::effect_play::EffectVertexHoles;
use render_fx::{
    BeamFate, BattleEffectsSnapshot, EffectJournalCursor, EffectsDynamicLightSet, PostEffectLightSet, JournalCount,
    ModelDrop, PostLightFate,
};

use crate::assemble::draw_items::{FxCodeDrawPlan, FxModelDrawList, FxCloudDrawPlan};

/// How many material rows and nearest sprites the report lists.
const LISTED: usize = 12;

/// What the commit stage drew, which the plans alone do not say.
pub(super) struct FrameTally {
    pub sprites_drawn: usize,
    pub trails_drawn: usize,
}

pub(super) struct DumpInput<'a> {
    pub host: &'a EffectSystemHost,
    pub out: &'a CornerBuildOutput,
    pub vertices_gaps: &'a EffectVertexHoles,
    pub tally: FrameTally,
    pub camera: &'a Transform,
    /// Sprites inside this distance are counted as "near".
    pub rad: f32,
    pub model_plan: &'a FxModelDrawList,
    pub plan: &'a FxCodeDrawPlan,
    pub spark_plan: &'a FxCloudDrawPlan,
    pub dlights: &'a EffectsDynamicLightSet,
    pub after_lights: &'a PostEffectLightSet,
    pub cursor: &'a EffectJournalCursor,
    pub combat: &'a BattleEffectsSnapshot,
}

pub(super) fn write_fx_dump(input: &DumpInput) {
    for line in frame_lines(input) {
        diag::info!(World, "{line}");
    }
    let eye = input.camera.translation.to_array();
    let facing = input.camera.forward().as_vec3();
    let radius = input.rad.max(1.0);

    let rows = material_rows(&input.out.sprites, eye);
    diag::info!(
        World,
        "fx_dump eye={:.0},{:.0},{:.0} facing={:.2},{:.2},{:.2} radius={radius:.0} materials: {}",
        eye[0],
        eye[1],
        eye[2],
        facing.x,
        facing.y,
        facing.z,
        rows.iter().take(LISTED).cloned().collect::<Vec<_>>().join(" | ")
    );

    let ranked = nearest_first(&input.out.sprites, eye);
    let inside = ranked.iter().take_while(|(dist, _)| *dist <= radius).count();
    diag::info!(
        World,
        "fx_dump sprites={} within_radius={inside}",
        ranked.len()
    );
    for (dist, sprite) in ranked.iter().take(LISTED) {
        diag::info!(World, "fx_dump near {}", describe_sprite(sprite, *dist, eye, facing));
    }
}

/// The counters, one line per area.
fn frame_lines(input: &DumpInput) -> Vec<String> {
    let DumpInput {
        host,
        out: built,
        vertices_gaps: gaps,
        tally,
        model_plan,
        plan,
        spark_plan,
        dlights,
        after_lights,
        cursor,
        combat,
        ..
    } = input;
    let counted = |kind| cursor.tally_of(kind);
    let beams = |fate| combat.beams.count_of(fate);
    let post = |fate| after_lights.count(fate);
    vec![
        format!(
            "fx_dump sprites={} drawn={} no_material={} trails={} trails_drawn={} clouds_skipped={} \
             codemesh_draws={} spark_draws={} spark_no_material={} no_ordinal={} not_emissive={}",
            built.sprites.len(),
            tally.sprites_drawn,
            counted(JournalCount::UnboundDraws),
            built.trail_strips.len(),
            tally.trails_drawn,
            built.cloud_skips,
            plan.runs.len(),
            spark_plan.runs.len(),
            spark_plan.unmatched_materials,
            counted(JournalCount::MeshNoOrdinal),
            counted(JournalCount::MeshNotEmissive),
        ),
        format!(
            "fx_dump left_out no_lookup={} unsupported={} other_type={} null_handler={} \
             no_trail_def={} dormant={} trail_code_mesh_gap={} \
             lookup[no_def={} no_elem={} no_material_visual={} no_size0={} size0<=0={} \
             no_size1={} size1<=0={}]",
            built.lookup_misses,
            built.unsupported_kind_skips,
            built.other_kind_skips,
            built.no_handler_skips,
            built.no_ribbon_def_skips,
            built.dormant_skips,
            host.gaps.hit_total(EffectHole::StreakProceduralMesh),
            gaps.without_definition,
            gaps.without_element,
            gaps.without_material_look,
            gaps.no_first_extent_sample,
            gaps.first_extent_nonpositive_total,
            gaps.no_second_extent_sample,
            gaps.second_extent_nonpositive_total,
        ),
        format!(
            "fx_dump models={} drawn={} skipped[model={} catalog={} pose={} lod={} culled={} \
             material={} lighting={} flags={}]",
            built.models.len(),
            model_plan.draws().len(),
            built.model_skips,
            model_plan.dropped.count_of(ModelDrop::NoCatalog),
            model_plan.dropped.count_of(ModelDrop::NoPose),
            model_plan.dropped.count_of(ModelDrop::NoLod),
            model_plan.dropped.count_of(ModelDrop::Culled),
            model_plan.dropped.count_of(ModelDrop::NoMaterial),
            model_plan.dropped.count_of(ModelDrop::NoLighting),
            model_plan.dropped.count_of(ModelDrop::RenderFlags),
        ),
        format!(
            "fx_dump lights omni={} spot={} scene={} omni_skipped={} spot_skipped={} cap_full={} \
             post[drawn={} short={} no_material={}] vis_blockers[write={} read={}]",
            built.omni_lights.len(),
            built.spot_lights.len(),
            dlights.scene.len(),
            built.omni_light_skips,
            built.skipped_cone_light,
            dlights.over_cap(),
            post(PostLightFate::Drawn),
            post(PostLightFate::TooShort),
            post(PostLightFate::Unbound),
            host.vis_blocker_write.count,
            host.vis_blocker_read.count,
        ),
        format!(
            "fx_dump tracer live={} queued={} drawn={} spawned={} \
             missing[material={} colour={} unprepared={} ordinal={} emissive={}] \
             skipped[interval={} short={} no_def={}] last[name={:?} material={:?} bind={:?} \
             colour={:?} speed={:?} beam_length={:?}]",
            combat.streak_live,
            beams(BeamFate::Queued),
            beams(BeamFate::Drawn),
            counted(JournalCount::TracerSpawned),
            beams(BeamFate::NoMaterial),
            beams(BeamFate::NoColor),
            beams(BeamFate::Unprepared),
            beams(BeamFate::NoOrdinal),
            beams(BeamFate::NoGlow),
            counted(JournalCount::TracerSkippedInterval),
            counted(JournalCount::TracerSkippedShort),
            counted(JournalCount::TracerSkippedNoDef),
            combat.sample.name,
            combat.sample.shader,
            combat.sample.bind,
            combat.sample.has_color,
            combat.sample.speed,
            combat.sample.ray_extent,
        ),
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from_array(a).distance(Vec3::from_array(b))
}

/// Every sprite with its distance from `eye`, nearest first.
fn nearest_first(sprites: &[EffectSpriteInstance], eye: [f32; 3]) -> Vec<(f32, &EffectSpriteInstance)> {
    let mut by_rank: Vec<_> = sprites
        .iter()
        .map(|sprite| (distance(sprite.where_at, eye), sprite))
        .collect();
    by_rank.sort_by(|a, b| a.0.total_cmp(&b.0));
    by_rank
}

/// One row per material, nearest material first: its name, how many sprites
/// use it, the closest of them and the highest alpha among them.
fn material_rows(sprites: &[EffectSpriteInstance], eye: [f32; 3]) -> Vec<String> {
    struct Stats<'a> {
        name: &'a str,
        count: usize,
        nearest: f32,
        top_alpha: u8,
    }
    let mut by_material: HashMap<Option<usize>, Stats> = HashMap::new();
    for seen in sprites {
        let fresh = Stats { name: seen.surface_title.as_ref(), count: 0, nearest: f32::INFINITY, top_alpha: 0 };
        let stats = by_material.entry(seen.shader_at).or_insert(fresh);
        stats.count += 1;
        stats.nearest = stats.nearest.min(distance(seen.where_at, eye));
        stats.top_alpha = stats.top_alpha.max(seen.rgba_tint[3]);
    }
    let mut row_list: Vec<_> = by_material.into_iter().collect();
    row_list.sort_by(|a, b| a.1.nearest.total_cmp(&b.1.nearest));
    row_list.into_iter()
        .map(|(material, stats)| {
            let id = material.map_or_else(|| "-".to_string(), |id| id.to_string());
            format!(
                "#{id} `{}` n={} near={:.0} alpha<={}",
                stats.name, stats.count, stats.nearest, stats.top_alpha
            )
        })
        .collect()
}

/// One near sprite as a report line: distance, whether it is in front of the
/// camera, colour, size, kind, names, position, flags and atlas size.
fn describe_sprite(shown: &EffectSpriteInstance, dist: f32, eye: [f32; 3], facing: Vec3) -> String {
    let ahead = (Vec3::from_array(shown.where_at) - Vec3::from_array(eye)).dot(facing) > 0.0;
    let [r, g, b, a] = shown.rgba_tint;
    let [x, y, z] = shown.where_at;
    let (size, kind, flags) = (shown.start_extent, shown.shape_kind, shown.bit_word);
    let (def, mat, cells) = (&shown.definition_name, &shown.surface_title, shown.atlas.entry_tally);
    format!(
        "dist={dist:.0} ahead={ahead} rgba={r},{g},{b},{a} size={size:.1} type={kind} def={def} mat={mat} \
         origin={x:.0},{y:.0},{z:.0} flags=0x{flags:x} atlas_entries={cells}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite_at(origin: [f32; 3], slot: Option<usize>, name: &str, alpha: u8) -> EffectSpriteInstance {
        EffectSpriteInstance {
            where_at: origin,
            start_extent: 1.0,
            second_extent: 0.0,
            rgba_tint: [10, 20, 30, alpha],
            definition_name: "def".into(),
            shape_kind: 0_u8,
            ledger_index: 0,
            definition_slot: 0,
            surface_title: name.into(),
            shader_at: slot,
            turn_rows: [Vec3::X, Vec3::Y, Vec3::Z].map(|unit| unit.to_array()),
            spin_rad: 0.0,
            move_dir: [0.0, 0.0, 1.0],
            atlas: fx_t6::runtime::BillboardSheetCell::FULL,
            bit_word: 0x42,
            line: false,
            lit_sample: [0; 3],
            shading_blend: 0,
            packed_lighting_src: fx::BakedShadeSource::White,
        }
    }

    #[test]
    fn distance_is_euclidean() {
        assert_eq!(distance([0.0; 3], [3.0, 4.0, 0.0]), 5.0);
    }

    #[test]
    fn sprites_are_ranked_nearest_first() {
        let sprites = [
            sprite_at([30.0, 0.0, 0.0], Some(1), "a", 1),
            sprite_at([10.0, 0.0, 0.0], Some(1), "a", 1),
            sprite_at([20.0, 0.0, 0.0], Some(1), "a", 1),
        ];
        let ranked = nearest_first(&sprites, [0.0; 3]);
        let dists: Vec<f32> = ranked.iter().map(|(d, _)| *d).collect();
        assert_eq!(dists, vec![10.0, 20.0, 30.0]);
    }

    #[test]
    fn material_rows_group_and_sort_by_the_nearest_sprite() {
        let sprites = [
            sprite_at([100.0, 0.0, 0.0], Some(7), "smoke", 90),
            sprite_at([50.0, 0.0, 0.0], Some(7), "smoke", 200),
            sprite_at([20.0, 0.0, 0.0], Some(3), "spark", 10),
        ];
        let rows = material_rows(&sprites, [0.0; 3]);
        assert_eq!(
            rows,
            vec![
                "#3 `spark` n=1 near=20 alpha<=10".to_string(),
                "#7 `smoke` n=2 near=50 alpha<=200".to_string(),
            ]
        );
    }

    #[test]
    fn a_sprite_without_a_material_gets_its_own_row() {
        let sprites = [sprite_at([5.0, 0.0, 0.0], None, "?", 1)];
        assert_eq!(material_rows(&sprites, [0.0; 3]), vec!["#- `?` n=1 near=5 alpha<=1".to_string()]);
    }

    #[test]
    fn a_sprite_is_ahead_when_it_lies_in_the_facing_half_space() {
        let s = sprite_at([10.0, 0.0, 0.0], Some(1), "m", 1);
        let ahead = describe_sprite(&s, 10.0, [0.0; 3], Vec3::X);
        let behind = describe_sprite(&s, 10.0, [0.0; 3], Vec3::NEG_X);
        assert!(ahead.contains("ahead=true"), "{ahead}");
        assert!(behind.contains("ahead=false"), "{behind}");
        assert!(ahead.contains("flags=0x42"));
    }
}
