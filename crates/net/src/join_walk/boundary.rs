//! Resetting the network ends when a match is torn down or the master leaves it.

use bevy::prelude::{Local, MessageReader, Res, ResMut};
use frame::{GameSessionStamp, GameSessionClosed};

use super::entry_gate::owner_match_key;
use crate::match_flow::{describe_session_change, teardown_hits_key};
use crate::net_links::lobby_link::DirectoryLink;
use crate::net_links::relay::{HostUdpSwitchboard, PlayerUdpSession};

pub fn step_match_edge_phase(
    mut torn: MessageReader<GameSessionClosed>,
    mut hub: Option<ResMut<HostUdpSwitchboard>>,
    mut link: Option<ResMut<PlayerUdpSession>>,
    bridge: Option<Res<DirectoryLink>>,
    mut was_in_match: Local<bool>,
) {
    let owner_key = owner_match_key(bridge.as_deref());

    // Teardowns of an older match are ignored; the last one for the live match decides.
    let mut ended = None;
    for closed in torn.read() {
        if teardown_hits_key(closed.game_session_key, owner_key) {
            ended = Some(*closed);
        } else {
            let note = describe_session_change(
                closed.game_session_key.stamp_bytes,
                None,
                closed.game_generation,
                None,
                None,
                None,
                None,
                &format!("live={}", owner_key.game_generation),
                "torn",
                "ignored",
                None,
                "outdated world",
            );
            diag::info!(WireLink, "session change: {note}");
        }
    }

    let inside = bridge
        .as_deref()
        .map_or(*was_in_match, |bridge| bridge.state().underway());
    let left_match = *was_in_match && !inside;
    if ended.is_some() || left_match {
        if let Some(hub) = hub.as_deref_mut() {
            hub.clear_for_new_game();
        }
        if let Some(link) = link.as_deref_mut() {
            link.clear_for_new_game();
        }
        if let (Some(fact), Some(bridge)) = (ended, bridge.as_deref())
            && fact.reason.keeps_session()
        {
            bridge.send_match_ended(fact.game_session_key);
        }
    }
    *was_in_match = inside;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::World;

    #[test]
    fn with_nothing_installed_the_boundary_system_is_a_no_op() {
        let mut world = World::new();
        world.init_resource::<bevy::prelude::Messages<GameSessionClosed>>();
        world.run_system_once(step_match_edge_phase).unwrap();
    }

    #[test]
    fn a_stale_key_is_not_a_boundary_for_the_live_match() {
        let mut live = GameSessionStamp::NONE;
        live.game_generation = 3;
        let mut older = GameSessionStamp::NONE;
        older.game_generation = 2;
        assert!(!teardown_hits_key(older, live));
        assert!(teardown_hits_key(live, live));
        assert!(teardown_hits_key(GameSessionStamp::NONE, GameSessionStamp::NONE));
    }
}
