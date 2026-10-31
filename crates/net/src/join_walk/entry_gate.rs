//! The client's admission core and the system that keeps it fed with match facts.

use bevy::prelude::{MessageReader, Res, ResMut, Resource};
use frame::{EntryPassToken, GameSessionArmed, GameSessionStamp, GameSessionClosed};

use crate::match_flow::PlayerRoundCore;
use crate::net_links::lobby_link::{DirectoryLink, DirectoryLinkStatus};
use crate::net_links::relay::PlayerUdpSession;

#[derive(Resource, Clone, Debug, Default)]
pub struct PlayerEntryTicket {
    pub core: PlayerRoundCore,
}

/// The key of the match the master says is running now, or `NONE` outside a match (or before the
/// match has an epoch).
pub fn current_round_key(bridge: Option<&DirectoryLink>) -> GameSessionStamp {
    let Some(state) = bridge.map(DirectoryLink::state) else {
        return GameSessionStamp::NONE;
    };
    live_key_of(&state)
}

fn live_key_of(state: &DirectoryLinkStatus) -> GameSessionStamp {
    if !state.underway() {
        return GameSessionStamp::NONE;
    }
    let who = state.identity();
    if who.epoch == 0 {
        GameSessionStamp::NONE
    } else {
        who.game_session_key()
    }
}

/// The key of whatever session the bridge is in, match or not.
pub(super) fn owner_match_key(bridge: Option<&DirectoryLink>) -> GameSessionStamp {
    bridge.map_or(GameSessionStamp::NONE, |bridge| bridge.state().identity().game_session_key())
}

/// Feed the admission core: teardowns, installs, the master's live match, and — once the link has
/// both entered the match and applied a snapshot — the adopt/enter steps keyed to this connection.
pub fn process_entry_ticket_events(
    mut admission: ResMut<PlayerEntryTicket>,
    mut installed: MessageReader<GameSessionArmed>,
    mut torn: MessageReader<GameSessionClosed>,
    link: Option<Res<PlayerUdpSession>>,
    bridge: Option<Res<DirectoryLink>>,
) {
    for closed in torn.read() {
        admission.core.carry_out_shutdown(*closed);
    }
    for armed in installed.read() {
        admission.core.note_match_announced(armed.decrypt_secret.game_session_key);
        admission.core.note_world_loaded(armed.decrypt_secret);
    }
    let live = current_round_key(bridge.as_deref());
    if !live.is_none() {
        admission.core.note_match_announced(live);
    }
    if let Some(bridge) = bridge.as_deref() {
        bridge.assign_installed_loadout(admission.core.installed());
    }

    let Some(link) = link.as_deref() else {
        return;
    };
    let Some(setup_token) = link.entry_setup_token() else {
        return;
    };
    if !(link.is_inside_match() && link.took_in_world_state()) {
        return;
    }
    let member_id = match bridge.as_deref().map(DirectoryLink::state) {
        Some(DirectoryLinkStatus::Joined { identity: who, .. }) => who.member_id.0,
        _ => [0; 16],
    };
    let key = EntryPassToken {
        game_session_key: admission.core.game_session_key(),
        seat_bytes: member_id,
        link_number: link.connection.map_or(0, |id| id.0),
        opening_number: setup_token,
    };
    for step in [admission.core.note_accepted(key), admission.core.note_match_entered(key)] {
        if let Some(note) = step {
            diag::info!(WireLink, "session change: {note}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_bridge_means_no_live_match() {
        assert!(current_round_key(None).is_none());
        assert!(owner_match_key(None).is_none());
    }

    #[test]
    fn a_default_admission_core_has_nothing_installed() {
        assert!(PlayerEntryTicket::default().core.installed().is_none());
    }
}
