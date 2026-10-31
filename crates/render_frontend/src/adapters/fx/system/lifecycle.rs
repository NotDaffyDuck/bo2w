//! Effects that start outside the frame's tick: the map's placed one-shots at load, the dust of a
//! sliding player, and the hold that keeps the authority waiting while the map spawns.

use bevy::prelude::*;
use net::{HostLoadingPause, GameActive, NewestTakenSnapshot};
use render_anim::occupancy::t6_body::{T6_SLIDE_DUST_FX, T6SlideDust};
use render_fx::{EffectJournalCursor, JournalCount};

use super::stage::{FxStage, Pose, StageParts};
use render_fx::effect_play::{EffectStage, start_map_placed_effects};
use crate::prepare::scene::world::LevelStage;

/// Whether the authority should keep waiting: a map is being presented but has not spawned yet,
/// or the bots' navigation is not ready. A headless run never presents, so only navigation holds
/// it.
fn authority_should_wait(
    presenting: bool,
    map_spawned: bool,
    navigation_ready: Option<bool>,
) -> bool {
    (presenting && !map_spawned) || navigation_ready == Some(false)
}

pub(super) fn latch_authority_load_hold(
    navigation: Option<Res<frame::BotNavigationReady>>,
    scene: Option<Res<LevelStage>>,
    hold: Option<ResMut<HostLoadingPause>>,
    headless: Option<Res<frame::Headless>>,
) {
    let Some(mut hold) = hold else {
        return;
    };
    let presenting = headless.is_none() && scene.is_some();
    let spawned = scene.is_some_and(|scene| scene.spawned);
    hold.0 = authority_should_wait(presenting, spawned, navigation.map(|ready| ready.0));
}

/// Whether the map's placed one-shots are still to be played, and whether that is now settled for
/// good. A Black Ops II map's scripts are compiled and never run here, so its placed effects always
/// boot from the load's read of them; any other map whose scripts already placed theirs skips the
/// boot.
fn createfx_boot(is_t6_map: bool, scripts_placed_effects: bool) -> CreateFxBoot {
    if !is_t6_map && scripts_placed_effects {
        CreateFxBoot::SkipForGood
    } else {
        CreateFxBoot::Play
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CreateFxBoot {
    SkipForGood,
    Play,
}

pub(super) fn boot_createfx_oneshots(
    emitters: Option<Res<asset_audio::CreateFxOneshotEmitters>>,
    mut stage: FxStage,
    mut cursor: ResMut<EffectJournalCursor>,
    game_active: Res<GameActive>,
    adopted: Option<Res<NewestTakenSnapshot>>,
) {
    if cursor.createfx.done || !game_active.get() {
        return;
    }
    let StageParts { catalog, infos, host, scene } = stage.parts();
    let (Some(fx_catalog), Some(emitter_set)) = (catalog, emitters.as_deref()) else {
        return;
    };
    let scripts_placed_effects = adopted
        .as_ref()
        .and_then(|adopted| adopted.next())
        .is_some_and(|snapshot| snapshot.meta.objectives.scripted_effects);
    let is_t6_map = fx_catalog.0.level_scope() == asset_core::AssetCategoryScope::T6;
    if createfx_boot(is_t6_map, scripts_placed_effects) == CreateFxBoot::SkipForGood {
        cursor.createfx.done = true;
        return;
    }
    let msec = host.0.clock_ms_now;
    cursor.createfx.done = true;
    cursor.createfx.ms_now = Some(msec);
    if emitter_set.0.is_empty() {
        return;
    }

    let scene = scene.as_ref().map(|scene| scene as &dyn EffectStage);
    let tally = start_map_placed_effects(&mut host.0, &fx_catalog.0, infos, &emitter_set.0, msec, scene);
    cursor.add(JournalCount::CreateFxMissedDefs, tally.missing_definition_total);
    for failure in &tally.refused {
        diag::warn!(World, "fx: a map-placed effect did not start: {failure}");
    }
    diag::info!(
        World,
        "fx: map-placed one-shot effects named={} kept={} without a definition={} (only billboard and oriented sprites draw; beams and trails do not yet)",
        tally.by_name,
        tally.kept_count,
        tally.missing_definition_total
    );
}

/// Plays the dust of a player sliding, where the body put it this frame, each time it is due.
pub(super) fn play_t6_slide_dust(
    mut due: MessageReader<T6SlideDust>,
    mut stage: FxStage,
    mut warned: Local<bool>,
) {
    let dust_fx: Vec<_> = due.read().copied().collect();
    if dust_fx.is_empty() {
        return;
    }
    let Some(mut play) = stage.playback() else {
        return;
    };
    if play.defs().resolve_def_for_map(T6_SLIDE_DUST_FX).is_none() {
        if !std::mem::replace(&mut *warned, true) {
            diag::warn!(
                World,
                "bo2mp: the slide's effect {T6_SLIDE_DUST_FX} is not loaded (typed gap)"
            );
        }
        return;
    }
    let name = play.defs().map_fx_name(T6_SLIDE_DUST_FX);
    let msec = play.now();
    for puff in dust_fx {
        let at = Pose {
            anchor: puff.origin,
            turn: puff.axis,
        };
        let _ = play.oriented_at(name, at, msec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authority_waits_for_a_presented_map_to_spawn() {
        assert!(authority_should_wait(true, false, None));
        assert!(!authority_should_wait(true, true, None));
    }

    #[test]
    fn a_run_that_presents_nothing_does_not_wait_for_a_map() {
        assert!(!authority_should_wait(false, false, None));
    }

    #[test]
    fn unready_navigation_always_holds() {
        assert!(authority_should_wait(false, true, Some(false)));
        assert!(!authority_should_wait(false, true, Some(true)));
    }

    #[test]
    fn a_black_ops_map_always_boots_its_placed_effects() {
        assert_eq!(createfx_boot(true, true), CreateFxBoot::Play);
        assert_eq!(createfx_boot(true, false), CreateFxBoot::Play);
    }

    #[test]
    fn another_map_skips_the_boot_only_when_scripts_placed_theirs() {
        assert_eq!(createfx_boot(false, true), CreateFxBoot::SkipForGood);
        assert_eq!(createfx_boot(false, false), CreateFxBoot::Play);
    }
}
