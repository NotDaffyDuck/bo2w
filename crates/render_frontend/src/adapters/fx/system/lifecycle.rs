//! Effects that start outside the frame's tick: the map's placed one-shots at load, the dust of a
//! sliding player, and the hold that keeps the authority waiting while the map spawns.

use bevy::prelude::*;
use net::{AuthorityLoadHold, GameActive, LastAdoptedSnapshot};
use render_anim::occupancy::t6_body::{T6_SLIDE_DUST_FX, T6SlideDust};
use render_fx::{FxJournalCursor, HostFxSystem, PreparedFxCatalog, PreparedFxElemInfos};

use super::scene_access::FxSceneAccess;
use crate::adapters::fx::present::{
    FxScene, boot_createfx_effects, play_named_oriented_at_msec,
};
use crate::prepare::scene::world::WorldScene;

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
    scene: Option<Res<WorldScene>>,
    hold: Option<ResMut<AuthorityLoadHold>>,
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
    catalog: Option<Res<PreparedFxCatalog>>,
    mut elem_infos: ResMut<PreparedFxElemInfos>,
    mut host: ResMut<HostFxSystem>,
    mut cursor: ResMut<FxJournalCursor>,
    fx_world: FxSceneAccess,
    game_active: Res<GameActive>,
    adopted: Option<Res<LastAdoptedSnapshot>>,
) {
    if cursor.createfx_booted || !game_active.get() {
        return;
    }
    let Some(catalog) = catalog else {
        return;
    };
    let scripts_placed_effects = adopted
        .as_ref()
        .and_then(|adopted| adopted.next())
        .is_some_and(|snapshot| snapshot.meta.objectives.scripted_effects);
    let is_t6_map = catalog.0.map_namespace() == asset_core::AssetNamespace::T6;
    if createfx_boot(is_t6_map, scripts_placed_effects) == CreateFxBoot::SkipForGood {
        cursor.createfx_booted = true;
        return;
    }
    let Some(emitters) = emitters else {
        return;
    };
    let msec = host.0.msec_now;
    cursor.createfx_booted = true;
    cursor.createfx_boot_msec = Some(msec);
    if emitters.0.is_empty() {
        return;
    }

    elem_infos.0.sync(&catalog.0);
    let scene = fx_world.view();
    let census = boot_createfx_effects(
        &mut host.0,
        &catalog.0,
        &elem_infos.0,
        &emitters.0,
        msec,
        scene.as_ref().map(|scene| scene as &dyn FxScene),
    );
    cursor.createfx_miss = cursor.createfx_miss.saturating_add(census.miss_def);
    for failure in &census.failed {
        diag::warn!(World, "fx: CreateFX spawn_oriented failed — {failure}");
    }
    diag::info!(
        World,
        "fx: CreateFX oneshots — named={} held={} miss_def={} (Billboard/Oriented sprites; Beam/Trail/… still gaps)",
        census.named,
        census.held,
        census.miss_def
    );
}

/// Plays the dust of a player sliding, where the body put it this frame, each time it is due.
pub(super) fn play_t6_slide_dust(
    mut due: MessageReader<T6SlideDust>,
    catalog: Option<Res<PreparedFxCatalog>>,
    mut elem_infos: ResMut<PreparedFxElemInfos>,
    mut host: ResMut<HostFxSystem>,
    fx_world: FxSceneAccess,
    mut warned: Local<bool>,
) {
    let dust: Vec<_> = due.read().copied().collect();
    let Some(catalog) = catalog.filter(|_| !dust.is_empty()) else {
        return;
    };
    if catalog.0.resolve_def_for_map(T6_SLIDE_DUST_FX).is_none() {
        if !*warned {
            *warned = true;
            diag::warn!(
                World,
                "bo2mp: the slide's effect {T6_SLIDE_DUST_FX} is not loaded (typed gap)"
            );
        }
        return;
    }
    elem_infos.0.sync(&catalog.0);
    let scene = fx_world.view();
    let scene = scene.as_ref().map(|scene| scene as &dyn FxScene);
    let name = catalog.0.map_fx_name(T6_SLIDE_DUST_FX);
    let msec = host.0.msec_now;
    for puff in dust {
        let _ = play_named_oriented_at_msec(
            &mut host.0,
            &catalog.0,
            &elem_infos.0,
            name,
            puff.origin,
            puff.axis,
            msec,
            scene,
        );
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
