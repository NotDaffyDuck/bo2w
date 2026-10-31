//! The client's counter for numbering its action requests.

use bevy::prelude::Resource;
use sim::IntentSerial;

/// Hands out request ids 1, 2, 3, ... and wraps past `u32::MAX` back to 1: 0 is never used.
#[derive(Resource, Debug)]
pub struct ActionRequestCounter {
    upcoming: IntentSerial,
}

impl Default for ActionRequestCounter {
    fn default() -> Self {
        Self { upcoming: 1 }
    }
}

impl ActionRequestCounter {
    pub fn allocate(&mut self) -> IntentSerial {
        let issued = self.upcoming;
        self.upcoming = issued.wrapping_add(1).max(1);
        issued
    }

    /// Start numbering again from 1, for a new connection.
    pub fn restart(&mut self) {
        self.upcoming = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_count_up_from_one() {
        let mut ids = ActionRequestCounter::default();
        assert_eq!([ids.allocate(), ids.allocate(), ids.allocate()], [1, 2, 3]);
    }

    #[test]
    fn the_counter_skips_zero_when_it_wraps() {
        let mut ids = ActionRequestCounter { upcoming: u32::MAX };
        assert_eq!(ids.allocate(), u32::MAX);
        assert_eq!(ids.allocate(), 1);
        assert_eq!(ids.allocate(), 2);
    }

    #[test]
    fn reset_starts_over() {
        let mut ids = ActionRequestCounter::default();
        ids.allocate();
        ids.allocate();
        ids.restart();
        assert_eq!(ids.allocate(), 1);
    }
}
