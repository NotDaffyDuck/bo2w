//! BO2's server player-corpse table (`g_scr_data.playerCorpseInfo`,
//! `g_player_corpse.cpp`): which slot a body uses, and which body goes
//! when a new one needs room.
//!
//! BO2 keeps 4 player bodies (MW2 kept 8).

/// Length of `g_scr_data.playerCorpseInfo`.
pub const MAX_PLAYER_CORPSES: usize = 4;
/// `corpseInfo_t::entnum` of a free slot.
pub const CORPSE_FREE: i32 = -1;

/// The parts of `corpseInfo_t` the table logic reads (the anim tree and
/// `clientInfo_t` copy live with the renderer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorpseInfo {
    pub entnum: i32,
    /// Level time the body was made.
    pub time: i32,
    pub falling: bool,
}

impl Default for CorpseInfo {
    fn default() -> Self {
        Self { entnum: CORPSE_FREE, time: 0, falling: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerCorpses {
    pub slots: [CorpseInfo; MAX_PLAYER_CORPSES],
}

impl PlayerCorpses {
    /// `G_GetPlayerCorpseIndex`. BO2 prints the table and asserts on a
    /// miss, then uses slot 0; here a miss is `None`.
    pub fn index_of(&self, number: i32) -> Option<usize> {
        self.slots.iter().position(|c| c.entnum == number)
    }

    /// `G_GetFreePlayerCorpseIndex`: the first free slot, else the body
    /// farthest from `player_origin` (the `trBase` of the first entity
    /// with classname `player`; ties keep the lower slot). The second
    /// value is the entity BO2 frees to make room; its slot is cleared.
    pub fn free_index(&mut self, player_origin: [f32; 3], origin_of: impl Fn(i32) -> [f32; 3]) -> (usize, Option<i32>) {
        let mut best = 0;
        let mut best_dist = -1.0f32;
        for (i, c) in self.slots.iter().enumerate() {
            if c.entnum == CORPSE_FREE {
                return (i, None);
            }
            let o = origin_of(c.entnum);
            let dx = player_origin[0] - o[0];
            let dy = player_origin[1] - o[1];
            let dz = player_origin[2] - o[2];
            let dist = dy * dy + dx * dx + dz * dz;
            if best_dist < dist {
                best = i;
                best_dist = dist;
            }
        }
        let freed = self.slots[best].entnum;
        self.slots[best].entnum = CORPSE_FREE;
        (best, Some(freed))
    }

    /// `PlayerCorpse_Free`: the body's slot goes back to free.
    pub fn free(&mut self, number: i32) -> Option<usize> {
        let i = self.index_of(number)?;
        self.slots[i].entnum = CORPSE_FREE;
        Some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_fill_then_the_farthest_goes() {
        let mut t = PlayerCorpses::default();
        let at = |n: i32| [n as f32 * 100.0, 0.0, 0.0];
        for n in [20, 21, 22, 23] {
            let (i, freed) = t.free_index([0.0; 3], at);
            assert_eq!(freed, None);
            t.slots[i].entnum = n;
        }
        assert_eq!(t.index_of(22), Some(2));
        assert_eq!(t.index_of(99), None);
        // Full: entity 23 is farthest from the player.
        assert_eq!(t.free_index([0.0; 3], at), (3, Some(23)));
        assert_eq!(t.slots[3].entnum, CORPSE_FREE);
        assert_eq!(t.free(21), Some(1));
        assert_eq!(t.free(21), None);
    }

    #[test]
    fn a_tie_keeps_the_lower_slot() {
        let mut t = PlayerCorpses::default();
        for (i, c) in t.slots.iter_mut().enumerate() {
            c.entnum = 30 + i as i32;
        }
        assert_eq!(t.free_index([0.0; 3], |_| [10.0, 0.0, 0.0]), (0, Some(30)));
    }
}
