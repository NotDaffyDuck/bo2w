//! The frame's effect transaction: everything the effects draw this frame, generated in one go on
//! the main thread and handed to the commit step, which publishes it.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::{FxGapCause, FxMsec, FxSystemHost};
use render_fx::{
    FxFrameOutcome, FxGeneratedFrame, FxMarkDvars, FxUnavailableCause, HostFxDlights,
    HostFxPostLights, HostFxSystem, PreparedFxCatalog, PreparedFxModels, TracerWorld,
    tick_tracer_beams,
};

use super::scene_access::FxSceneAccess;
use crate::adapters::fx::present::{
    FxDrawCull, FxScene, build_fx_verts, restamp_missing_packed_lighting,
};
use crate::assemble::drawsurf::tan_half_fov_from_clip;
use crate::prepare::scene::{
    camera::FlyCamera, model_lighting_atlas::WorldModelLightingAtlas,
    smodel_geom_cache::LodRampSkinnedDvar, view_parms::PreparedSceneView,
};

/// What one frame of effects produced, and the span that times it until the commit step drops it.
pub(super) struct FxFrameTransaction {
    pub(super) outcome: FxFrameOutcome,
    pub(super) _present_span: perf::SpanGuard,
}

/// Everything the commit step reads besides the transaction itself.
#[derive(SystemParam)]
pub(super) struct FxPresentEnv<'w> {
    pub(super) dlights: ResMut<'w, HostFxDlights>,
    pub(super) post_lights: ResMut<'w, HostFxPostLights>,
    pub(super) models: Option<Res<'w, PreparedFxModels>>,
    pub(super) model_geometry: Option<Res<'w, render_fx::PreparedFxModelGeometry>>,
    pub(super) atlas: Option<Res<'w, WorldModelLightingAtlas>>,
    pub(super) atpoint: Res<'w, render_scene::DynAtPointLookup>,
    pub(super) lod_skinned: Res<'w, LodRampSkinnedDvar>,
    pub(super) staged_models: ResMut<'w, render_fx::FxModelStaging>,
}

/// Tells the host which effect elements found no light-grid sample, so the gap census counts them.
fn raise_lighting_gaps(host: &mut FxSystemHost, def_indices: &[u8]) {
    for &def_index in def_indices {
        host.gaps.raise(FxGapCause::NoLightGridSample { def_index });
    }
}

/// Rolls the visibility blockers' write buffer into this frame's read buffer.
fn roll_vis_blockers(host: &mut FxSystemHost) {
    let mut write = host.vis_blocker_write;
    let mut read = host.vis_blocker_read;
    fx_t6::runtime::vis_blocker_generate_verts(&mut write, &mut read);
    host.vis_blocker_write = write;
    host.vis_blocker_read = read;
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate_fx_transaction(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut host: ResMut<HostFxSystem>,
    catalog: Option<Res<PreparedFxCatalog>>,
    dvars: Res<FxMarkDvars>,
    cameras: Query<&Transform, With<FlyCamera>>,
    fx_world: FxSceneAccess,
    scene_view: Option<Res<PreparedSceneView>>,
    mut post_lights: ResMut<HostFxPostLights>,
    mut tracers: ResMut<TracerWorld>,
    mark_models: Option<Res<asset_world::MapXModelSceneCatalog>>,
    mark_owners: Query<(
        &crate::prepare::scene::world::WorldScriptModelInstance,
        &Transform,
        &Visibility,
    )>,
    xanims: Option<Res<assets::PreparedXAnims>>,
    mut sensor_lasers: super::super::sensor_laser::SensorLaserQueue,
    mut tag_lasers: super::super::tag_laser::TagLaserQueue,
) -> FxFrameTransaction {
    let present_span = perf::Span::HostFxPresentCpuMs.enter();
    let Some(catalog) = catalog else {
        return FxFrameTransaction {
            outcome: FxFrameOutcome::Unavailable(FxUnavailableCause::CatalogAbsent),
            _present_span: present_span,
        };
    };
    let camera = cameras.iter().next().copied().unwrap_or(Transform::IDENTITY);
    let eye = camera.translation.to_array();

    host.0.generate_world_mark_verts();
    if let (Some(scene), Some(models)) = (fx_world.scene(), mark_models.as_deref()) {
        super::super::entity_mark::generate(
            &mut host.0,
            scene,
            fx_world.marks(),
            &mark_owners,
            xanims.as_deref(),
            models,
        );
    }

    let view = fx_world.view();
    restamp_missing_packed_lighting(&mut host.0, view.as_ref().map(|view| view as &dyn FxScene));
    let frustum_planes = scene_view
        .as_ref()
        .map(|view| view.frustum_planes.to_vec())
        .unwrap_or_default();
    let (out, verts_gaps) = build_fx_verts(
        &mut host.0,
        &catalog.0,
        FxDrawCull {
            elem_draw: dvars.fx_cull_elem_draw,
            planes: &frustum_planes,
        },
        eye,
    );
    raise_lighting_gaps(&mut host.0, &verts_gaps.lighting_frac_def_indices);
    roll_vis_blockers(&mut host.0);

    tick_tracer_beams(&mut tracers, FxMsec(host.0.msec_now));
    sensor_lasers.queue(&mut tracers, host.0.msec_now, eye);
    tag_lasers.queue(&mut tracers, &mut post_lights, eye);

    FxFrameTransaction {
        outcome: FxFrameOutcome::Generated(FxGeneratedFrame {
            out,
            verts_gaps,
            mark_mesh: host.0.last_mark_mesh.take(),
            post_lights: std::mem::take(&mut post_lights.queued),
            tracers: std::mem::take(&mut *tracers),
            cam_tf: camera,
            frustum_planes,
            clip_from_world: scene_view
                .as_ref()
                .map(|view| view.clip_from_world.to_cols_array()),
            tan_half_fov: scene_view
                .as_ref()
                .map(|view| tan_half_fov_from_clip(view.clip_from_view)),
            world_present: fx_world.scene().is_some_and(|scene| scene.spawned),
        }),
        _present_span: present_span,
    }
}
