//! The frame's effect transaction: everything the effects draw this frame, generated in one go on
//! the main thread and handed to the commit step, which publishes it.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::{EffectHoleReason, EffectClockMs, EffectSystemHost};
use render_fx::FxFrameResult;
use render_fx::FxFrameView;
use render_fx::EffectStainTunables;
use render_fx::FxSkipCause;
use render_fx::GeneratedFxFrame;
use render_fx::EffectsDynamicLightSet;
use render_fx::PostEffectLightSet;
use render_fx::EffectsSystemHandle;
use render_fx::MarkSwitch;
use render_fx::ReadyFxBook;
use render_fx::StreakLevel;
use render_fx::step_tracer_beams;

use super::scene_access::EffectStageAccess;
use render_fx::effect_play::{
    EffectDrawHide, EffectStage, assemble_effect_vertices, refill_missing_packed_lighting,
};
use crate::assemble::draw_items::half_fov_tan_of_clip;
use crate::prepare::scene::camera::FreeRoamCamera;
use crate::prepare::scene::model_illum_atlas::LevelModelIllumAtlas;
use crate::prepare::scene::prop_geom_memo::DetailRampSkinnedTunable;
use crate::prepare::scene::eye_parms::StagedStageEye;

/// What one frame of effects produced, and the span that times it until the commit step drops it.
pub(super) struct EffectFrameTransaction {
    pub(super) outcome: FxFrameResult,
    pub(super) _present_span: perf::TimedStretchCloser,
}

/// Everything the commit step reads besides the transaction itself.
#[derive(SystemParam)]
pub(super) struct EffectPresentEnv<'w> {
    pub(super) dlights: ResMut<'w, EffectsDynamicLightSet>,
    pub(super) after_lights: ResMut<'w, PostEffectLightSet>,
    pub(super) models: Option<Res<'w, render_fx::ReadyFxProps>>,
    pub(super) model_geometry: Option<Res<'w, render_fx::PreparedFxModelPlan>>,
    pub(super) atlas: Option<Res<'w, LevelModelIllumAtlas>>,
    pub(super) at_spot: Res<'w, render_scene::LooseAtSpotLookup>,
    pub(super) detail_skinned: Res<'w, DetailRampSkinnedTunable>,
    pub(super) staged_models: ResMut<'w, render_fx::FxModelStagingPlan>,
}

/// Tells the host which effect elements found no light-grid sample, so the gap census counts them.
fn raise_lighting_gaps(host: &mut EffectSystemHost, def_indices: &[u8]) {
    for &definition_slot in def_indices {
        host.gaps.log_hole(EffectHoleReason::NoLightGridSample { definition_slot });
    }
}

/// Rolls the visibility blockers' write buffer into this frame's read buffer.
fn roll_vis_blockers(host: &mut EffectSystemHost) {
    let mut write = host.vis_blocker_write;
    let mut read = host.vis_blocker_read;
    fx_t6::runtime::vis_blocker_generate_verts(&mut write, &mut read);
    host.vis_blocker_write = write;
    host.vis_blocker_read = read;
}

#[allow(clippy::too_many_arguments)]
pub(super) fn open_fx_transaction(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut fx_host: ResMut<EffectsSystemHandle>,
    fx_book: Option<Res<ReadyFxBook>>,
    dvars: Res<EffectStainTunables>,
    cam_query: Query<&Transform, With<FreeRoamCamera>>,
    effect_level: EffectStageAccess,
    scene_eye: Option<Res<StagedStageEye>>,
    mut after_lights: ResMut<PostEffectLightSet>,
    mut tracer_list: ResMut<StreakLevel>,
    mark_models: Option<Res<asset_world::LevelModelPlacementRoster>>,
    stain_owners: Query<(
        &Visibility,
        &crate::prepare::scene::world::LevelScriptModelInstance,
        &Transform,
    )>,
    animation_refs: Option<Res<assets::StagedXAnims>>,
    mut sensor_lasers: super::super::sensor_laser::SensorLaserQueue,
    mut tag_lasers: super::super::tag_laser::TagLaserQueue,
) -> EffectFrameTransaction {
    let show_span = perf::Span::ServerEffectsShowCpuMillis.enter();
    let Some(effect_book) = fx_book else {
        return EffectFrameTransaction {
            outcome: FxFrameResult::Skipped(FxSkipCause::NoCatalog),
            _present_span: show_span,
        };
    };
    let camera = cam_query.iter().next().copied().unwrap_or(Transform::IDENTITY);
    let eye = camera.translation.to_array();

    fx_host.0.generate_level_stain_vertices();
    if let (Some(models), Some(scene)) = (mark_models.as_deref(), effect_level.scene()) {
        super::super::entity_decal::make_up(
            &mut fx_host.0,
            scene,
            effect_level.marks(),
            &stain_owners,
            animation_refs.as_deref(),
            models,
        );
    }

    let view = effect_level.view();
    refill_missing_packed_lighting(&mut fx_host.0, view.as_ref().map(|view| view as &dyn EffectStage));
    let view_volume_planes = scene_eye
        .as_ref()
        .map(|view| view.view_volume_planes.to_vec())
        .unwrap_or_default();
    let (out, vertices_gaps) = assemble_effect_vertices(
        &mut fx_host.0,
        &effect_book.0,
        EffectDrawHide {
            element_drawn: dvars.is_on(MarkSwitch::CullElemDraw),
            plane_list: &view_volume_planes,
        },
        eye,
    );
    raise_lighting_gaps(&mut fx_host.0, &vertices_gaps.light_fraction_default_slots);
    roll_vis_blockers(&mut fx_host.0);

    step_tracer_beams(&mut tracer_list, EffectClockMs(fx_host.0.clock_ms_now));
    sensor_lasers.queue(&mut tracer_list, fx_host.0.clock_ms_now, eye);
    tag_lasers.queue(&mut tracer_list, &mut after_lights, eye);

    EffectFrameTransaction {
        outcome: FxFrameResult::Ready(GeneratedFxFrame {
            products: out,
            gaps: vertices_gaps,
            marks: fx_host.0.last_stain_mesh.take(),
            after_lights: std::mem::take(&mut after_lights.lights),
            tracers: std::mem::take(&mut *tracer_list),
            view: FxFrameView {
                camera,
                frustum: view_volume_planes,
                view_proj: scene_eye
                    .as_ref()
                    .map(|view_pos| view_pos.screen_from_map.to_cols_array()),
                fov_half_tan: scene_eye
                    .as_ref()
                    .map(|look| half_fov_tan_of_clip(look.screen_from_eye)),
                level_present: effect_level.scene().is_some_and(|scene| scene.spawned),
            },
        }),
        _present_span: show_span,
    }
}
