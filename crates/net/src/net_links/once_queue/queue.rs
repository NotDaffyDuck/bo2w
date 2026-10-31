use sim::{IntentSerial, MatchOccurrence};

use super::row::{ActionRulingNotice, GuaranteedEntry};

/// How many unacknowledged rows a client may be owed before new ones are refused.
pub const GUARANTEED_BACKLOG_LIMIT: usize = 64;

/// Whether sequence number `later` comes after `earlier`, counting around the 16-bit wrap: it does when it
/// is less than half the range ahead.
pub(crate) fn is_newer_seq(later: u16, earlier: u16) -> bool {
    later != earlier && later.wrapping_sub(earlier) < 0x8000
}

/// The rows owed to one client, numbered from 1; 0 is never a sequence number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuaranteedEventBacklog {
    upcoming_serial: u16,
    confirmed_up_to: u16,
    owed: Vec<(u16, GuaranteedEntry)>,
    /// Rows refused because the queue was full.
    pub evicted_stale_count: u32,
}

impl GuaranteedEventBacklog {
    pub fn new() -> Self {
        Self {
            upcoming_serial: 1,
            ..Self::default()
        }
    }

    /// Queue `row` and return its sequence number, or 0 if the queue was full and the row was
    /// not kept.
    pub fn stash_row(&mut self, row: GuaranteedEntry) -> u16 {
        if self.owed.len() >= GUARANTEED_BACKLOG_LIMIT {
            self.evicted_stale_count = self.evicted_stale_count.saturating_add(1);
            return 0;
        }
        // A queue made by `Default` starts at 0, and the counter wraps upto 0.
        let seq = self.upcoming_serial.max(1);
        self.upcoming_serial = seq.wrapping_add(1);
        self.owed.push((seq, row));
        seq
    }

    pub fn post_event(&mut self, event: MatchOccurrence) -> u16 {
        self.stash_row(GuaranteedEntry::Event(event))
    }

    pub fn enqueue_resolution(&mut self, request_id: IntentSerial, verdict: ActionRulingNotice) -> u16 {
        self.stash_row(GuaranteedEntry::CommandResolution {
            request_id,
            verdict,
        })
    }

    /// The client has everything up to and including `upto`. An confirm_receipt that is 0 or behind what
    /// is already receipt_confirmed changes nothing.
    pub fn confirm_receipt(&mut self, upto: u16) {
        if upto == 0 {
            return;
        }
        if is_newer_seq(upto, self.confirmed_up_to) {
            self.confirmed_up_to = upto;
        }
        let receipt_confirmed = self.confirmed_up_to;
        self.owed.retain(|(seq, _)| is_newer_seq(*seq, receipt_confirmed));
    }

    pub fn confirmed_up_to(&self) -> u16 {
        self.confirmed_up_to
    }

    pub fn waiting_count(&self) -> usize {
        self.owed.len()
    }

    pub fn owed_rows(&self) -> &[(u16, GuaranteedEntry)] {
        &self.owed
    }

    /// What goes out in a packet right now.
    pub fn envelope(&self) -> GuaranteedEnvelope {
        GuaranteedEnvelope {
            confirmed_up_to: self.confirmed_up_to,
            entries: self.owed.clone(),
            evicted_stale_count: self.evicted_stale_count,
        }
    }
}

/// The reliable part of one packet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuaranteedEnvelope {
    /// The last of the receiver's own rows this side has seen.
    pub confirmed_up_to: u16,
    pub entries: Vec<(u16, GuaranteedEntry)>,
    pub evicted_stale_count: u32,
}

impl GuaranteedEnvelope {
    /// For a client with no queue: nothing owed and nothing receipt_confirmed.
    pub fn has_no_debt() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(text: &str) -> GuaranteedEntry {
        GuaranteedEntry::Standings(text.to_owned())
    }

    fn filled(count: usize) -> GuaranteedEventBacklog {
        let mut queue = GuaranteedEventBacklog::new();
        for n in 0..count {
            queue.stash_row(scores(&n.to_string()));
        }
        queue
    }

    fn seqs(queue: &GuaranteedEventBacklog) -> Vec<u16> {
        queue.owed_rows().iter().map(|(seq, _)| *seq).collect()
    }

    #[test]
    fn sequence_numbers_start_at_one_and_count_up() {
        let mut queue = GuaranteedEventBacklog::new();
        assert_eq!(queue.stash_row(scores("a")), 1);
        assert_eq!(queue.stash_row(scores("b")), 2);
        assert_eq!(queue.waiting_count(), 2);
    }

    #[test]
    fn a_default_queue_also_starts_at_one() {
        let mut queue = GuaranteedEventBacklog::default();
        assert_eq!(queue.stash_row(scores("a")), 1);
    }

    #[test]
    fn the_counter_skips_zero_when_it_wraps() {
        let mut queue = GuaranteedEventBacklog::new();
        queue.upcoming_serial = u16::MAX;
        assert_eq!(queue.stash_row(scores("last")), u16::MAX);
        assert_eq!(queue.stash_row(scores("wrapped")), 1);
    }

    #[test]
    fn a_full_queue_refuses_the_row_and_counts_it() {
        let mut queue = filled(GUARANTEED_BACKLOG_LIMIT);
        assert_eq!(queue.evicted_stale_count, 0);
        assert_eq!(queue.stash_row(scores("one too many")), 0);
        assert_eq!(queue.stash_row(scores("and another")), 0);
        assert_eq!(queue.waiting_count(), GUARANTEED_BACKLOG_LIMIT);
        assert_eq!(queue.evicted_stale_count, 2);
    }

    #[test]
    fn an_ack_releases_everything_up_to_it() {
        let mut queue = filled(5);
        queue.confirm_receipt(3);
        assert_eq!(seqs(&queue), [4, 5]);
        assert_eq!(queue.confirmed_up_to(), 3);
    }

    #[test]
    fn an_ack_of_zero_or_of_something_old_changes_nothing() {
        let mut queue = filled(5);
        queue.confirm_receipt(3);
        queue.confirm_receipt(0);
        queue.confirm_receipt(2);
        assert_eq!(queue.confirmed_up_to(), 3);
        assert_eq!(seqs(&queue), [4, 5]);
    }

    #[test]
    fn room_opens_up_again_once_rows_are_acked() {
        let mut queue = filled(GUARANTEED_BACKLOG_LIMIT);
        queue.confirm_receipt(10);
        assert_eq!(queue.stash_row(scores("fits")), GUARANTEED_BACKLOG_LIMIT as u16 + 1);
    }

    #[test]
    fn acks_compare_around_the_wrap() {
        assert!(is_newer_seq(2, 1));
        assert!(!is_newer_seq(1, 2));
        assert!(!is_newer_seq(5, 5));
        assert!(is_newer_seq(3, u16::MAX));
        assert!(!is_newer_seq(u16::MAX, 3));
        assert!(is_newer_seq(0x8000, 1));
        assert!(!is_newer_seq(0x8001, 1));
    }

    #[test]
    fn an_ack_across_the_wrap_releases_the_rows_before_it() {
        let mut queue = GuaranteedEventBacklog::new();
        queue.upcoming_serial = u16::MAX - 1;
        for _ in 0..4 {
            queue.stash_row(scores("x"));
        }
        assert_eq!(seqs(&queue), [u16::MAX - 1, u16::MAX, 1, 2]);
        queue.confirm_receipt(1);
        assert_eq!(seqs(&queue), [2]);
    }

    #[test]
    fn the_payload_carries_the_ack_the_rows_and_the_drop_count() {
        let mut queue = filled(GUARANTEED_BACKLOG_LIMIT);
        queue.stash_row(scores("refused"));
        queue.confirm_receipt(60);
        let payload = queue.envelope();
        assert_eq!(payload.confirmed_up_to, 60);
        assert_eq!(payload.entries.len(), 4);
        assert_eq!(payload.evicted_stale_count, 1);
    }

    #[test]
    fn a_payload_that_owes_nothing_is_empty() {
        let payload = GuaranteedEnvelope::has_no_debt();
        assert_eq!(payload.confirmed_up_to, 0);
        assert!(payload.entries.is_empty());
        assert_eq!(payload.evicted_stale_count, 0);
    }
}
