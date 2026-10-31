//! Tracer beams: turning each queued beam into camera-facing triangles and
//! handing them to whichever mesh plan draws its material.

use bevy::math::Mat4;
use bevy::prelude::*;
use fx_t6::runtime::{TRAIL_TANGENT_WORD, trail_pack_normal, trail_pack_texcoord};
use fx_t6::{
    EffectBeamStitch, RayCorner, beam_generate_verts, beam_index_count, beam_vert_count,
    create_clip_matrix,
};
use render_fx::{
    BeamFate, BeamReport, BattleEffectsSnapshot, EffectLevelColorImages, StreakLevel, beam_undivided_colors,
};

use super::system::{EffectEngineMeshBind, EffectPresentSkip, T6FxMesh};
use crate::assemble::draw_items::MaterialEpoch;
use crate::assemble::draw_items::mesh_build::fx::FxCodeDrawPlan;

/// Is the `BO2MP_SHOT_LOG` debug switch set? Read once.
fn shot_log_enabled() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

/// Where the beams are seen from.
struct BeamView {
    pos: [f32; 3],
    fwd: [f32; 3],
    /// Clip matrix and its inverse, if the camera has one.
    clip: Option<([f32; 16], [f32; 16])>,
}


/// The view the beams are generated for. A clip matrix built from the field
/// of view is preferred; without a usable one, the camera's own clip matrix
/// stands in.
fn beam_view(
    camera: &Transform,
    world_to_clip: Option<[f32; 16]>,
    fov_half_tan: Option<(f32, f32)>,
) -> BeamView {
    let pos = camera.translation.to_array();
    let fwd = camera.forward().to_array();
    let from_fov = fov_half_tan
        .filter(|&(tan_x, tan_y)| tan_x > 0.0 && tan_y > 0.0)
        .map(|(tan_x, tan_y)| {
            let axes = [fwd, camera.left().to_array(), camera.up().to_array()];
            create_clip_matrix(pos, axes, tan_x, tan_y)
        });
    let clip = from_fov
        .or(world_to_clip)
        .map(|clip| (clip, Mat4::from_cols_array(&clip).inverse().to_cols_array()));
    BeamView { pos, fwd, clip }
}

/// Scratch space for one beam's triangles.
#[derive(Default)]
struct BeamScratch {
    vertex_points: Vec<RayCorner>,
    index_list: Vec<u16>,
}

impl BeamScratch {
    /// Generate `tess` into the scratch: how many vertices and indices were
    /// written, or `None` when the beam makes no triangles.
    fn generate(&mut self, tess: &EffectBeamStitch, view: &BeamView) -> Option<(usize, usize)> {
        let segments = usize::from(tess.piece_tally.max(1));
        let blank = RayCorner {
            spot: [0.0; 3],
            sheet_uv: [0.0; 2],
            color: [0; 4],
        };
        self.vertex_points.resize(beam_vert_count(segments), blank);
        self.index_list.resize(beam_index_count(segments), 0);
        let clip = view.clip.as_ref().map(|(clip, inverse)| (clip, inverse));
        let (vertex_tally, idx_count) = beam_generate_verts(
            tess,
            view.pos,
            view.fwd,
            clip,
            &mut self.vertex_points,
            &mut self.index_list,
        )?;
        beam_undivided_colors(tess, &mut self.vertex_points[..vertex_tally]);
        Some((vertex_tally, idx_count))
    }
}

/// Count a beam that could not be bound to a material.
fn count_skip(combat: &mut BattleEffectsSnapshot, skip: EffectPresentSkip) {
    combat.beams.bump(match skip {
        EffectPresentSkip::MissingColorMap => BeamFate::NoColor,
        EffectPresentSkip::NoOrdinal => BeamFate::NoOrdinal,
        EffectPresentSkip::NoGlow => BeamFate::NoGlow,
    });
}

/// Copy the generated triangles into the mesh plan as one draw. If either the
/// vertices or the indices do not fit, the draw is rolled back and left empty.
fn push_into_plan(
    plan: &mut FxCodeDrawPlan,
    scratch: &BeamScratch,
    counts: (usize, usize),
    draw: (Option<Handle<Image>>, u8, u32),
) -> bool {
    let (color, sort_key, ord) = draw;
    let start = (plan.pools.vertices_consumed, plan.pools.index_fill);
    let run = plan.open_run(color, sort_key, Some(ord));
    let up = trail_pack_normal([0.0, 1.0, 0.0]);

    let placed = scratch.vertex_points[..counts.0]
        .iter()
        .try_for_each(|v| {
            let words = [trail_pack_texcoord(v.sheet_uv[0], v.sheet_uv[1]), up, TRAIL_TANGENT_WORD];
            plan.append_trail_vertex(v.spot, v.color, words).then_some(())
        })
        .and_then(|()| {
            scratch.index_list[..counts.1].chunks_exact(3).try_for_each(|tri| {
                let shifted = [0, 1, 2].map(|k| start.0.saturating_add(u32::from(tri[k])));
                plan.append_indices(&shifted).then_some(())
            })
        })
        .is_some();
    if !placed {
        plan.cut_vertices_to(start.0);
        plan.cut_indices_to(start.1);
    }
    plan.close_run(run);
    placed
}

/// Present this frame's tracer beams: Black Ops II materials go to their
/// own mesh, others to the code-mesh plan through `bind`. Fills the combat
/// counters with what drew and what was skipped, and why.
#[allow(clippy::too_many_arguments)]
pub(crate) fn present_streak_beams(
    world: &StreakLevel,
    camera: &Transform,
    world_to_clip: Option<[f32; 16]>,
    fov_half_tan: Option<(f32, f32)>,
    colour_targets: &EffectLevelColorImages,
    runtime: &MaterialEpoch,
    plan: &mut FxCodeDrawPlan,
    combat: &mut BattleEffectsSnapshot,
    bind_fn: impl Fn(usize, &EffectLevelColorImages, &MaterialEpoch) -> EffectEngineMeshBind,
    t6: &mut T6FxMesh,
    t6_material: impl Fn(usize) -> bool,
) {
    combat.streak_live = world.pool.active_total() as u32;
    combat.beams = BeamReport::default();
    combat.beams.add_to(BeamFate::Queued, world.pending_list.len() as u32);
    combat.sample.has_color = None;

    let view = beam_view(camera, world_to_clip, fov_half_tan);
    let mut scratch = BeamScratch::default();
    let no_colour_images = colour_targets.is_empty();

    for ray in &world.pending_list {
        let Some(asset_id) = ray.mat_ref else {
            combat.beams.bump(BeamFate::NoMaterial);
            continue;
        };
        if t6_material(asset_id) {
            let Some((vertex_tally, index_count)) = scratch.generate(&ray.tess, &view) else {
                continue;
            };
            t6.push_beam(
                &scratch.vertex_points[..vertex_tally],
                &scratch.index_list[..index_count],
                asset_id as u32,
            );
            if shot_log_enabled() {
                diag::info!(
                    World,
                    "tracer verts: n {vertex_tally} i {index_count} v0 {:?} v1 {:?} v2 {:?} v3 {:?} idx {:?}",
                    scratch.vertex_points.first(),
                    scratch.vertex_points.get(1),
                    scratch.vertex_points.get(2),
                    scratch.vertex_points.get(3),
                    &scratch.index_list[..index_count.min(12)]
                );
            }
            combat.beams.bump(BeamFate::Drawn);
            continue;
        }
        if no_colour_images {
            combat.sample.has_color = Some(false);
            combat.beams.bump(BeamFate::Unprepared);
            continue;
        }
        combat.sample.has_color = Some(colour_targets.by_asset.colors.contains_key(&asset_id));
        match bind_fn(asset_id, colour_targets, runtime) {
            EffectEngineMeshBind::Skip(skip) => count_skip(combat, skip),
            EffectEngineMeshBind::Ready {
                tint: color,
                order_key: sort_key,
                ord_no: ordinal,
            } => {
                let Some(counts) = scratch.generate(&ray.tess, &view) else {
                    continue;
                };
                if push_into_plan(plan, &scratch, counts, (color, sort_key, ordinal)) {
                    combat.beams.bump(BeamFate::Drawn);
                }
            }
        }
    }

    if shot_log_enabled() && combat.streak_live > 0 {
        diag::info!(
            World,
            "tracer draw: live {} queued {} drawn {} miss_mat {} miss_color {} first {:?}",
            combat.streak_live,
            combat.beams.count_of(BeamFate::Queued),
            combat.beams.count_of(BeamFate::Drawn),
            combat.beams.count_of(BeamFate::NoMaterial),
            combat.beams.count_of(BeamFate::NoColor),
            world.pending_list.first().map(|b| (
                b.mat_ref,
                b.tess.begin.map(f32::round),
                b.tess.end.map(f32::round),
                b.tess.radius,
                b.tess.colors[0]
            ))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Transform {
        Transform::from_xyz(1.0, 2.0, 3.0)
    }

    #[test]
    fn the_view_takes_position_and_forward_from_the_camera() {
        let view = beam_view(&camera(), None, None);
        assert_eq!(view.pos, [1.0, 2.0, 3.0]);
        assert_eq!(view.fwd, camera().forward().to_array());
        assert!(view.clip.is_none());
    }

    #[test]
    fn the_camera_clip_matrix_is_the_fallback() {
        let ident_m = Mat4::IDENTITY.to_cols_array();
        let view = beam_view(&camera(), Some(ident_m), None);
        let (clip, inverse) = view.clip.expect("has clip");
        assert_eq!(clip, ident_m);
        assert_eq!(inverse, ident_m);
    }

    #[test]
    fn a_usable_field_of_view_builds_its_own_clip_matrix() {
        let ident_m = Mat4::IDENTITY.to_cols_array();
        let view = beam_view(&camera(), Some(ident_m), Some((1.0, 0.5)));
        let (clip, inverse) = view.clip.expect("has clip");
        assert_ne!(clip, ident_m);
        let product = Mat4::from_cols_array(&clip) * Mat4::from_cols_array(&inverse);
        assert!(product.abs_diff_eq(Mat4::IDENTITY, 1e-3));
    }

    #[test]
    fn a_field_of_view_with_a_zero_side_is_ignored() {
        let ident_m = Mat4::IDENTITY.to_cols_array();
        for fov in [(0.0, 1.0), (1.0, 0.0), (-1.0, 1.0)] {
            let view = beam_view(&camera(), Some(ident_m), Some(fov));
            assert_eq!(view.clip.map(|(clip, _)| clip), Some(ident_m));
        }
        assert!(beam_view(&camera(), None, Some((0.0, 0.0))).clip.is_none());
    }

    #[test]
    fn each_skip_reason_has_its_own_counter() {
        let mut combat = BattleEffectsSnapshot::default();
        count_skip(&mut combat, EffectPresentSkip::MissingColorMap);
        count_skip(&mut combat, EffectPresentSkip::NoOrdinal);
        count_skip(&mut combat, EffectPresentSkip::NoOrdinal);
        count_skip(&mut combat, EffectPresentSkip::NoGlow);
        assert_eq!(combat.beams.count_of(BeamFate::NoColor), 1);
        assert_eq!(combat.beams.count_of(BeamFate::NoOrdinal), 2);
        assert_eq!(combat.beams.count_of(BeamFate::NoGlow), 1);
    }

    #[test]
    fn skip_counters_stop_at_the_maximum() {
        let mut combat = BattleEffectsSnapshot::default();
        *combat.beams.slot(BeamFate::NoColor) = u32::MAX;
        count_skip(&mut combat, EffectPresentSkip::MissingColorMap);
        assert_eq!(combat.beams.count_of(BeamFate::NoColor), u32::MAX);
    }
}
