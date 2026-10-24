//! The report behind the `fx_dump [radius]` console command: what the
//! effects system produced this frame, what became of it, and the sprites
//! nearest the camera. It runs on the frame after the command and then
//! clears the request.

use std::collections::HashMap;

use bevy::prelude::*;
use fx::{FxGap, FxGenerateVertsOut, FxSpriteInstance, FxSystemHost};
use render_fx::present::FxVertsGaps;
use render_fx::{CombatFxDump, FxJournalCursor, HostFxDlights, HostFxPostLights};

use crate::assemble::drawsurf::{FxCodeMeshPlan, FxModelDrawPlan, FxParticleCloudPlan};

/// How many material rows and nearest sprites the report lists.
const LISTED: usize = 12;

/// What the commit stage drew, which the plans alone do not say.
pub(super) struct FrameTally {
    pub sprites_drawn: usize,
    pub trails_drawn: usize,
}

pub(super) struct DumpInput<'a> {
    pub host: &'a FxSystemHost,
    pub out: &'a FxGenerateVertsOut,
    pub verts_gaps: &'a FxVertsGaps,
    pub tally: FrameTally,
    pub camera: &'a Transform,
    /// Sprites inside this distance are counted as "near".
    pub radius: f32,
    pub model_plan: &'a FxModelDrawPlan,
    pub plan: &'a FxCodeMeshPlan,
    pub spark_plan: &'a FxParticleCloudPlan,
    pub dlights: &'a HostFxDlights,
    pub post_lights: &'a HostFxPostLights,
    pub cursor: &'a FxJournalCursor,
    pub combat: &'a CombatFxDump,
}

pub(super) fn write_fx_dump(input: &DumpInput) {
    for line in frame_lines(input) {
        diag::info!(World, "{line}");
    }
    let eye = input.camera.translation.to_array();
    let facing = input.camera.forward().as_vec3();
    let radius = input.radius.max(1.0);

    let rows = material_rows(&input.out.sprites, eye);
    diag::info!(
        World,
        "fx_dump cam={:.0},{:.0},{:.0} facing={:.2},{:.2},{:.2} radius={radius:.0} materials: {}",
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
        out,
        verts_gaps: gaps,
        tally,
        model_plan,
        plan,
        spark_plan,
        dlights,
        post_lights,
        cursor,
        combat,
        ..
    } = input;
    vec![
        format!(
            "fx_dump sprites={} drawn={} no_material={} trails={} trails_drawn={} clouds_skipped={} \
             codemesh_draws={} spark_draws={} spark_no_material={} no_ordinal={} not_emissive={}",
            out.sprites.len(),
            tally.sprites_drawn,
            cursor.draw_miss_material,
            out.trail_meshes.len(),
            tally.trails_drawn,
            out.skipped_cloud,
            plan.draws.len(),
            spark_plan.draws.len(),
            spark_plan.miss_material,
            cursor.skipped_no_ordinal,
            cursor.skipped_not_emissive,
        ),
        format!(
            "fx_dump left_out no_lookup={} unsupported={} other_type={} null_handler={} \
             no_trail_def={} dormant={} trail_code_mesh_gap={} \
             lookup[no_def={} no_elem={} no_material_visual={} no_size0={} size0<=0={} \
             no_size1={} size1<=0={}]",
            out.skipped_no_lookup,
            out.skipped_unsupported_type,
            out.skipped_other_type,
            out.skipped_null_handler,
            out.skipped_no_trail_def,
            out.skipped_dormant,
            host.gaps.hits(FxGap::TrailCodeMesh),
            gaps.no_def,
            gaps.no_elem,
            gaps.no_material_visual,
            gaps.no_size0_sample,
            gaps.size0_not_positive,
            gaps.no_size1_sample,
            gaps.size1_not_positive,
        ),
        format!(
            "fx_dump models={} drawn={} skipped[model={} catalog={} pose={} lod={} culled={} \
             material={} lighting={} flags={}]",
            out.models.len(),
            model_plan.draws().len(),
            out.skipped_model,
            model_plan.skipped_no_catalog,
            model_plan.skipped_no_pose,
            model_plan.skipped_no_lod,
            model_plan.skipped_culled,
            model_plan.skipped_no_material,
            model_plan.skipped_no_lighting,
            model_plan.skipped_render_fx_flags,
        ),
        format!(
            "fx_dump lights omni={} spot={} scene={} omni_skipped={} spot_skipped={} cap_full={} \
             post[drawn={} short={} no_material={}] vis_blockers[write={} read={}]",
            out.omni_lights.len(),
            out.spot_lights.len(),
            dlights.scene.len(),
            out.skipped_omni_light,
            out.skipped_spot_light,
            dlights.cap_full,
            post_lights.drawn,
            post_lights.skipped_short,
            post_lights.miss_material,
            host.vis_blocker_write.count,
            host.vis_blocker_read.count,
        ),
        format!(
            "fx_dump tracer live={} queued={} drawn={} spawned={} \
             missing[material={} colour={} unprepared={} ordinal={} emissive={}] \
             skipped[interval={} short={} no_def={}] last[name={:?} material={:?} bind={:?} \
             colour={:?} speed={:?} beam_length={:?}]",
            combat.tracer_live,
            combat.beam_queued,
            combat.beam_drawn,
            combat.tracer_spawned,
            combat.beam_miss_material,
            combat.beam_miss_color,
            combat.beam_miss_unprepared,
            combat.beam_miss_ordinal,
            combat.beam_miss_emissive,
            combat.tracer_skip_interval,
            combat.tracer_skip_short,
            combat.tracer_skip_no_def,
            combat.last_tracer_name,
            combat.last_tracer_material,
            combat.last_tracer_bind,
            combat.last_tracer_has_color,
            combat.last_tracer_speed,
            combat.last_tracer_beam_length,
        ),
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from_array(a).distance(Vec3::from_array(b))
}

/// Every sprite with its distance from `eye`, nearest first.
fn nearest_first(sprites: &[FxSpriteInstance], eye: [f32; 3]) -> Vec<(f32, &FxSpriteInstance)> {
    let mut ranked: Vec<_> = sprites
        .iter()
        .map(|sprite| (distance(sprite.origin, eye), sprite))
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
    ranked
}

/// One row per material, nearest material first: its name, how many sprites
/// use it, the closest of them and the highest alpha among them.
fn material_rows(sprites: &[FxSpriteInstance], eye: [f32; 3]) -> Vec<String> {
    struct Stats<'a> {
        name: &'a str,
        count: usize,
        nearest: f32,
        top_alpha: u8,
    }
    let mut by_material: HashMap<Option<usize>, Stats> = HashMap::new();
    for sprite in sprites {
        let stats = by_material.entry(sprite.material_index).or_insert(Stats {
            name: sprite.material_name.as_ref(),
            count: 0,
            nearest: f32::INFINITY,
            top_alpha: 0,
        });
        stats.count += 1;
        stats.nearest = stats.nearest.min(distance(sprite.origin, eye));
        stats.top_alpha = stats.top_alpha.max(sprite.color_rgba[3]);
    }
    let mut rows: Vec<_> = by_material.into_iter().collect();
    rows.sort_by(|a, b| a.1.nearest.total_cmp(&b.1.nearest));
    rows.into_iter()
        .map(|(material, stats)| {
            let id = material.map_or_else(|| "-".to_string(), |id| id.to_string());
            format!(
                "#{id} `{}` n={} near={:.0} alpha<={}",
                stats.name, stats.count, stats.nearest, stats.top_alpha
            )
        })
        .collect()
}

fn describe_sprite(sprite: &FxSpriteInstance, dist: f32, eye: [f32; 3], facing: Vec3) -> String {
    let ahead = (Vec3::from_array(sprite.origin) - Vec3::from_array(eye)).dot(facing) > 0.0;
    let [r, g, b, a] = sprite.color_rgba;
    format!(
        "dist={dist:.0} ahead={ahead} rgba={r},{g},{b},{a} size={:.1} type={} def={} mat={} \
         origin={:.0},{:.0},{:.0} flags=0x{:x} atlas_entries={}",
        sprite.size0,
        sprite.elem_type,
        sprite.def_name,
        sprite.material_name,
        sprite.origin[0],
        sprite.origin[1],
        sprite.origin[2],
        sprite.flags,
        sprite.atlas.entry_count,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite_at(origin: [f32; 3], material: Option<usize>, name: &str, alpha: u8) -> FxSpriteInstance {
        FxSpriteInstance {
            origin,
            size0: 1.0,
            size1: 0.0,
            color_rgba: [10, 20, 30, alpha],
            elem_type: 0,
            def_name: "def".into(),
            catalog_index: 0,
            def_index: 0,
            material_name: name.into(),
            material_index: material,
            axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            rotation_rad: 0.0,
            vel_dir: [0.0, 0.0, 1.0],
            atlas: fx_t6::runtime::FxSpriteAtlasUv::FULL,
            flags: 0x42,
            line: false,
            lighting_sample: [0; 3],
            lighting_frac: 0,
            packed_lighting_src: fx::FxPackedLightingSrc::White,
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
