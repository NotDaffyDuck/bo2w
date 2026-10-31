//! The sign-on phases and the rules that pick one.

use frame::{GameSessionStamp, ProcessDuty};
use master_protocol::RoomShutdownCause;
use sim::ClientId;

use crate::match_flow::AbortStep;
use crate::net_links::datagrams::JoinRefusal;
use crate::net_links::relay::PlayerUdpSession;

/// Which match of the session the client is signing on to; 0 means none yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct GameRoundSerial(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinFailureCause {
    Handshake(JoinRefusal),
    Transport {
        source: String,
        stage: AbortStep,
        game_session_key: GameSessionStamp,
    },
    SessionEnded(RoomShutdownCause),
}

impl core::fmt::Display for JoinFailureCause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Handshake(cause_note) => write!(f, "handshake rejected: {cause_note}"),
            Self::Transport {
                source,
                stage,
                game_session_key,
            } => write!(
                f,
                "{source} during {stage:?}, round {}",
                game_session_key.game_generation
            ),
            Self::SessionEnded(RoomShutdownCause::HostLeft) => f.write_str("the host left the session"),
            Self::SessionEnded(RoomShutdownCause::TenancyLapsed) => {
                f.write_str("session lease ran out")
            }
            Self::SessionEnded(RoomShutdownCause::Overload) => {
                f.write_str("session closed: control channel overloaded")
            }
            Self::SessionEnded(RoomShutdownCause::Kicked) => f.write_str("kicked from the party"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum JoinSequenceStep {
    /// Not a client of anybody else's match.
    #[default]
    Local,
    /// Looking for the host; no handshake accepted yet.
    VerifyingSession { epoch: GameRoundSerial },
    /// The host's match is known and the map is loading here.
    LoadingMap { epoch: GameRoundSerial },
    /// Handshake accepted; waiting for the bootstrap and the first snapshot.
    CatchingUp { epoch: GameRoundSerial },
    /// In the match with a client slot.
    InMatch { epoch: GameRoundSerial, slot: ClientId },
    Failed(JoinFailureCause),
    Cancelled,
}

impl JoinSequenceStep {
    pub const fn can_choose_loadout(&self) -> bool {
        matches!(*self, Self::InMatch { .. })
    }

    pub const fn can_propose_link(&self) -> bool {
        matches!(*self, Self::VerifyingSession { epoch: _ })
    }

    pub const fn has_failed(&self) -> bool {
        matches!(*self, Self::Failed(ref _why))
    }

    /// The epoch carried by the phases that have one.
    pub(super) const fn epoch(&self) -> Option<GameRoundSerial> {
        match self {
            Self::VerifyingSession { epoch }
            | Self::LoadingMap { epoch }
            | Self::CatchingUp { epoch }
            | Self::InMatch { epoch, .. } => Some(*epoch),
            Self::Local | Self::Failed(_) | Self::Cancelled => None,
        }
    }

    /// What the connect screen shows while the phase is in progress.
    pub fn summary_banner(&self) -> Option<String> {
        let text = match self {
            Self::VerifyingSession { .. } => "waiting for host: handshake not accepted".to_owned(),
            Self::LoadingMap { .. } => "waiting for host: loading map".to_owned(),
            Self::CatchingUp { .. } => {
                "waiting for host: handshake accepted, applying bootstrap".to_owned()
            }
            Self::Failed(reason) => reason.to_string(),
            Self::Cancelled => "signon cancelled".to_owned(),
            Self::Local | Self::InMatch { .. } => return None,
        };
        Some(text)
    }
}

/// The phase a live UDP link says we are in.
pub fn stage_of_link(link: &PlayerUdpSession, epoch: GameRoundSerial) -> JoinSequenceStep {
    if let Some(reason) = link.greeting_refusal() {
        return JoinSequenceStep::Failed(JoinFailureCause::Handshake(reason));
    }
    if link.connection.is_none() {
        return JoinSequenceStep::VerifyingSession { epoch };
    }
    let snapshot = link.took_in_world_state();
    if !snapshot {
        return JoinSequenceStep::CatchingUp { epoch };
    }
    match (link.granted_player_slot, link.is_inside_match()) {
        (Some(slot), true) => JoinSequenceStep::InMatch { epoch, slot },
        (Some(_), false) => JoinSequenceStep::CatchingUp { epoch },
        (None, _) => JoinSequenceStep::VerifyingSession { epoch },
    }
}

/// The phase for this frame: only clients sign on; with no link the epoch says whether the map is
/// the thing being waited for.
pub fn join_stage_now(
    role: ProcessDuty,
    link: Option<&PlayerUdpSession>,
    epoch: GameRoundSerial,
) -> JoinSequenceStep {
    if role != ProcessDuty::Client {
        return JoinSequenceStep::Local;
    }
    match link {
        Some(link) => stage_of_link(link, epoch),
        None if epoch != GameRoundSerial(0) => JoinSequenceStep::LoadingMap { epoch },
        None => JoinSequenceStep::VerifyingSession { epoch },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: GameRoundSerial = GameRoundSerial(4);

    #[test]
    fn only_a_client_signs_on() {
        for role in [ProcessDuty::Dedicated, ProcessDuty::Listen] {
            assert_eq!(join_stage_now(role, None, EPOCH), JoinSequenceStep::Local);
        }
    }

    #[test]
    fn without_a_link_the_epoch_decides_between_loading_and_checking() {
        assert_eq!(
            join_stage_now(ProcessDuty::Client, None, GameRoundSerial(0)),
            JoinSequenceStep::VerifyingSession { epoch: GameRoundSerial(0) }
        );
        assert_eq!(
            join_stage_now(ProcessDuty::Client, None, EPOCH),
            JoinSequenceStep::LoadingMap { epoch: EPOCH }
        );
    }

    #[test]
    fn only_active_selects_classes_and_only_checking_offers_connect() {
        let active = JoinSequenceStep::InMatch {
            epoch: EPOCH,
            slot: ClientId(2),
        };
        let all = [
            JoinSequenceStep::Local,
            JoinSequenceStep::VerifyingSession { epoch: EPOCH },
            JoinSequenceStep::LoadingMap { epoch: EPOCH },
            JoinSequenceStep::CatchingUp { epoch: EPOCH },
            active.clone(),
            JoinSequenceStep::Failed(JoinFailureCause::SessionEnded(RoomShutdownCause::HostLeft)),
            JoinSequenceStep::Cancelled,
        ];
        for phase in &all {
            assert_eq!(phase.can_choose_loadout(), *phase == active);
            assert_eq!(
                phase.can_propose_link(),
                matches!(phase, JoinSequenceStep::VerifyingSession { .. })
            );
            assert_eq!(phase.has_failed(), matches!(phase, JoinSequenceStep::Failed(_)));
        }
    }

    #[test]
    fn the_epoch_is_carried_by_the_four_live_phases() {
        assert_eq!(JoinSequenceStep::Local.epoch(), None);
        assert_eq!(JoinSequenceStep::Cancelled.epoch(), None);
        assert_eq!(JoinSequenceStep::LoadingMap { epoch: EPOCH }.epoch(), Some(EPOCH));
        let active = JoinSequenceStep::InMatch {
            epoch: EPOCH,
            slot: ClientId(1),
        };
        assert_eq!(active.epoch(), Some(EPOCH));
    }

    #[test]
    fn status_lines_name_what_the_client_is_waiting_for() {
        let line = |phase: JoinSequenceStep| phase.summary_banner();
        assert_eq!(line(JoinSequenceStep::Local), None);
        assert_eq!(
            line(JoinSequenceStep::InMatch {
                epoch: EPOCH,
                slot: ClientId(1)
            }),
            None
        );
        assert_eq!(
            line(JoinSequenceStep::VerifyingSession { epoch: EPOCH }).as_deref(),
            Some("waiting for host: handshake not accepted")
        );
        assert_eq!(
            line(JoinSequenceStep::LoadingMap { epoch: EPOCH }).as_deref(),
            Some("waiting for host: loading map")
        );
        assert_eq!(
            line(JoinSequenceStep::CatchingUp { epoch: EPOCH }).as_deref(),
            Some("waiting for host: handshake accepted, applying bootstrap")
        );
        assert_eq!(
            line(JoinSequenceStep::Cancelled).as_deref(),
            Some("signon cancelled")
        );
    }

    #[test]
    fn failure_text_reaches_the_status_line() {
        let closed = JoinSequenceStep::Failed(JoinFailureCause::SessionEnded(
            RoomShutdownCause::TenancyLapsed,
        ));
        assert_eq!(closed.summary_banner().as_deref(), Some("session lease ran out"));
        let rejected = JoinFailureCause::Handshake(JoinRefusal::Limits);
        assert!(rejected.to_string().starts_with("handshake rejected: "));
    }

    #[test]
    fn close_reasons_have_their_own_wording() {
        let say = |reason| JoinFailureCause::SessionEnded(reason).to_string();
        assert_eq!(say(RoomShutdownCause::HostLeft), "the host left the session");
        assert_eq!(say(RoomShutdownCause::TenancyLapsed), "session lease ran out");
        assert_eq!(say(RoomShutdownCause::Overload), "session closed: control channel overloaded");
    }

    #[test]
    fn a_transport_failure_names_its_stage_and_epoch() {
        let mut game_session_key = GameSessionStamp::NONE;
        game_session_key.game_generation = 7;
        let reason = JoinFailureCause::Transport {
            source: "socket closed".to_owned(),
            stage: AbortStep::Transport,
            game_session_key,
        };
        assert_eq!(reason.to_string(), "socket closed during Transport, round 7");
    }
}
