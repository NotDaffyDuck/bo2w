//! The two per-frame effect ticks: the part that does not depend on this frame's poses (windows,
//! effect lights, spawning) and the part that does (bolted effects, movement).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use fx::DecalAcceptToggle;
use net::ClientGuessState;
use render_fx::{
    EffectsEyePosition, EffectStainTunables, PostEffectLightSet, EffectsSystemHandle, MarkSwitch,
    ReadyFxBook, PreparedFxElemTable,
};

use super::glass_cues::{PredictionGlassTrace, sound_cues, play_shatter_effects};
use super::scene_access::EffectStageAccess;
use render_fx::effect_play::{EffectStage, tick_effect_non_dependent, tick_effect_remaining};
use crate::assemble::draw_items::T6GlassCues;

/// The prediction world effects collide with, once the client is predicting against a map that
/// has clip data.
fn collision_world(predicted: Option<&ClientGuessState>) -> Option<&sim::SimWorld> {
    predicted
        .filter(|prediction| prediction.0.is_forecast_live() && prediction.0.world().has_static_collision())
        .map(|prediction| prediction.0.world())
}

fn stain_receivers(dvars: &EffectStainTunables) -> DecalAcceptToggle {
    let on = |switch| dvars.is_on(switch);
    DecalAcceptToggle {
        stains: on(MarkSwitch::Marks),
        stains_on_ents: on(MarkSwitch::Entities),
        stains_on_props: on(MarkSwitch::StaticModels),
    }
}

/// What both ticks read besides the host itself.
#[derive(SystemParam)]
pub(super) struct TickWorld<'w> {
    fx_catalog: Option<Res<'w, ReadyFxBook>>,
    elem_infos: ResMut<'w, PreparedFxElemTable>,
    prediction: Option<Res<'w, ClientGuessState>>,
    camera_origin: Res<'w, EffectsEyePosition>,
    effect_level: EffectStageAccess<'w>,
}

pub(super) fn tick_fx_non_dependent_update(
    mut fx_host: ResMut<EffectsSystemHandle>,
    dvars: Res<EffectStainTunables>,
    mut world: TickWorld,
    mut after_lights: ResMut<PostEffectLightSet>,
    mut aliases: MessageWriter<audio::AliasCommand>,
    mut glass: ResMut<T6GlassCues>,
) {
    let msec = fx_host.0.clock_ms_now;
    let collision = collision_world(world.prediction.as_deref());

    let trace = collision.map(PredictionGlassTrace);
    let trace = trace.as_ref().map(|trace| trace as &dyn fx::GlassWorldTrace);
    fx_host.0.glass.advance_in_world(msec, trace);

    // A window's own def sounds as it cracks and shatters, at its origin.
    let cues = std::mem::take(&mut glass.0);
    sound_cues(&cues, &mut aliases);

    let Some(effect_book) = world.fx_catalog.as_deref() else {
        return;
    };
    after_lights.begin_tick();
    fx_host.0.stain_receivers = stain_receivers(&dvars);
    let _update = perf::Span::ServerEffectsRefreshCpuMillis.enter();

    let infos = &mut world.elem_infos.0;
    infos.restock(&effect_book.0);
    let view = world.effect_level.view();
    let scene = view.as_ref().map(|view| view as &dyn EffectStage);
    play_shatter_effects(&cues, &mut fx_host.0, &effect_book.0, infos, scene);
    tick_effect_non_dependent(
        &mut fx_host.0,
        &effect_book.0,
        infos,
        msec,
        world.camera_origin.0,
        collision,
        scene,
    );
}

pub(super) fn tick_fx_remaining_update(
    mut fx_host: ResMut<EffectsSystemHandle>,
    mut world: TickWorld,
    dobj_poses: Option<Res<crate::adapters::motion::puppet_pose::HostSkeletonFrame>>,
) {
    let Some(effect_book) = world.fx_catalog.as_deref() else {
        return;
    };
    let msec = fx_host.0.clock_ms_now;
    let _update = perf::Span::ServerEffectsRefreshCpuMillis.enter();

    fx_host.0
        .redo_attach_frames(|dobj, bone| dobj_poses.as_deref()?.find_live_mount(dobj, bone));
    let infos = &mut world.elem_infos.0;
    infos.restock(&effect_book.0);
    let view = world.effect_level.view();
    tick_effect_remaining(
        &mut fx_host.0,
        &effect_book.0,
        infos,
        msec,
        world.camera_origin.0,
        collision_world(world.prediction.as_deref()),
        view.as_ref().map(|view| view as &dyn EffectStage),
    );
    let host = &fx_host.0;
    perf::Counter::TallyEffectSlotsInUse.emit(f64::from(host.live_parts));
    perf::Counter::TallyEffectSlotShortage.emit(f64::from(host.part_take_failures));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_dvars_carry_over_one_for_one() {
        let dvars = EffectStainTunables::default()
            .with(MarkSwitch::Marks, true)
            .with(MarkSwitch::Entities, false)
            .with(MarkSwitch::StaticModels, true);
        let receivers = stain_receivers(&dvars);
        assert!(receivers.stains);
        assert!(!receivers.stains_on_ents);
        assert!(receivers.stains_on_props);
    }

    #[test]
    fn nothing_collides_without_a_prediction() {
        assert!(collision_world(None).is_none());
    }
}
