//! The pool of HUD elements created by the map scripts, and the choice of which ones each player receives.
//!
//! A script allocates an element, changes it with the host natives, and frees it. The pool here only decides the slot
//! an element lives in; [`banks`] picks, for every client, the elements that enter its snapshot banks, and [`pulse`]
//! hands out the sound ids of the pulse effect.

mod banks;
mod pulse;

use hud_t6::overlay_piece::{HUDELEM_ALL_CLIENTS, OverlayPiece, PIECE_UNUSED, SCRIPT_PIECE_POOL, piece_bits};

pub use banks::{
    HUD_SYNC_SAVED_BANK, HUD_SYNC_ALL_BANKS, HUD_SYNC_LIVE_BANK, hud_sync_bank_for_player, shift_replayed_clock,
};
pub use pulse::{PulseFx, PulseEffectSoundSet, set_pulse_fx};

/// The killcam mark of an element the killcam replays.
const KEPT: i32 = 1;

/// One slot of the pool: an element and the audience it is for. A slot holding the free type is empty.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptHudSlot {
    pub elem: OverlayPiece,
    /// The single client that sees it, or `HUDELEM_ALL_CLIENTS`.
    pub seat: i32,
    /// The single team that sees it; 0 means all teams.
    pub side: i32,
    /// Not zero when the killcam replays it.
    pub kept: i32,
}

impl ScriptHudSlot {
    /// Whether the slot holds an element.
    #[must_use]
    pub fn in_use(&self) -> bool {
        let vacant = self.elem.piece_kind == PIECE_UNUSED;
        !vacant
    }

    /// Whether the element is sent to client number `viewer`, who plays on `team`. Debug elements never are.
    fn visible_to(&self, viewer: i32, team: i32) -> bool {
        let shown = self.in_use() && self.elem.bits & piece_bits::DEBUG == 0;
        let for_team = self.side == 0 || self.side == team;
        let for_client = self.seat == HUDELEM_ALL_CLIENTS || self.seat == viewer;
        shown && for_team && for_client
    }
}

/// Stores a new element, set to the script defaults and archived, in the first empty slot. When no slot is empty the
/// pool grows by one, up to its capacity; past that the result is `None`.
pub fn alloc_hud_elem(pool: &mut Vec<ScriptHudSlot>, audience: i32, team: i32) -> Option<usize> {
    let fresh = ScriptHudSlot { kept: KEPT, side: team, seat: audience, elem: OverlayPiece::script_default() };
    if let Some(vacant) = pool.iter().position(|slot| !slot.in_use()) {
        pool[vacant] = fresh;
        return Some(vacant);
    }
    if SCRIPT_PIECE_POOL <= pool.len() {
        return None;
    }
    pool.push(fresh);
    Some(pool.len() - 1)
}

/// Clears one slot. An index beyond the pool is ignored.
pub fn free_hud_elem(pool: &mut [ScriptHudSlot], at: usize) {
    if let Some(slot) = pool.get_mut(at) {
        *slot = ScriptHudSlot::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool filled from `(client, team, archived)` triples.
    pub(super) fn filled(rows: &[(i32, i32, i32)]) -> Vec<ScriptHudSlot> {
        let mut pool = Vec::new();
        for &(seen_by, side, kept) in rows {
            let index = alloc_hud_elem(&mut pool, seen_by, side).expect("the pool has room");
            pool[index].kept = kept;
        }
        pool
    }

    #[test]
    fn allocation_reuses_the_first_empty_slot() {
        let mut pool = Vec::new();
        for expected in 0..3 {
            assert_eq!(alloc_hud_elem(&mut pool, 0, 0), Some(expected));
        }
        free_hud_elem(&mut pool, 1);
        assert!(!pool[1].in_use());
        assert_eq!(alloc_hud_elem(&mut pool, 5, 2), Some(1));
        assert_eq!((pool[1].seat, pool[1].side, pool[1].kept), (5, 2, 1));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn allocation_starts_from_the_script_defaults() {
        let mut pool = Vec::new();
        let index = alloc_hud_elem(&mut pool, 1, 0).unwrap();
        assert_eq!(pool[index].elem, OverlayPiece::script_default());
    }

    #[test]
    fn the_pool_does_not_grow_past_its_capacity() {
        let mut pool = Vec::new();
        for _ in 0..SCRIPT_PIECE_POOL {
            assert!(alloc_hud_elem(&mut pool, 0, 0).is_some());
        }
        assert_eq!(alloc_hud_elem(&mut pool, 0, 0), None);
        free_hud_elem(&mut pool, 700);
        assert_eq!(alloc_hud_elem(&mut pool, 0, 0), Some(700));
    }

    #[test]
    fn freeing_an_index_outside_the_pool_changes_nothing() {
        let mut pool = filled(&[(0, 0, 1)]);
        free_hud_elem(&mut pool, 9);
        assert!(pool[0].in_use());
    }
}
