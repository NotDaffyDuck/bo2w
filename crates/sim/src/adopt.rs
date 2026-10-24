//! Putting an authoritative snapshot over a predicting client's world ("adopting" it).
//!
//! A snapshot carries what a client must agree with the host on, not every field the world holds. What an adoption
//! leaves alone, and why that is safe:
//!
//! - **Random streams** (spawn, combat, bot). Snapshots carry only how many numbers each stream has drawn, not where it
//!   is, so a client cannot resume one. It never needs to: every draw feeds a decision the host already made and
//!   replicated (a spawn place, a spread). A client that re-runs a step draws something else and has it overwritten at
//!   the next adoption, which is why shots that depend on spread are settled by the host rather than predicted.
//! - **Shot and projectile id counters.** A predicted shot gets an id the host never issued; the real one arrives with
//!   the snapshot that is adopted over it. Nothing presented may be keyed by a predicted id, and events come from the
//!   adopted snapshot, not from predicted output. A projectile's flight is predicted; only its id waits.
//! - **Lag compensation state** (the pose history, the script-model collision history, the per-shooter sample record).
//!   Only the host resolves damage, so only the host rewinds. A client keeps a history from its own poses and never
//!   reads it.
//! - **The event journal and its counter.** The journal is emptied at the start of every step and republished from the
//!   adopted state; adopting it too would show one-shot events twice, once from the snapshot and once from the re-run.
//! - **The previous command's buttons and angles.** They are not in the snapshot but are not lost: they are on the
//!   previous user command, and the client puts them back on every adoption, a retired or missing acknowledgement
//!   included. Skipping the retired case once made a held sprint button look like a new press on a snapshot that
//!   already had the player sprinting.
//! - **Content** (collision brushes, weapon tables, equipment tables). The same on both sides by construction and
//!   checked by the content digest; adopting it would mean sending the map and the weapon data every tick.
//! - **Match identity** (the root seed and the started flag), fixed when the match is set up.

/// The tally of one adoption.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AdoptReport {
    /// How many players had their state replaced.
    pub players: usize,
    /// How many clients had their match record replaced.
    pub clients: usize,
    /// How many projectiles came over from the snapshot.
    pub projectiles: usize,
    /// How many predicted things the snapshot had no match for and so were dropped.
    pub dropped: usize,
    /// True when the snapshot was made against other content than this world holds.
    pub content_mismatch: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_starts_with_nothing_adopted() {
        let report = AdoptReport::default();
        assert_eq!((report.players, report.clients, report.projectiles, report.dropped), (0, 0, 0, 0));
        assert!(!report.content_mismatch);
    }

    #[test]
    fn two_reports_are_equal_only_when_every_count_is() {
        let base = AdoptReport { players: 2, clients: 2, projectiles: 1, dropped: 0, content_mismatch: false };
        assert_eq!(base, base);
        assert_ne!(base, AdoptReport { dropped: 1, ..base });
        assert_ne!(base, AdoptReport { content_mismatch: true, ..base });
    }
}
