use bevy::prelude::Resource;

/// A tally of what the dispatcher did in the most recent frame it ran, read by tests and the
/// census.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledEntityEventTraversal {
    /// Server event records examined.
    pub examined: u32,
    /// Events handed on as a Bevy event, or tallied as unsupported.
    pub handed_on: u32,
    /// Weapon-fire events that belong to the local player.
    pub self_trigger_pull: u32,
    /// Where the record cursor ended up.
    pub observed_up_to: u32,
    /// Events taken from the entities' own rings.
    pub seat_event_triggered: u32,
    /// Kind and entity number of the record dispatched last.
    pub newest_happening: i32,
    pub latest_index: i32,
}

impl InstalledEntityEventTraversal {
    /// Zero the tally for a new frame; only the cursor position is carried over.
    pub(super) fn begin_frame(&mut self, observed_up_to: u32) {
        *self = Self::default();
        self.observed_up_to = observed_up_to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beginning_a_frame_zeroes_every_counter_but_the_cursor() {
        let mut busy = InstalledEntityEventTraversal {
            examined: 6,
            ..Default::default()
        };
        busy.handed_on = 7;
        busy.self_trigger_pull = 8;
        busy.observed_up_to = 9;
        busy.seat_event_triggered = 10;
        (busy.newest_happening, busy.latest_index) = (11, 12);
        busy.begin_frame(123);
        let fresh = InstalledEntityEventTraversal {
            observed_up_to: 123,
            ..Default::default()
        };
        assert_eq!(busy, fresh);
    }
}
