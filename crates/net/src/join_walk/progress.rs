//! The per-frame system that advances the sign-on state.

use bevy::prelude::{Local, Res, ResMut};
use frame::ProcessDuty;

use super::entry_gate::PlayerEntryTicket;
use super::join_phase::{GameRoundSerial, JoinFailureCause, JoinSequenceStep, join_stage_now};
use super::state::JoinProgressRecord;
use crate::match_flow::{AbortStep, SessionAbortRecord};
use crate::net_links::lobby_link::{DirectoryLink, DirectoryLinkStatus};
use crate::net_links::relay::PlayerUdpSession;

/// The epoch to sign on to: the link's if it has one, else the master's live match, else whatever
/// the state already held.
fn epoch_in_play(
    link: Option<&PlayerUdpSession>,
    bridge: Option<&DirectoryLink>,
    held: GameRoundSerial,
) -> GameRoundSerial {
    let from_link = link
        .map(|link| GameRoundSerial(link.game_generation()))
        .filter(|serial| serial.0 != 0);
    let from_master = || {
        let state = bridge?.state();
        let epoch = state.identity().epoch;
        (state.underway() && epoch != 0).then_some(GameRoundSerial(epoch))
    };
    from_link.or_else(from_master).unwrap_or(held)
}

pub fn step_join_handshake(
    mut signon: ResMut<JoinProgressRecord>,
    mut admission: ResMut<PlayerEntryTicket>,
    role: Res<ProcessDuty>,
    udp: Option<Res<PlayerUdpSession>>,
    bridge: Option<Res<DirectoryLink>>,
    mut seen_link: Local<Option<u64>>,
    mut reported_terminal: Local<Option<u64>>,
) {
    // A new bridge is a new session: forget everything the last one left behind.
    let current = bridge.as_deref().map(DirectoryLink::incarnation);
    if *seen_link != current {
        *seen_link = current;
        *reported_terminal = None;
        *signon = JoinProgressRecord::default();
    }
    if signon.phase.has_failed() {
        return;
    }

    // A bridge reports its terminal state once. `DirectoryLink::fail` cancels the worker but leaves
    // the state on the resource, which outlives the teardown that resets `JoinProgressRecord`; reading it
    // again would fail the next session's fresh signon with the previous one's message.
    let already_reported = current.is_some() && *reported_terminal == current;
    if let Some(bridge) = bridge.as_deref() {
        let master_state = bridge.state();
        match master_state {
            DirectoryLinkStatus::Failed { .. } | DirectoryLinkStatus::Closed { .. }
                if already_reported => {}
            DirectoryLinkStatus::Failed { error: fault, identity: who } => {
                *reported_terminal = current;
                let game_session_key = who.game_session_key();
                let stage = if fault.step_name == "admission" {
                    AbortStep::Admission
                } else {
                    AbortStep::Transport
                };
                admission.core.keep_first_failure(SessionAbortRecord {
                    stage,
                    source: fault.to_string(),
                    game_session_key,
                });
                signon.enter_phase(JoinSequenceStep::Failed(JoinFailureCause::Transport {
                    source: fault.to_string(),
                    stage: AbortStep::Transport,
                    game_session_key,
                }));
                signon.admitted = admission.core.loadout_pick_permitted();
            }
            DirectoryLinkStatus::Closed { reason, .. } => {
                *reported_terminal = current;
                admission.core.finish_shutdown();
                signon.enter_phase(JoinSequenceStep::Failed(JoinFailureCause::SessionEnded(reason)));
                signon.admitted = admission.core.loadout_pick_permitted();
            }
            _ => {
                step_stage(&mut signon, *role, udp.as_deref(), Some(bridge));
            }
        }
        return;
    }
    step_stage(&mut signon, *role, udp.as_deref(), None);
}

fn step_stage(
    signon: &mut JoinProgressRecord,
    role: ProcessDuty,
    link: Option<&PlayerUdpSession>,
    bridge: Option<&DirectoryLink>,
) {
    let epoch = epoch_in_play(link, bridge, signon.epoch);
    signon.enter_phase(join_stage_now(role, link, epoch));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::World;

    fn world(role: ProcessDuty) -> World {
        let mut world = World::new();
        world.insert_resource(role);
        world.init_resource::<JoinProgressRecord>();
        world.init_resource::<PlayerEntryTicket>();
        world
    }

    #[test]
    fn without_a_bridge_or_link_a_client_checks_for_a_match() {
        let mut world = world(ProcessDuty::Client);
        world.run_system_once(step_join_handshake).unwrap();
        let state = world.resource::<JoinProgressRecord>();
        assert_eq!(state.phase, JoinSequenceStep::VerifyingSession { epoch: GameRoundSerial(0) });
        assert!(!state.admitted);
    }

    #[test]
    fn a_listen_host_stays_local() {
        let mut world = world(ProcessDuty::Listen);
        world.run_system_once(step_join_handshake).unwrap();
        assert_eq!(world.resource::<JoinProgressRecord>().phase, JoinSequenceStep::Local);
    }

    #[test]
    fn a_failed_signon_is_not_advanced_again() {
        let mut world = world(ProcessDuty::Client);
        let failed = JoinSequenceStep::Failed(JoinFailureCause::SessionEnded(
            master_protocol::RoomShutdownCause::HostLeft,
        ));
        world.resource_mut::<JoinProgressRecord>().phase = failed.clone();
        // No bridge on either side of the run, so the incarnation does not change and the failure
        // is final.
        world.run_system_once(step_join_handshake).unwrap();
        assert_eq!(world.resource::<JoinProgressRecord>().phase, failed);
    }

    #[test]
    fn the_held_epoch_is_used_when_neither_link_nor_master_has_one() {
        assert_eq!(epoch_in_play(None, None, GameRoundSerial(9)), GameRoundSerial(9));
    }
}
