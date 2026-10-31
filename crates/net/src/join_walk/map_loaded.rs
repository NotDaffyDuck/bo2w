//! Telling the master that the local world is installed.
//!
//! A hosting peer reports "host world ready" for its live epoch; every other peer reports
//! "map loaded" for the epoch sign-on is working on, which owes it a bootstrap offer.

use bevy::prelude::{Local, Res};
use frame::LevelPresent;

use super::entry_gate::{PlayerEntryTicket, current_round_key};
use super::defer::{HoldMapLoadedSignal, wall_clock_ms};
use super::state::JoinProgressRecord;
use crate::match_flow::{HostMapLoaded, acknowledge_tagged_scene_loaded};
use crate::net_links::lobby_link::{DirectoryLink, DirectoryLinkStatus};
use crate::{HostLoadingPause, GameSessionSummary};

fn ready_fact(epoch: u32, descriptor: &GameSessionSummary) -> HostMapLoaded {
    let GameSessionSummary { map, weapons, classes } = *descriptor;
    HostMapLoaded { round_era: epoch, map, weapon_digest: weapons, class_digest: classes }
}

fn descriptor_of(ready: &HostMapLoaded) -> GameSessionSummary {
    let &HostMapLoaded { map, weapon_digest: weapons, class_digest: classes, .. } = ready;
    GameSessionSummary { map, weapons, classes }
}

pub fn step_map_ready_phase(
    bridge: Option<Res<DirectoryLink>>,
    descriptor: Option<Res<GameSessionSummary>>,
    level_is_up: Option<Res<LevelPresent>>,
    admission: Option<Res<PlayerEntryTicket>>,
    signon: Res<JoinProgressRecord>,
    hold: Option<Res<HostLoadingPause>>,
    defer: Option<Res<HoldMapLoadedSignal>>,
    mut last_report: Local<Option<(u64, u32)>>,
    mut deferred: Local<bool>,
    mut withheld: Local<bool>,
) {
    let (Some(bridge), Some(descriptor)) = (bridge.as_deref(), descriptor.as_deref()) else {
        return;
    };
    let installed = level_is_up.is_some_and(|world| world.0);
    let live = current_round_key(Some(bridge));
    let installed_key = admission.as_deref().and_then(|admission| admission.core.installed());
    let belongs = installed_key
        .is_some_and(|key| key.issued_for(live) || (live.is_none() && key.game_session_key.is_none()));

    // Each report goes out once per (link incarnation, epoch); `sent` remembers the last one.
    let link_state = bridge.state();
    match link_state {
        DirectoryLinkStatus::Hosting { identity: who, in_match, .. } if in_match && who.epoch != 0 => {
            let paused = hold.is_some_and(|pause| pause.0);
            let acknowledged = acknowledge_tagged_scene_loaded(installed, paused, belongs, ready_fact(who.epoch, descriptor));
            let Some(ready) = acknowledged else {
                return;
            };
            if defer.is_some_and(|gate| gate.holds_at(wall_clock_ms())) {
                // Said once, the first time it holds.
                if !std::mem::replace(&mut *deferred, true) {
                    diag::info!(WireLink, "host world-ready report is being held; the network and control loop keep running");
                }
                return;
            }
            let once = (bridge.incarnation(), who.epoch);
            if *last_report != Some(once) {
                let ticket = installed_key.expect("confirmed installed host world");
                bridge.send_host_scene_loaded(ready.round_era, descriptor_of(&ready), ticket);
                *last_report = Some(once);
            }
        }
        _ => {
            let round = signon.epoch.0;
            let acknowledged = acknowledge_tagged_scene_loaded(installed, false, belongs, ready_fact(round, descriptor));
            let Some(ready) = acknowledged else {
                // The world is on screen but the report is still held back: say which gate holds
                // it, or the client sits in LoadingMap with nothing in the log to read.
                if installed && !std::mem::replace(&mut *withheld, true) {
                    diag::info!(WireLink, "map-loaded report withheld: round={round} belongs={belongs} installed_key={installed_key:?} live={live:?}");
                }
                return;
            };
            *withheld = false;
            let once = (bridge.incarnation(), round);
            if *last_report != Some(once) {
                bridge.send_map_loaded(ready.round_era, descriptor_of(&ready));
                let HostMapLoaded { round_era: done, map, weapon_digest: weapons, class_digest: classes, .. } = ready;
                diag::info!(WireLink, "map-loaded report sent for epoch={} map={:016x} weapons={:016x} classes={:016x}; a bootstrap offer is now due from the host", done, map, weapons, classes);
                *last_report = Some(once);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ready_fact_carries_the_descriptor_digests() {
        let descriptor = GameSessionSummary { map: 1, weapons: 2, classes: 3 };
        let fact = ready_fact(8, &descriptor);
        assert_eq!((fact.round_era, fact.map, fact.weapon_digest, fact.class_digest), (8, 1, 2, 3));
        let back = descriptor_of(&fact);
        assert_eq!((back.map, back.weapons, back.classes), (1, 2, 3));
    }

    #[test]
    fn without_a_bridge_the_system_does_nothing() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = bevy::prelude::World::new();
        world.init_resource::<JoinProgressRecord>();
        world.run_system_once(step_map_ready_phase).unwrap();
    }
}
