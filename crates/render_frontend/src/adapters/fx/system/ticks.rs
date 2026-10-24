//! The two per-frame effect ticks: the part that does not depend on this frame's poses (windows,
//! effect lights, spawning) and the part that does (bolted effects, movement).

use bevy::prelude::*;
use fx::MarkReceiverEnable;
use net::ClientPredictionState;
use render_fx::{
    FxCameraOrigin, FxMarkDvars, HostFxPostLights, HostFxSystem, PreparedFxCatalog,
    PreparedFxElemInfos,
};

use super::glass_cues::{self, PredictionGlassTrace};
use super::scene_access::FxSceneAccess;
use crate::adapters::fx::present::{FxScene, tick_fx_non_dependent, tick_fx_remaining};
use crate::assemble::drawsurf::T6GlassCues;

/// The prediction world effects collide with, once the client is predicting against a map that
/// has clip data.
fn collision_world(prediction: Option<&ClientPredictionState>) -> Option<&sim::SimWorld> {
    prediction
        .filter(|prediction| prediction.0.is_armed() && prediction.0.world().has_world_clip())
        .map(|prediction| prediction.0.world())
}

fn mark_receivers(dvars: &FxMarkDvars) -> MarkReceiverEnable {
    MarkReceiverEnable {
        fx_marks: dvars.fx_marks,
        fx_marks_ents: dvars.fx_marks_ents,
        fx_marks_smodels: dvars.fx_marks_smodels,
    }
}

pub(super) fn tick_fx_non_dependent_update(
    mut host: ResMut<HostFxSystem>,
    dvars: Res<FxMarkDvars>,
    catalog: Option<Res<PreparedFxCatalog>>,
    mut elem_infos: ResMut<PreparedFxElemInfos>,
    prediction: Option<Res<ClientPredictionState>>,
    camera_origin: Res<FxCameraOrigin>,
    fx_world: FxSceneAccess,
    mut post_lights: ResMut<HostFxPostLights>,
    mut aliases: MessageWriter<audio::AliasCommand>,
    mut glass: ResMut<T6GlassCues>,
) {
    let msec = host.0.msec_now;
    let world = collision_world(prediction.as_deref());

    let trace = world.map(PredictionGlassTrace);
    host.0.glass.advance_in_world(
        msec,
        trace.as_ref().map(|trace| trace as &dyn fx::GlassWorldTrace),
    );

    // A window's own def sounds as it cracks and shatters, at its origin.
    let cues = std::mem::take(&mut glass.0);
    glass_cues::sound_cues(&cues, &mut aliases);

    let Some(catalog) = catalog else {
        return;
    };
    post_lights.queued.clear();
    post_lights.cap_full = 0;
    host.0.mark_receivers = mark_receivers(&dvars);
    let _update = perf::Span::HostFxUpdateCpuMs.enter();

    elem_infos.0.sync(&catalog.0);
    let scene = fx_world.view();
    let scene = scene.as_ref().map(|scene| scene as &dyn FxScene);
    glass_cues::play_shatter_effects(&cues, &mut host.0, &catalog.0, &elem_infos.0, scene);
    tick_fx_non_dependent(
        &mut host.0,
        &catalog.0,
        &elem_infos.0,
        msec,
        camera_origin.0,
        world,
        scene,
    );
}

pub(super) fn tick_fx_remaining_update(
    mut host: ResMut<HostFxSystem>,
    catalog: Option<Res<PreparedFxCatalog>>,
    mut elem_infos: ResMut<PreparedFxElemInfos>,
    prediction: Option<Res<ClientPredictionState>>,
    camera_origin: Res<FxCameraOrigin>,
    fx_world: FxSceneAccess,
    dobj_poses: Option<Res<crate::adapters::anim::dobj_pose::HostDObjPoseFrame>>,
) {
    let Some(catalog) = catalog else {
        return;
    };
    let msec = host.0.msec_now;
    let _update = perf::Span::HostFxUpdateCpuMs.enter();

    host.0.refresh_bolt_poses(|dobj, bone| {
        dobj_poses
            .as_deref()
            .and_then(|poses| poses.resolve_live_bolt(dobj, bone))
    });
    elem_infos.0.sync(&catalog.0);
    let scene = fx_world.view();
    tick_fx_remaining(
        &mut host.0,
        &catalog.0,
        &elem_infos.0,
        msec,
        camera_origin.0,
        collision_world(prediction.as_deref()),
        scene.as_ref().map(|scene| scene as &dyn FxScene),
    );
    perf::Counter::CounterFxElemLive.emit(f64::from(host.0.elem_live_count));
    perf::Counter::CounterFxElemAllocFail.emit(f64::from(host.0.elem_alloc_failures));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_dvars_carry_over_one_for_one() {
        let dvars = FxMarkDvars {
            fx_marks: true,
            fx_marks_ents: false,
            fx_marks_smodels: true,
            ..FxMarkDvars::default()
        };
        let receivers = mark_receivers(&dvars);
        assert!(receivers.fx_marks);
        assert!(!receivers.fx_marks_ents);
        assert!(receivers.fx_marks_smodels);
    }

    #[test]
    fn nothing_collides_without_a_prediction() {
        assert!(collision_world(None).is_none());
    }
}
