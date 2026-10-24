//! A client that joined someone else's match goes back to the menu when that
//! match ends, when the lobby breaks, or when signon fails, and tells the
//! player why.

use bevy::prelude::*;
use frame::{HasWorld, RuntimeRole, UiMenuDvars};
use net::{MasterBridge, MasterBridgeState, PresentedSnapshot, SignonFailReason, SignonPhase};

use super::SessionSwapRequest;

/// The longest connection notice shown whole; a longer one is cut and points
/// at the log.
const NOTICE_LIMIT: usize = 420;

/// Whether the lobby has joined a match (`Some(in_match)`), or is not a
/// joined lobby at all.
fn joined_in_match(state: Option<&MasterBridgeState>) -> Option<bool> {
    match state {
        Some(MasterBridgeState::Joined { in_match, .. }) => Some(*in_match),
        _ => None,
    }
}

fn lobby_is_over(state: Option<&MasterBridgeState>) -> bool {
    matches!(
        state,
        Some(
            MasterBridgeState::Failed { .. }
                | MasterBridgeState::Closed { .. }
                | MasterBridgeState::Left { .. }
        )
    )
}

/// The match this client was in is over, as the lobby tells it.
fn left_match(was_in_match: bool, in_match: bool) -> bool {
    was_in_match && !in_match
}

/// With no lobby in the way, a client learns the match ended from the
/// server's event journal.
fn journal_has_match_ended(presented: Option<&PresentedSnapshot>) -> bool {
    presented
        .and_then(|presented| presented.snapshot())
        .is_some_and(|snapshot| {
            snapshot
                .meta
                .journal
                .iter()
                .any(|record| matches!(record.event, sim::SimEvent::MatchEnded { .. }))
        })
}

/// Whether the journal is the way this process finds out the match ended: a
/// remote client with a world and no lobby.
fn watches_journal(role: RuntimeRole, in_lobby: bool, has_world: bool) -> bool {
    role == RuntimeRole::Client && !in_lobby && has_world
}

/// Whether the journal just started to say the match ended.
fn match_end_appeared(already_seen: bool, journal_says_ended: bool) -> bool {
    journal_says_ended && !already_seen
}

/// A player-readable version of why a connection ended. Known mismatches say
/// what to do about them; anything else is shown as it came, cut when long.
fn connection_notice(reason: &str) -> String {
    let reason = ["handshake rejected: ", "join admission: "]
        .iter()
        .find_map(|prefix| reason.strip_prefix(prefix))
        .unwrap_or(reason);
    let message = if reason.starts_with("class catalog mismatch") {
        "Your class rules differ from the host's. Both players need the same game build."
    } else if reason.starts_with("map content mismatch") {
        "Your map data differs from the host's. Both players need the same map data to join."
    } else if reason.starts_with("weapon table mismatch") {
        "Your weapon data differs from the host's. Check that both players use the same build and game content."
    } else {
        reason
    };
    if message.chars().count() > NOTICE_LIMIT {
        let kept: String = message.chars().take(NOTICE_LIMIT).collect();
        format!("{kept}…\nSee the log for details.")
    } else {
        message.to_owned()
    }
}

/// Why the lobby or signon ended, in words for the player. `None` for a
/// quiet end (the player left on purpose) or no failure at all.
fn failure_reason(lobby: Option<&MasterBridgeState>, phase: &SignonPhase) -> Option<String> {
    match lobby {
        Some(MasterBridgeState::Failed { error, .. }) => Some(error.source.clone()),
        Some(MasterBridgeState::Closed { reason, .. }) => {
            Some(SignonFailReason::SessionClosed(*reason).to_string())
        }
        Some(MasterBridgeState::Left { .. }) => None,
        _ => match phase {
            SignonPhase::Failed(SignonFailReason::Transport { source, .. }) => Some(source.clone()),
            SignonPhase::Failed(reason) => Some(reason.to_string()),
            _ => None,
        },
    }
}

pub(super) fn run_peer_lobby_return(
    signon: Res<net::SignonState>,
    bridge: Option<Res<MasterBridge>>,
    mut swap: ResMut<SessionSwapRequest>,
    mut dvars: ResMut<UiMenuDvars>,
    mut was_in_match: Local<bool>,
    mut cleaned_up: Local<Option<u64>>,
) {
    let lobby = bridge.as_ref().map(|bridge| bridge.state());
    if signon.phase.is_failed() || lobby_is_over(lobby.as_ref()) {
        let incarnation = bridge.as_ref().map_or(0, |bridge| bridge.incarnation());
        // One return per lobby, however long the failure stays on record.
        if *cleaned_up != Some(incarnation) {
            match swap.request_menu() {
                Ok(id) => {
                    *cleaned_up = Some(incarnation);
                    // The notice lives outside the game: teardown clears the
                    // signon state and the menu stack before the front end
                    // can be used.
                    if let Some(reason) = failure_reason(lobby.as_ref(), &signon.phase) {
                        dvars.set("partyend_reason", connection_notice(&reason));
                        dvars.set("ui_connection_error", "1");
                    }
                    diag::info!(Sim, "session: peer terminal -> lobby (swap #{id})");
                }
                Err(err) => diag::info!(Sim, "session: peer terminal lobby refused - {err}"),
            }
        }
        *was_in_match = false;
        return;
    }
    *cleaned_up = None;
    let Some(in_match) = joined_in_match(lobby.as_ref()) else {
        *was_in_match = false;
        return;
    };
    if left_match(*was_in_match, in_match) {
        match swap.request_menu() {
            Ok(id) => diag::info!(Sim, "session: peer match ended -> lobby (swap #{id})"),
            Err(err) => diag::info!(Sim, "session: peer lobby return refused - {err}"),
        }
    }
    *was_in_match = in_match;
}

pub(super) fn run_udp_peer_lobby_return(
    role: Res<RuntimeRole>,
    presented: Option<Res<PresentedSnapshot>>,
    bridge: Option<Res<MasterBridge>>,
    has_world: Option<Res<HasWorld>>,
    mut swap: ResMut<SessionSwapRequest>,
    mut seen_end: Local<bool>,
) {
    let in_lobby = joined_in_match(bridge.as_ref().map(|bridge| bridge.state()).as_ref()).is_some();
    let has_world = has_world.is_some_and(|world| world.0);
    if !watches_journal(*role, in_lobby, has_world) {
        *seen_end = false;
        return;
    }
    let ended = journal_has_match_ended(presented.as_deref());
    if match_end_appeared(*seen_end, ended) {
        match swap.request_menu() {
            Ok(id) => diag::info!(Sim, "session: udp MatchEnded -> lobby (swap #{id})"),
            Err(err) => diag::info!(Sim, "session: udp lobby return refused - {err}"),
        }
    }
    if ended {
        *seen_end = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_a_match_is_seen_on_the_edge_only() {
        assert!(left_match(true, false));
        assert!(!left_match(true, true));
        assert!(!left_match(false, false));
        assert!(!left_match(false, true));
    }

    #[test]
    fn the_journal_is_watched_by_a_remote_client_with_a_world_and_no_lobby() {
        assert!(watches_journal(RuntimeRole::Client, false, true));
        assert!(!watches_journal(RuntimeRole::Client, true, true));
        assert!(!watches_journal(RuntimeRole::Client, false, false));
        for role in [
            RuntimeRole::Listen,
            RuntimeRole::Dedicated,
            RuntimeRole::Replay,
        ] {
            assert!(!watches_journal(role, false, true));
        }
    }

    #[test]
    fn the_end_of_a_match_is_acted_on_once() {
        assert!(match_end_appeared(false, true));
        assert!(!match_end_appeared(true, true));
        assert!(!match_end_appeared(false, false));
    }

    #[test]
    fn no_lobby_state_is_neither_joined_nor_over() {
        assert_eq!(joined_in_match(None), None);
        assert!(!lobby_is_over(None));
    }

    #[test]
    fn no_snapshot_has_no_match_end() {
        assert!(!journal_has_match_ended(None));
    }

    fn identity() -> net::SessionIdentity {
        net::SessionIdentity::unassigned(1)
    }

    #[test]
    fn a_failed_signon_names_its_transport_error() {
        let phase = SignonPhase::Failed(SignonFailReason::Transport {
            source: "timed out".into(),
            stage: net::FailStage::Transport,
            match_key: frame::MatchKey::NONE,
        });
        assert_eq!(failure_reason(None, &phase).as_deref(), Some("timed out"));
    }

    #[test]
    fn a_lobby_failure_wins_over_the_signon_phase() {
        let lobby = MasterBridgeState::Failed {
            identity: identity(),
            error: net::TransportFault::new("join", "client", "no route"),
        };
        let phase = SignonPhase::Failed(SignonFailReason::Transport {
            source: "timed out".into(),
            stage: net::FailStage::Transport,
            match_key: frame::MatchKey::NONE,
        });
        assert_eq!(
            failure_reason(Some(&lobby), &phase).as_deref(),
            Some("no route")
        );
    }

    #[test]
    fn leaving_on_purpose_gives_no_reason() {
        let lobby = MasterBridgeState::Left {
            identity: identity(),
        };
        assert_eq!(failure_reason(Some(&lobby), &SignonPhase::Local), None);
        assert!(lobby_is_over(Some(&lobby)));
    }

    #[test]
    fn a_phase_that_has_not_failed_has_no_reason() {
        assert_eq!(failure_reason(None, &SignonPhase::Local), None);
    }

    #[test]
    fn known_mismatches_get_a_plain_explanation() {
        assert!(connection_notice("class catalog mismatch: 3 vs 4").contains("class rules"));
        assert!(connection_notice("map content mismatch").contains("map data"));
        assert!(connection_notice("weapon table mismatch").contains("weapon data"));
    }

    #[test]
    fn the_wrapper_text_is_dropped_before_matching() {
        assert!(
            connection_notice("handshake rejected: class catalog mismatch").contains("class rules")
        );
        assert!(connection_notice("join admission: map content mismatch").contains("map data"));
        assert_eq!(connection_notice("handshake rejected: banned"), "banned");
    }

    #[test]
    fn an_unknown_reason_is_shown_as_it_came() {
        assert_eq!(connection_notice("timed out"), "timed out");
    }

    #[test]
    fn a_long_reason_is_cut_and_points_at_the_log() {
        let long = "x".repeat(NOTICE_LIMIT + 50);
        let shown = connection_notice(&long);
        assert!(shown.starts_with(&"x".repeat(NOTICE_LIMIT)));
        assert!(shown.ends_with("See the log for details."));
        assert!(!shown.contains(&"x".repeat(NOTICE_LIMIT + 1)));
    }

    #[test]
    fn a_reason_just_at_the_limit_is_shown_whole() {
        let exact = "y".repeat(NOTICE_LIMIT);
        assert_eq!(connection_notice(&exact), exact);
    }

    #[test]
    fn the_limit_counts_characters_not_bytes() {
        let wide = "é".repeat(NOTICE_LIMIT);
        assert_eq!(connection_notice(&wide), wide);
    }
}
