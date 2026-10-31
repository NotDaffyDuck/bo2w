use bevy::prelude::Resource;
use sim::{ClientId, EntityEventRecord, EventSequence, Tick};

/// How far this client has read into the server's stream of event records.
#[derive(Resource, Default, Debug)]
pub struct EntityEventReadPos {
    replaying: bool,
    /// The latest archived record taken (sequence and tick) while a killcam replays the archive.
    archive_mark: Option<(EventSequence, Tick)>,
    live_through: u32,
}

impl EntityEventReadPos {
    pub(super) fn raw_seen_through(&self) -> u32 {
        self.live_through
    }

    pub const fn observed_up_to(&self) -> EventSequence {
        EventSequence(self.live_through)
    }

    /// Record whether the killcam is playing and say whether that changed. A change drops the
    /// archive mark.
    pub(super) fn switch_killcam(&mut self, now_in_killcam: bool) -> bool {
        if self.replaying == now_in_killcam {
            return false;
        }
        self.archive_mark = None;
        self.replaying = now_in_killcam;
        true
    }

    /// Accept an archived record addressed to `local`. Each snapshot repeats the records of the
    /// last temp-event lifetime, so a record counts as a replay only while it lies inside that
    /// window and is not newer than the mark; a tick further back than the window is a rewind.
    pub fn accepts_archived(&mut self, record: &EntityEventRecord, local: ClientId) -> bool {
        let repeated = match self.archive_mark {
            _ if !record.audience.is_visible_to(local) => return false,
            None => false,
            Some((marked, marked_tick)) => {
                let behind_ms = marked_tick.0.saturating_sub(record.tick.0) * sim::SIM_STEP_MILLIS;
                behind_ms <= sim::EVENT_ENTITY_LIFESPAN_MS as u32
                    && !record.sequence.is_newer_than(marked)
            }
        };
        if !repeated {
            self.archive_mark = Some((record.sequence, record.tick));
        }
        !repeated
    }

    /// Accept a live record when it is addressed to `own_peer` and is ahead of everything accepted
    /// so far.
    pub fn takes_record(&mut self, record: &EntityEventRecord, own_peer: ClientId) -> bool {
        let ahead = record.sequence.is_newer_than(self.observed_up_to());
        let fresh = ahead && record.audience.is_visible_to(own_peer);
        if fresh {
            self.live_through = record.sequence.0;
        }
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use entity_t6::EventCode;
    use sim::{EntityEventPayload, EventRecipients};

    const LOCAL: ClientId = ClientId(1);
    const OTHER: ClientId = ClientId(2);

    fn everyone(seq: u32, tick: u32) -> EntityEventRecord {
        EntityEventRecord {
            sequence: EventSequence(seq),
            ..aimed_at(EventRecipients::All, tick)
        }
    }

    fn aimed_at(audience: EventRecipients, tick: u32) -> EntityEventRecord {
        EntityEventRecord {
            sequence: EventSequence(9),
            tick: Tick(tick),
            audience,
            event: EventCode::NOAMMO,
            payload: EntityEventPayload::default(),
        }
    }

    #[test]
    fn toggling_the_killcam_reports_a_change_once_and_clears_the_mark() {
        let mut c = EntityEventReadPos::default();
        let flips: Vec<bool> = [false, true, true, false]
            .map(|on| c.switch_killcam(on))
            .into();
        assert_eq!(flips, [false, true, false, true]);

        // The mark is gone after a flip, so a record seen before is taken again.
        assert!(c.accepts_archived(&everyone(30, 300), LOCAL));
        assert!(!c.accepts_archived(&everyone(30, 300), LOCAL));
        c.switch_killcam(true);
        assert!(c.accepts_archived(&everyone(30, 300), LOCAL));
    }

    #[test]
    fn an_archived_record_older_than_the_window_is_a_rewind() {
        let window = sim::EVENT_ENTITY_LIFESPAN_MS as u32 / sim::SIM_STEP_MILLIS;
        let mut c = EntityEventReadPos::default();
        assert!(c.accepts_archived(&everyone(70, 2000), LOCAL));
        assert!(c.accepts_archived(&everyone(4, 2000 - window - 10), LOCAL));
    }

    #[test]
    fn a_repeated_archived_record_inside_the_window_is_skipped() {
        let mut c = EntityEventReadPos::default();
        let takes: Vec<bool> = [(20, 200), (20, 200), (21, 202)]
            .into_iter()
            .map(|(move_seq, tick)| c.accepts_archived(&everyone(move_seq, tick), LOCAL))
            .collect();
        assert_eq!(takes, [true, false, true]);
    }

    #[test]
    fn sequence_ordering_survives_wrap_around() {
        let mut c = EntityEventReadPos::default();
        let all_taken = [0x7FFF_FFFF, 0xFFFF_FFF0, 5]
            .into_iter()
            .all(|seq| c.takes_record(&everyone(seq, 2), LOCAL));
        assert!(all_taken);
        assert_eq!(c.raw_seen_through(), 5);
    }

    #[test]
    fn records_addressed_elsewhere_leave_the_cursor_alone() {
        let mut c = EntityEventReadPos::default();
        let not_mine = aimed_at(EventRecipients::Client(OTHER), 2);
        let excluded = aimed_at(EventRecipients::AllExcept(LOCAL), 2);
        assert!(!c.takes_record(&not_mine, LOCAL));
        assert!(!c.accepts_archived(&excluded, LOCAL));
        assert_eq!(c.raw_seen_through(), 0);
    }

    #[test]
    fn live_records_must_be_strictly_newer() {
        let mut c = EntityEventReadPos::default();
        let verdicts: Vec<bool> = [7, 7, 6, 8]
            .into_iter()
            .map(|seq| c.takes_record(&everyone(seq, 2), LOCAL))
            .collect();
        assert_eq!(verdicts, [true, false, false, true]);
        assert_eq!(c.observed_up_to(), EventSequence(8));
    }
}
