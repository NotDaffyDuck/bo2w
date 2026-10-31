//! The resource holding the client's current sign-on phase.

use bevy::prelude::Resource;

use super::join_phase::{GameRoundSerial, JoinSequenceStep};

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct JoinProgressRecord {
    pub epoch: GameRoundSerial,
    pub phase: JoinSequenceStep,
    /// Set from the admission core once the client may pick a class.
    pub admitted: bool,
}

impl JoinProgressRecord {
    pub fn can_choose_loadout(&self) -> bool {
        self.admitted
    }

    /// Move to `next`, logging the change once and following the epoch the new phase carries.
    pub(crate) fn enter_phase(&mut self, target: JoinSequenceStep) {
        if self.phase == target {
            return;
        }
        match &target {
            JoinSequenceStep::Failed(reason) => {
                diag::error!(WireLink, "sign-on failed: {reason} (Connect will not be offered again)");
            }
            JoinSequenceStep::InMatch { epoch, slot } => {
                diag::info!(WireLink, "sign-on in match: epoch={} slot={}; class select is now allowed", epoch.0, slot.0);
            }
            other => diag::info!(WireLink, "sign-on step: {other:?}"),
        }
        if let Some(round) = target.epoch() {
            self.epoch = round;
        }
        self.phase = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_walk::JoinFailureCause;
    use master_protocol::RoomShutdownCause;
    use sim::ClientId;

    #[test]
    fn a_fresh_state_is_local_and_not_admitted() {
        let state = JoinProgressRecord::default();
        assert_eq!(state.phase, JoinSequenceStep::Local);
        assert_eq!(state.epoch, GameRoundSerial(0));
        assert!(!state.admitted && !state.can_choose_loadout());
    }

    #[test]
    fn class_selection_follows_the_admitted_flag_not_the_phase() {
        let mut state = JoinProgressRecord::default();
        state.enter_phase(JoinSequenceStep::InMatch {
            epoch: GameRoundSerial(1),
            slot: ClientId(1),
        });
        assert!(!state.can_choose_loadout());
        state.admitted = true;
        assert!(state.can_choose_loadout());
    }

    #[test]
    fn phases_with_an_epoch_move_the_state_epoch() {
        let mut state = JoinProgressRecord::default();
        state.enter_phase(JoinSequenceStep::LoadingMap { epoch: GameRoundSerial(3) });
        assert_eq!(state.epoch, GameRoundSerial(3));
        state.enter_phase(JoinSequenceStep::CatchingUp { epoch: GameRoundSerial(4) });
        assert_eq!(state.epoch, GameRoundSerial(4));
    }

    #[test]
    fn epochless_phases_keep_the_last_epoch() {
        let mut state = JoinProgressRecord::default();
        state.enter_phase(JoinSequenceStep::CatchingUp { epoch: GameRoundSerial(5) });
        state.enter_phase(JoinSequenceStep::Failed(JoinFailureCause::SessionEnded(
            RoomShutdownCause::Overload,
        )));
        assert_eq!(state.epoch, GameRoundSerial(5));
        assert!(state.phase.has_failed());
        state.enter_phase(JoinSequenceStep::Cancelled);
        assert_eq!(state.epoch, GameRoundSerial(5));
    }

    #[test]
    fn setting_the_same_phase_changes_nothing() {
        let mut state = JoinProgressRecord::default();
        state.enter_phase(JoinSequenceStep::VerifyingSession { epoch: GameRoundSerial(2) });
        let before = state.clone();
        state.enter_phase(JoinSequenceStep::VerifyingSession { epoch: GameRoundSerial(2) });
        assert_eq!(state, before);
    }
}
