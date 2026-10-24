//! Tracer beams: turning each queued beam into camera-facing triangles and
//! handing them to whichever mesh plan draws its material.

use bevy::math::Mat4;
use bevy::prelude::*;
use fx_t6::runtime::{FX_TRAIL_TANGENT_PACKED, trail_pack_normal, trail_pack_texcoord};
use fx_t6::{
    FxBeamTess, FxBeamVert, beam_generate_verts, beam_index_count, beam_vert_count,
    create_clip_matrix,
};
use render_fx::{CombatFxDump, FxWorldColorImages, TracerWorld, beam_undivided_colors};

use crate::adapters::fx::system::{FxCodeMeshBind, FxPresentSkip, T6FxMesh};
use crate::assemble::drawsurf::MaterialGeneration;
use crate::assemble::drawsurf::tess::fx::FxCodeMeshPlan;

/// Is the `BO2MP_SHOT_LOG` debug switch set? Read once.
fn shot_log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

/// Where the beams are seen from.
struct BeamView {
    pos: [f32; 3],
    fwd: [f32; 3],
    /// Clip matrix and its inverse, if the camera has one.
    clip: Option<([f32; 16], [f32; 16])>,
}

fn inverse_of(matrix: &[f32; 16]) -> [f32; 16] {
    Mat4::from_cols_array(matrix).inverse().to_cols_array()
}

/// The view the beams are generated for. A clip matrix built from the field
/// of view is preferred; without a usable one, the camera's own clip matrix
/// stands in.
fn beam_view(
    camera: &Transform,
    clip_from_world: Option<[f32; 16]>,
    tan_half_fov: Option<(f32, f32)>,
) -> BeamView {
    let pos = camera.translation.to_array();
    let fwd = camera.forward().to_array();
    let from_fov = tan_half_fov
        .filter(|&(tan_x, tan_y)| tan_x > 0.0 && tan_y > 0.0)
        .map(|(tan_x, tan_y)| {
            let axes = [fwd, camera.left().to_array(), camera.up().to_array()];
            create_clip_matrix(pos, axes, tan_x, tan_y)
        });
    let clip = from_fov
        .or(clip_from_world)
        .map(|clip| (clip, inverse_of(&clip)));
    BeamView { pos, fwd, clip }
}

/// Scratch space for one beam's triangles.
#[derive(Default)]
struct BeamScratch {
    verts: Vec<FxBeamVert>,
    indices: Vec<u16>,
}

impl BeamScratch {
    /// Generate `tess` into the scratch: how many vertices and indices were
    /// written, or `None` when the beam makes no triangles.
    fn generate(&mut self, tess: &FxBeamTess, view: &BeamView) -> Option<(usize, usize)> {
        let segments = usize::from(tess.segment_count.max(1));
        let blank = FxBeamVert {
            xyz: [0.0; 3],
            uv: [0.0; 2],
            color: [0; 4],
        };
        self.verts.resize(beam_vert_count(segments), blank);
        self.indices.resize(beam_index_count(segments), 0);
        let clip = view.clip.as_ref().map(|(clip, inverse)| (clip, inverse));
        let (vert_count, index_count) = beam_generate_verts(
            tess,
            view.pos,
            view.fwd,
            clip,
            &mut self.verts,
            &mut self.indices,
        )?;
        beam_undivided_colors(tess, &mut self.verts[..vert_count]);
        Some((vert_count, index_count))
    }
}

/// Count a beam that could not be bound to a material.
fn count_skip(combat: &mut CombatFxDump, skip: FxPresentSkip) {
    let counter = match skip {
        FxPresentSkip::NoColorMap => &mut combat.beam_miss_color,
        FxPresentSkip::NoOrdinal => &mut combat.beam_miss_ordinal,
        FxPresentSkip::NotEmissive => &mut combat.beam_miss_emissive,
    };
    *counter = counter.saturating_add(1);
}

/// Copy the generated triangles into the mesh plan as one draw. If either the
/// vertices or the indices do not fit, the draw is rolled back and left empty.
fn push_into_plan(
    plan: &mut FxCodeMeshPlan,
    scratch: &BeamScratch,
    counts: (usize, usize),
    draw: (Option<Handle<Image>>, u8, u32),
) -> bool {
    let (vert_count, index_count) = counts;
    let (color, sort_key, ordinal) = draw;
    let slot = plan.begin_material_draw(color, sort_key, Some(ordinal));
    let first_vert = plan.mesh.vert_used;
    let first_index = plan.mesh.index_used;
    let normal = trail_pack_normal([0.0, 1.0, 0.0]);

    let verts_fit = scratch.verts[..vert_count].iter().all(|vert| {
        plan.push_trail_vert(
            vert.xyz,
            vert.color,
            trail_pack_texcoord(vert.uv[0], vert.uv[1]),
            normal,
            FX_TRAIL_TANGENT_PACKED,
        )
    });
    let indices_fit = verts_fit
        && scratch.indices[..index_count].chunks_exact(3).all(|tri| {
            let rebased =
                [tri[0], tri[1], tri[2]].map(|index| first_vert.saturating_add(u32::from(index)));
            plan.extend_indices(&rebased)
        });
    if !indices_fit {
        plan.shrink_verts_to(first_vert);
        if verts_fit {
            plan.shrink_indices_to(first_index);
        }
    }
    plan.end_material_draw(slot);
    indices_fit
}

/// Present this frame's tracer beams: Black Ops II materials go to their
/// own mesh, others to the code-mesh plan through `bind`. Fills the combat
/// counters with what drew and what was skipped, and why.
#[allow(clippy::too_many_arguments)]
pub(crate) fn present_tracer_beams(
    world: &TracerWorld,
    camera: &Transform,
    clip_from_world: Option<[f32; 16]>,
    tan_half_fov: Option<(f32, f32)>,
    color_images: &FxWorldColorImages,
    runtime: &MaterialGeneration,
    plan: &mut FxCodeMeshPlan,
    combat: &mut CombatFxDump,
    bind: impl Fn(usize, &FxWorldColorImages, &MaterialGeneration) -> FxCodeMeshBind,
    t6: &mut T6FxMesh,
    t6_material: impl Fn(usize) -> bool,
) {
    combat.tracer_live = world.pool.live_count() as u32;
    combat.beam_queued = world.queued.len() as u32;
    combat.beam_drawn = 0;
    combat.beam_miss_material = 0;
    combat.beam_miss_color = 0;
    combat.beam_miss_unprepared = 0;
    combat.beam_miss_ordinal = 0;
    combat.beam_miss_emissive = 0;
    combat.last_tracer_has_color = None;

    let view = beam_view(camera, clip_from_world, tan_half_fov);
    let mut scratch = BeamScratch::default();
    let no_colour_images =
        color_images.colors.is_empty() && color_images.colors_by_asset.is_empty();

    for beam in &world.queued {
        let Some(asset_id) = beam.material else {
            combat.beam_miss_material = combat.beam_miss_material.saturating_add(1);
            continue;
        };
        if t6_material(asset_id) {
            let Some((vert_count, index_count)) = scratch.generate(&beam.tess, &view) else {
                continue;
            };
            t6.push_beam(
                &scratch.verts[..vert_count],
                &scratch.indices[..index_count],
                asset_id as u32,
            );
            if shot_log_enabled() {
                diag::info!(
                    World,
                    "tracer verts: n {vert_count} i {index_count} v0 {:?} v1 {:?} v2 {:?} v3 {:?} idx {:?}",
                    scratch.verts.first(),
                    scratch.verts.get(1),
                    scratch.verts.get(2),
                    scratch.verts.get(3),
                    &scratch.indices[..index_count.min(12)]
                );
            }
            combat.beam_drawn = combat.beam_drawn.saturating_add(1);
            continue;
        }
        if no_colour_images {
            combat.last_tracer_has_color = Some(0);
            combat.beam_miss_unprepared = combat.beam_miss_unprepared.saturating_add(1);
            continue;
        }
        combat.last_tracer_has_color = Some(i64::from(
            color_images.colors_by_asset.contains_key(&asset_id),
        ));
        match bind(asset_id, color_images, runtime) {
            FxCodeMeshBind::Skip(skip) => count_skip(combat, skip),
            FxCodeMeshBind::Ready {
                color,
                sort_key,
                ordinal,
            } => {
                let Some(counts) = scratch.generate(&beam.tess, &view) else {
                    continue;
                };
                if push_into_plan(plan, &scratch, counts, (color, sort_key, ordinal)) {
                    combat.beam_drawn = combat.beam_drawn.saturating_add(1);
                }
            }
        }
    }

    if shot_log_enabled() && combat.tracer_live > 0 {
        diag::info!(
            World,
            "tracer draw: live {} queued {} drawn {} miss_mat {} miss_color {} first {:?}",
            combat.tracer_live,
            combat.beam_queued,
            combat.beam_drawn,
            combat.beam_miss_material,
            combat.beam_miss_color,
            world.queued.first().map(|b| (
                b.material,
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
        let identity = Mat4::IDENTITY.to_cols_array();
        let view = beam_view(&camera(), Some(identity), None);
        let (clip, inverse) = view.clip.expect("has clip");
        assert_eq!(clip, identity);
        assert_eq!(inverse, identity);
    }

    #[test]
    fn a_usable_field_of_view_builds_its_own_clip_matrix() {
        let identity = Mat4::IDENTITY.to_cols_array();
        let view = beam_view(&camera(), Some(identity), Some((1.0, 0.5)));
        let (clip, inverse) = view.clip.expect("has clip");
        assert_ne!(clip, identity);
        let product = Mat4::from_cols_array(&clip) * Mat4::from_cols_array(&inverse);
        assert!(product.abs_diff_eq(Mat4::IDENTITY, 1e-3));
    }

    #[test]
    fn a_field_of_view_with_a_zero_side_is_ignored() {
        let identity = Mat4::IDENTITY.to_cols_array();
        for fov in [(0.0, 1.0), (1.0, 0.0), (-1.0, 1.0)] {
            let view = beam_view(&camera(), Some(identity), Some(fov));
            assert_eq!(view.clip.map(|(clip, _)| clip), Some(identity));
        }
        assert!(beam_view(&camera(), None, Some((0.0, 0.0))).clip.is_none());
    }

    #[test]
    fn each_skip_reason_has_its_own_counter() {
        let mut combat = CombatFxDump::default();
        count_skip(&mut combat, FxPresentSkip::NoColorMap);
        count_skip(&mut combat, FxPresentSkip::NoOrdinal);
        count_skip(&mut combat, FxPresentSkip::NoOrdinal);
        count_skip(&mut combat, FxPresentSkip::NotEmissive);
        assert_eq!(combat.beam_miss_color, 1);
        assert_eq!(combat.beam_miss_ordinal, 2);
        assert_eq!(combat.beam_miss_emissive, 1);
    }

    #[test]
    fn skip_counters_stop_at_the_maximum() {
        let mut combat = CombatFxDump::default();
        combat.beam_miss_color = u32::MAX;
        count_skip(&mut combat, FxPresentSkip::NoColorMap);
        assert_eq!(combat.beam_miss_color, u32::MAX);
    }
}
