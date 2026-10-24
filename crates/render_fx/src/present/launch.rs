//! Starting effects: by name, bolted to a model bone, or from the level's createfx
//! list. Every start stamps the light the effect sits in and then settles whatever
//! runners and decals the start queued.

use asset_audio::CreateFxOneshot;
use asset_game::{FxDefinitions, FxName};
use fx::{
    FxBoltTarget, FxPackedLightingSrc, FxPlayPose, FxSystemHost, PlayResult, axis_from_hit_normal,
    play_bolted, play_oriented, spawn_oriented,
};
use fx_t6::runtime::effect_def_needs_lighting_sample;
use game_math::angle_vectors;

use super::catalog::{FxElemInfoCache, Launchable, catalog_lookup};
use super::queued::settle;
use super::{FxCreateFxBoot, FxScene};

/// The start time of a createfx emitter: its delay in seconds, in whole milliseconds,
/// added to `msec_now` with the same wrap-around the clock itself has.
pub(super) fn delayed_msec(msec_now: i32, delay_seconds: f32) -> i32 {
    msec_now.wrapping_add((delay_seconds * 1000.0) as i32)
}

/// Record the light an effect is sitting in, for the effects whose definition asks for it.
///
/// When the scene cannot answer, the effect is marked `Missing` so a later restamp can
/// try again once lighting is available.
fn stamp_lighting(
    host: &mut FxSystemHost,
    handle: u16,
    catalog: &FxDefinitions,
    scene: Option<&dyn FxScene>,
) {
    let Some(live) = host.slot_for_handle(handle) else {
        return;
    };
    let (index, origin) = (live.catalog_index, live.origin);
    let wants_sample = catalog_lookup(catalog, index)
        .is_some_and(|effect| effect_def_needs_lighting_sample(effect.view.flags));
    if !wants_sample {
        return;
    }
    let sampled = scene.and_then(|scene| scene.sample_atpoint_rgb(origin));
    let Some(slot) = host.slot_for_handle_mut(handle) else {
        return;
    };
    match sampled {
        Some(rgb) => {
            slot.packed_lighting = rgb;
            slot.packed_lighting_src = FxPackedLightingSrc::AtPointRgb;
        }
        None => slot.packed_lighting_src = FxPackedLightingSrc::Missing,
    }
}

/// Stamp the lighting of whatever effect `result` started (nothing, when it started none).
pub(super) fn stamped(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn FxScene>,
    result: PlayResult,
) -> PlayResult {
    if let Some(handle) = result.handle() {
        stamp_lighting(host, handle, catalog, scene);
    }
    result
}

/// Play `effect` at a world pose and stamp its lighting. Queued runners and decals are
/// left for the caller to settle.
pub(super) fn play_at(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn FxScene>,
    effect: &Launchable<'_>,
    pose: FxPlayPose,
) -> PlayResult {
    let result = play_oriented(host, effect.request(pose));
    stamped(host, catalog, scene, result)
}

/// Give every live effect that is still waiting for a lighting sample another try.
pub fn restamp_missing_packed_lighting(host: &mut FxSystemHost, scene: Option<&dyn FxScene>) {
    let Some(scene) = scene else {
        return;
    };
    let waiting: Vec<(u16, [f32; 3])> = host
        .live_effects()
        .filter(|effect| effect.packed_lighting_src == FxPackedLightingSrc::Missing)
        .map(|effect| (effect.own_handle, effect.origin))
        .collect();
    for (handle, origin) in waiting {
        let Some(rgb) = scene.sample_atpoint_rgb(origin) else {
            continue;
        };
        if let Some(slot) = host.slot_for_handle_mut(handle) {
            slot.packed_lighting = rgb;
            slot.packed_lighting_src = FxPackedLightingSrc::AtPointRgb;
        }
    }
}

/// The orientation of an effect placed with level-editor angles: its forward axis is
/// the editor's forward vector.
fn axis_from_angles(angles_deg: [f32; 3]) -> [[f32; 3]; 3] {
    let (forward, _, _) = angle_vectors(angles_deg);
    axis_from_hit_normal(forward)
}

/// Spawn the level's createfx emitters at map start and report how many resolved,
/// were held back, or could not start.
pub fn boot_createfx_effects(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    emitters: &[CreateFxOneshot],
    msec_now: i32,
    scene: Option<&dyn FxScene>,
) -> FxCreateFxBoot {
    host.set_msec_now(msec_now);
    let mut census = FxCreateFxBoot::default();
    for shot in emitters {
        let Some(effect) = catalog.resolve_createfx_id(&shot.fxid) else {
            census.miss_def = census.miss_def.saturating_add(1);
            continue;
        };
        census.named = census.named.saturating_add(1);
        let effect = Launchable::new(catalog, cache, effect);
        let pose = FxPlayPose {
            origin: shot.origin_inches,
            axis: axis_from_angles(shot.angles_deg),
            msec: delayed_msec(msec_now, shot.delay),
        };
        let started = spawn_oriented(host, effect.request(pose));
        match stamped(host, catalog, scene, started) {
            PlayResult::Held { .. } => census.held = census.held.saturating_add(1),
            PlayResult::Failed(e) => census.failed.push(format!("{}: {e:?}", effect.effect.name)),
            _ => {}
        }
    }
    settle(host, catalog, cache, scene);
    restamp_missing_packed_lighting(host, scene);
    census
}

/// Play the named effect at the host's current time.
pub fn play_named_oriented_in_world(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    name: FxName<'_>,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    scene: Option<&dyn FxScene>,
) -> Option<PlayResult> {
    let msec = host.msec_now;
    play_named_at_msec(host, catalog, cache, name, origin, axis, msec, scene)
}

/// Play the named effect as of `msec`.
#[allow(clippy::too_many_arguments)]
pub fn play_named_oriented_at_msec(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    name: FxName<'_>,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    msec: i32,
    scene: Option<&dyn FxScene>,
) -> Option<PlayResult> {
    play_named_at_msec(host, catalog, cache, name, origin, axis, msec, scene)
}

#[allow(clippy::too_many_arguments)]
fn play_named_at_msec(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    name: FxName<'_>,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    msec: i32,
    scene: Option<&dyn FxScene>,
) -> Option<PlayResult> {
    let effect = Launchable::named(catalog, cache, name)?;
    let result = play_at(host, catalog, scene, &effect, FxPlayPose { origin, axis, msec });
    settle(host, catalog, cache, scene);
    Some(result)
}

/// Spawn the named effect (placed, not yet released) as of `msec`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_named_oriented_in_world(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    name: FxName<'_>,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    msec: i32,
    scene: Option<&dyn FxScene>,
) -> Option<PlayResult> {
    let effect = Launchable::named(catalog, cache, name)?;
    let started = spawn_oriented(host, effect.request(FxPlayPose { origin, axis, msec }));
    let result = stamped(host, catalog, scene, started);
    settle(host, catalog, cache, scene);
    Some(result)
}

/// Play the named effect bolted to a model bone, starting at the host's current time.
pub fn play_named_bolted_in_world(
    host: &mut FxSystemHost,
    catalog: &FxDefinitions,
    cache: &FxElemInfoCache,
    name: FxName<'_>,
    target: FxBoltTarget,
    scene: Option<&dyn FxScene>,
) -> Option<PlayResult> {
    let effect = Launchable::named(catalog, cache, name)?;
    let pose = FxPlayPose {
        origin: target.orientation.origin,
        axis: target.orientation.axis,
        msec: host.msec_now,
    };
    let started = play_bolted(host, effect.request(pose), target);
    let result = stamped(host, catalog, scene, started);
    settle(host, catalog, cache, scene);
    Some(result)
}
