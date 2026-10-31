//! Starting effects: by name, bolted to a model bone, or from the level's createfx
//! list. Every start stamps the light the effect sits in and then settles whatever
//! runners and decals the start queued.

use asset_audio::SpawnSingleShotEffect;
use asset_game::{FxDefinitions, FxName};
use fx::{
    EffectBoltTarget, BakedShadeSource, EffectStartFrame, EffectSystemHost, PlaybackResult, basis_off_contact_facing,
    start_attached, start_aligned, play_facing,
};
use fx_t6::runtime::effect_def_needs_lighting_sample;
use game_math::euler_basis;

use super::catalog::{EffectElementInfoMemo, Launchable, recipe_lookup};
use super::queued::settle;
use super::{EffectsBringupParams, EffectStage};

/// The start time of a createfx emitter: its delay in seconds, in whole milliseconds,
/// added to `clock_ms_now` with the same wrap-around the clock itself has.
pub(super) fn delayed_msec(clock_ms_now: i32, wait_secs: f32) -> i32 {
    clock_ms_now.wrapping_add((wait_secs * 1000.0) as i32)
}

/// Record the light an effect is sitting in, for the effects whose definition asks for it.
///
/// When the scene cannot answer, the effect is marked `Missing` so a later restamp can
/// try again once lighting is available.
fn stamp_lighting(
    host: &mut EffectSystemHost,
    handle: u16,
    catalog: &FxDefinitions,
    scene: Option<&dyn EffectStage>,
) {
    let Some((index, origin)) = host
        .bin_of_ticket(handle)
        .map(|live| (live.ledger_index, live.where_at))
    else {
        return;
    };
    let needs_light = recipe_lookup(catalog, index)
        .is_some_and(|effect| effect_def_needs_lighting_sample(effect.view.flags));
    if let (true, Some(slot)) = (needs_light, host.bin_of_ticket_editable(handle)) {
        slot.packed_lighting_src = match scene.and_then(|scene| scene.light_at(origin)) {
            Some(tint) => {
                slot.compact_illum = tint;
                BakedShadeSource::AtPointRgb
            }
            None => BakedShadeSource::Missing,
        };
    }
}

/// Stamp the lighting of whatever effect `result` started (nothing, when it started none).
pub(super) fn stamped(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn EffectStage>,
    result: PlaybackResult,
) -> PlaybackResult {
    result
        .handle()
        .inspect(|&handle| stamp_lighting(host, handle, catalog, scene));
    result
}

/// Play `effect` at a world pose and stamp its lighting. Queued runners and decals are
/// left for the caller to settle.
pub(super) fn play_at(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    scene: Option<&dyn EffectStage>,
    effect: &Launchable<'_>,
    pose: EffectStartFrame,
) -> PlaybackResult {
    let result = start_aligned(host, effect.request(pose));
    stamped(host, catalog, scene, result)
}

/// Give every live effect that is still waiting for a lighting sample another try.
pub fn refill_missing_packed_lighting(host: &mut EffectSystemHost, scene: Option<&dyn EffectStage>) {
    let Some(stage) = scene else {
        return;
    };
    let found: Vec<(u16, [u8; 3])> = host
        .running_emitters()
        .filter(|effect| effect.packed_lighting_src == BakedShadeSource::Missing)
        .filter_map(|effect| {
            let rgb = stage.light_at(effect.where_at)?;
            Some((effect.self_ticket, rgb))
        })
        .collect();
    for (ticket, rgb) in found {
        if let Some(slot) = host.bin_of_ticket_editable(ticket) {
            (slot.compact_illum, slot.packed_lighting_src) =
                (rgb, BakedShadeSource::AtPointRgb);
        }
    }
}

/// Spawn the level's createfx emitters at map start and report how many resolved,
/// were held back, or could not start.
pub fn start_map_placed_effects(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    spawners: &[SpawnSingleShotEffect],
    clock_ms_now: i32,
    scene: Option<&dyn EffectStage>,
) -> EffectsBringupParams {
    host.store_effect_clock(clock_ms_now);
    let mut census = EffectsBringupParams::default();
    for fire in spawners {
        match catalog.find_createfx_id(&fire.effect_id) {
            None => census.missing_definition_total = census.missing_definition_total.saturating_add(1),
            Some(def) => {
                census.by_name = census.by_name.saturating_add(1);
                let effect = Launchable::new(catalog, cache, def);
                // Level-editor angles: the effect faces the editor's forward vector.
                let (ahead, _, _) = euler_basis(fire.rotation_degrees);
                let started = play_facing(
                    host,
                    effect.request(EffectStartFrame {
                        anchor: fire.position_in_inches,
                        frame: basis_off_contact_facing(ahead),
                        ms_now: delayed_msec(clock_ms_now, fire.start_after),
                    }),
                );
                match stamped(host, catalog, scene, started) {
                    PlaybackResult::Kept { .. } => census.kept_count = census.kept_count.saturating_add(1),
                    PlaybackResult::Refused(e) => {
                        census.refused.push(format!("{}: {e:?}", effect.effect.name));
                    }
                    _ => {}
                }
            }
        }
    }
    settle(host, catalog, cache, scene);
    refill_missing_packed_lighting(host, scene);
    census
}

/// Play the named effect at the host's current time.
pub fn play_named_oriented_in_level(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    name: FxName<'_>,
    spot: [f32; 3],
    frame: [[f32; 3]; 3],
    scene: Option<&dyn EffectStage>,
) -> Option<PlaybackResult> {
    let msec = host.clock_ms_now;
    play_named_oriented_at_msec(host, catalog, cache, name, spot, frame, msec, scene)
}

/// Play the named effect as of `msec`.
#[allow(clippy::too_many_arguments)]
pub fn play_named_oriented_at_msec(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    name: FxName<'_>,
    spot: [f32; 3],
    basis: [[f32; 3]; 3],
    msec: i32,
    scene: Option<&dyn EffectStage>,
) -> Option<PlaybackResult> {
    launch_named(host, catalog, cache, scene, name, |host, effect| {
        play_at(host, catalog, scene, effect, EffectStartFrame { anchor: spot, frame: basis, ms_now: msec })
    })
}

/// Looks the effect up by name, starts it with `start`, then lets what it queued settle.
fn launch_named(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    scene: Option<&dyn EffectStage>,
    name: FxName<'_>,
    start: impl FnOnce(&mut EffectSystemHost, &Launchable<'_>) -> PlaybackResult,
) -> Option<PlaybackResult> {
    let effect = Launchable::named(catalog, cache, name)?;
    let result = start(host, &effect);
    settle(host, catalog, cache, scene);
    Some(result)
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_named_oriented_in_world(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    name: FxName<'_>,
    spot: [f32; 3],
    basis: [[f32; 3]; 3],
    msec: i32,
    scene: Option<&dyn EffectStage>,
) -> Option<PlaybackResult> {
    launch_named(host, catalog, cache, scene, name, |host, effect| {
        let started = play_facing(host, effect.request(EffectStartFrame { anchor: spot, frame: basis, ms_now: msec }));
        stamped(host, catalog, scene, started)
    })
}

/// Play the named effect bolted to a model bone, starting at the host's current time.
pub fn spawn_named_attached_world_effect(
    host: &mut EffectSystemHost,
    catalog: &FxDefinitions,
    cache: &EffectElementInfoMemo,
    name: FxName<'_>,
    target: EffectBoltTarget,
    scene: Option<&dyn EffectStage>,
) -> Option<PlaybackResult> {
    launch_named(host, catalog, cache, scene, name, |host, effect| {
        let at = target.orientation;
        let pose = EffectStartFrame {
            anchor: at.origin,
            frame: at.axis,
            ms_now: host.clock_ms_now,
        };
        let started = start_attached(host, effect.request(pose), target);
        stamped(host, catalog, scene, started)
    })
}
