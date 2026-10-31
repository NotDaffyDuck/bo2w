//! The client's game clock: server time as the client sees it, smoothed between snapshots.

mod advance;
mod census;
mod clock;

pub use census::GameJoinCensus;
pub use clock::FrameClock;

use bevy::prelude::Resource;

/// Whether the client is in a running game: it has a world and has received a snapshot.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameActive(pub bool);

impl GameActive {
    pub fn get(self) -> bool {
        self.0
    }

    /// The game becomes active once the world exists and the first snapshot has arrived.
    pub fn once_world_and_snapshot(world_exists: bool, holds_world_state: bool) -> bool {
        world_exists && holds_world_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_is_active_only_with_both_a_world_and_a_snapshot() {
        for (world, snapshot) in [(false, false), (true, false), (false, true)] {
            assert!(!GameActive::once_world_and_snapshot(world, snapshot));
        }
        assert!(GameActive::once_world_and_snapshot(true, true));
    }

    #[test]
    fn the_flag_reads_back() {
        assert!(!GameActive::default().get());
        assert!(GameActive(true).get());
    }
}
