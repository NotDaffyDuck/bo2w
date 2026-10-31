//! The moment the client joined the game, noted once.

use bevy::prelude::Resource;

/// Clock readings from the first moment of a game, for the join-time log.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameJoinCensus {
    /// The game clock at the join.
    pub buffered_depth_millis: Option<i32>,
    /// The snapshot tick at the join.
    pub joined_tick: Option<u32>,
    /// Bevy's elapsed time at the join.
    pub engine_elapsed_millis: Option<i32>,
}

impl GameJoinCensus {
    /// Note the join. Only the first call counts until [`Self::clear`].
    pub fn note_join(&mut self, buffered_depth_millis: i32, at_tick: u32, engine_elapsed_millis: i32) {
        if self.buffered_depth_millis.is_some() {
            return;
        }
        *self = Self {
            buffered_depth_millis: Some(buffered_depth_millis),
            joined_tick: Some(at_tick),
            engine_elapsed_millis: Some(engine_elapsed_millis),
        };
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_join_is_kept() {
        let mut census = GameJoinCensus::default();
        census.note_join(100, 2, 30);
        census.note_join(900, 18, 800);
        assert_eq!(
            census,
            GameJoinCensus {
                buffered_depth_millis: Some(100),
                joined_tick: Some(2),
                engine_elapsed_millis: Some(30),
            }
        );
    }

    #[test]
    fn clearing_lets_a_new_join_be_kept() {
        let mut census = GameJoinCensus::default();
        census.note_join(100, 2, 30);
        census.clear();
        assert_eq!(census, GameJoinCensus::default());
        census.note_join(5, 1, 4);
        assert_eq!(census.buffered_depth_millis, Some(5));
    }
}
