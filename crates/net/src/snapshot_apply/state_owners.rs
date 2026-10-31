//! Who owns each kind of client state, and how a new snapshot corrects it.

/// Who is the source of truth for a piece of state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateCategory {
    /// The server's snapshot decides it.
    HostDecided,
    /// The client guesses it ahead of the server.
    Predicted,
    /// Only seen, never fed back into the simulation.
    DisplayOnly,
    /// Known not to be covered yet.
    AnnouncedSkip,
}

/// What happens to a piece of state when a snapshot lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrectionStrategy {
    TakeWorldState,
    ReapplyQueuedInputs,
    KeepBeyondSimulation,
    StoreOrderedFrames,
    HonorAnnouncedSkip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRuleEntry {
    pub state_name: &'static str,
    pub class: StateCategory,
    pub correction: CorrectionStrategy,
}

const fn row(state_name: &'static str, class: StateCategory, correction: CorrectionStrategy) -> StateRuleEntry {
    StateRuleEntry { state_name, class, correction }
}

pub const STATE_RULES: &[StateRuleEntry] = &[
    row(
        "resolved Snapshot and snapshot-restorable SimWorld state",
        StateCategory::HostDecided,
        CorrectionStrategy::TakeWorldState,
    ),
    row(
        "unacknowledged MoveHistory and predicted local state",
        StateCategory::Predicted,
        CorrectionStrategy::ReapplyQueuedInputs,
    ),
    row(
        "predictedError, smoothing between frames, camera, audio, FX, plus delivery cursors",
        StateCategory::DisplayOnly,
        CorrectionStrategy::KeepBeyondSimulation,
    ),
    row(
        "FrameArchive of resolved authoritative snapshots",
        StateCategory::HostDecided,
        CorrectionStrategy::StoreOrderedFrames,
    ),
    row(
        "SimWorld residue that adoption declares as gaps",
        StateCategory::AnnouncedSkip,
        CorrectionStrategy::HonorAnnouncedSkip,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_for(state_prefix: &str) -> &'static StateRuleEntry {
        STATE_RULES
            .iter()
            .find(|row| row.state_name.starts_with(state_prefix))
            .unwrap_or_else(|| panic!("no row starting {state_prefix:?}"))
    }

    #[test]
    fn every_kind_of_state_has_exactly_one_row() {
        let mut listed: Vec<_> = STATE_RULES.iter().map(|row| row.state_name).collect();
        listed.sort_unstable();
        listed.dedup();
        assert_eq!(listed.len(), STATE_RULES.len());
        assert_eq!(STATE_RULES.len(), 5);
    }

    #[test]
    fn snapshot_state_is_adopted_and_unacked_moves_are_replayed() {
        let adopted = rule_for("resolved Snapshot");
        assert_eq!(adopted.class, StateCategory::HostDecided);
        assert_eq!(adopted.correction, CorrectionStrategy::TakeWorldState);
        let moves = rule_for("unacknowledged MoveHistory");
        assert_eq!(moves.class, StateCategory::Predicted);
        assert_eq!(moves.correction, CorrectionStrategy::ReapplyQueuedInputs);
    }

    #[test]
    fn presentation_state_is_left_alone_and_the_archive_only_grows() {
        assert_eq!(
            rule_for("predictedError").correction,
            CorrectionStrategy::KeepBeyondSimulation
        );
        assert_eq!(rule_for("predictedError").class, StateCategory::DisplayOnly);
        assert_eq!(
            rule_for("FrameArchive").correction,
            CorrectionStrategy::StoreOrderedFrames
        );
    }

    #[test]
    fn the_declared_gaps_row_follows_the_adopt_gaps_list() {
        let gaps = rule_for("SimWorld residue");
        assert_eq!(gaps.class, StateCategory::AnnouncedSkip);
        assert_eq!(gaps.correction, CorrectionStrategy::HonorAnnouncedSkip);
    }
}
