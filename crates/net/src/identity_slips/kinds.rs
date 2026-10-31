//! The kinds of gap, and the script notifications a gap can name.

use core::fmt;

use diag::gap::Gap;

/// A way entity identity can fail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkDeficiency {
    /// A client or entity could not be given a unique entity number.
    ClientEntityIndex,
    /// Entity state is carried as a vector, not by entity number. Always on; never cleared.
    ObjectStateRecord,
    /// An entity event named an entity that is not there.
    ObjectEventRecipient,
    /// A script notification named a client with no player entity.
    ScenarioAlertRecipient,
}

impl Gap for LinkDeficiency {
    const ALL_KINDS: &'static [LinkDeficiency] = &[
        LinkDeficiency::ClientEntityIndex,
        LinkDeficiency::ObjectStateRecord,
        LinkDeficiency::ObjectEventRecipient,
        LinkDeficiency::ScenarioAlertRecipient,
    ];

    /// The log names, in declaration order.
    fn name(self) -> &'static str {
        const LOG_NAMES: [&str; 4] = [
            "centity-number",
            "entity-state-vector",
            "entity-event-target",
            "script-notify-target",
        ];
        LOG_NAMES[self as usize]
    }

    /// Only the state-vector gap: entity state really is carried as a vector.
    fn still_open(self) -> bool {
        self == Self::ObjectStateRecord
    }
}

/// The script notifications the client sends for a killcam and for spawning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScriptEventKind {
    BeginKillcam,
    SpawnedPlayer,
    AbortKillcam,
    EndedKillcam,
}

/// The script name of a notification is its variant name, which is what the
/// derived `Debug` writes.
impl fmt::Display for ScriptEventKind {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_the_log_names() {
        let names: Vec<_> = LinkDeficiency::ALL_KINDS.iter().map(|gap| gap.name()).collect();
        assert_eq!(
            names,
            ["centity-number", "entity-state-vector", "entity-event-target", "script-notify-target"]
        );
    }

    #[test]
    fn only_the_state_vector_gap_is_standing() {
        let standing: Vec<_> = LinkDeficiency::ALL_KINDS.iter().filter(|gap| gap.still_open()).collect();
        assert_eq!(standing, [&LinkDeficiency::ObjectStateRecord]);
    }

    #[test]
    fn every_gap_has_its_own_index() {
        for (at, gap) in LinkDeficiency::ALL_KINDS.iter().enumerate() {
            assert_eq!(gap.index(), at);
        }
    }

    #[test]
    fn notifications_display_as_their_script_names() {
        let notes = [
            ScriptEventKind::BeginKillcam,
            ScriptEventKind::SpawnedPlayer,
            ScriptEventKind::AbortKillcam,
            ScriptEventKind::EndedKillcam,
        ];
        let shown = notes.map(|note| note.to_string());
        assert_eq!(shown, ["BeginKillcam", "SpawnedPlayer", "AbortKillcam", "EndedKillcam"]);
    }
}
