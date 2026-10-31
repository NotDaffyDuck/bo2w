//! The two banks of elements a client receives in its snapshot.

use hud_t6::overlay_piece::{SCRIPT_PIECE_POOL, OverlayPiece, OVERLAY_PIECE_LIMIT};

use super::ScriptHudSlot;
use crate::sim_parts::SeatNo;

/// Mask bit for `hud_sync_bank_for_player`: the archival bank, whose elements the killcam replays.
pub const HUD_SYNC_SAVED_BANK: u8 = 1;

/// Mask bit for `hud_sync_bank_for_player`: the current bank, whose elements only the live view shows.
pub const HUD_SYNC_LIVE_BANK: u8 = 2;

/// Mask for both banks at once.
pub const HUD_SYNC_ALL_BANKS: u8 = HUD_SYNC_SAVED_BANK | HUD_SYNC_LIVE_BANK;

/// Builds the two element banks one client receives, `(archival, current)`, in slot order. A bank never exceeds
/// `OVERLAY_PIECE_LIMIT`, and a bank that `which` does not ask for stays empty.
#[must_use]
pub fn hud_sync_bank_for_player(
    slots: &[ScriptHudSlot],
    audience: SeatNo,
    team: i32,
    which: u8,
) -> (Vec<OverlayPiece>, Vec<OverlayPiece>) {
    let viewer = i32::try_from(audience.0).unwrap_or(i32::MAX);
    let mut replayed = Vec::new();
    let mut shown_now = Vec::new();
    for slot in slots.iter().take(SCRIPT_PIECE_POOL).filter(|slot| slot.visible_to(viewer, team)) {
        let (bank, bit) = match slot.kept {
            0 => (&mut shown_now, HUD_SYNC_LIVE_BANK),
            _ => (&mut replayed, HUD_SYNC_SAVED_BANK),
        };
        if which & bit != 0 && bank.len() < OVERLAY_PIECE_LIMIT {
            bank.push(slot.elem);
        }
    }
    (replayed, shown_now)
}

/// Shifts every start time of the elements in a replayed bank by `clock_shift_millis`, so the killcam sees them on its own
/// clock. A time of zero was never started, and stays zero.
pub fn shift_replayed_clock(bank: &mut [OverlayPiece], clock_shift_millis: i32) {
    fn shift(time: &mut i32, by: i32) {
        if *time != 0 {
            *time = time.saturating_add(by);
        }
    }
    for elem in bank.iter_mut() {
        for time in [
            &mut elem.time,
            &mut elem.fade_from,
            &mut elem.grow_from,
            &mut elem.slide_from,
            &mut elem.text_grow_from,
        ] {
            shift(time, clock_shift_millis);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::filled;
    use super::super::free_hud_elem;
    use super::*;
    use hud_t6::overlay_piece::{HUDELEM_ALL_CLIENTS, piece_bits};

    fn everyone(team: i32, archived: i32) -> (i32, i32, i32) {
        (HUDELEM_ALL_CLIENTS, team, archived)
    }

    fn started_at(time: i32) -> OverlayPiece {
        OverlayPiece {
            time,
            fade_from: time,
            grow_from: time,
            slide_from: time,
            text_grow_from: time,
            ..OverlayPiece::default()
        }
    }

    fn bank_sizes(banks: &(Vec<OverlayPiece>, Vec<OverlayPiece>)) -> (usize, usize) {
        (banks.0.len(), banks.1.len())
    }

    #[test]
    fn a_client_receives_its_own_elements_and_the_shared_ones() {
        let pool = filled(&[everyone(0, 1), (2, 0, 1), (3, 0, 1)]);
        let banks = hud_sync_bank_for_player(&pool, SeatNo(2), 0, HUD_SYNC_ALL_BANKS);
        assert_eq!(bank_sizes(&banks), (2, 0));
    }

    #[test]
    fn team_elements_reach_that_team_only() {
        let pool = filled(&[everyone(1, 1), everyone(2, 1), everyone(0, 1)]);
        let allies = hud_sync_bank_for_player(&pool, SeatNo(0), 1, HUD_SYNC_ALL_BANKS);
        let spectators = hud_sync_bank_for_player(&pool, SeatNo(0), 3, HUD_SYNC_ALL_BANKS);
        assert_eq!(allies.0.len(), 2);
        assert_eq!(spectators.0.len(), 1);
    }

    #[test]
    fn debug_elements_and_empty_slots_stay_out() {
        let mut pool = filled(&[everyone(0, 1); 3]);
        pool[0].elem.bits |= piece_bits::DEBUG;
        free_hud_elem(&mut pool, 2);
        let banks = hud_sync_bank_for_player(&pool, SeatNo(0), 0, HUD_SYNC_ALL_BANKS);
        assert_eq!(banks.0.len(), 1);
    }

    #[test]
    fn the_banks_follow_the_archived_flag_and_the_mask() {
        let pool = filled(&[everyone(0, 1), everyone(0, 0), everyone(0, 0)]);
        let ask = |mask| bank_sizes(&hud_sync_bank_for_player(&pool, SeatNo(0), 0, mask));
        assert_eq!(ask(HUD_SYNC_ALL_BANKS), (1, 2));
        assert_eq!(ask(HUD_SYNC_SAVED_BANK), (1, 0));
        assert_eq!(ask(HUD_SYNC_LIVE_BANK), (0, 2));
        assert_eq!(ask(0), (0, 0));
    }

    #[test]
    fn a_bank_stops_at_its_capacity() {
        let pool = filled(&vec![everyone(0, 1); OVERLAY_PIECE_LIMIT + 10]);
        let banks = hud_sync_bank_for_player(&pool, SeatNo(0), 0, HUD_SYNC_ALL_BANKS);
        assert_eq!(banks.0.len(), OVERLAY_PIECE_LIMIT);
    }

    #[test]
    fn a_bank_keeps_the_slot_order() {
        let mut pool = filled(&[everyone(0, 1); 3]);
        for (rank, slot) in pool.iter_mut().enumerate() {
            slot.elem.sort = rank as f32;
        }
        let banks = hud_sync_bank_for_player(&pool, SeatNo(0), 0, HUD_SYNC_ALL_BANKS);
        let sorts: Vec<f32> = banks.0.iter().map(|elem| elem.sort).collect();
        assert_eq!(sorts, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn rebasing_shifts_started_times_and_keeps_zero() {
        let mut bank = [started_at(1000), started_at(0)];
        shift_replayed_clock(&mut bank, -400);
        assert_eq!(bank[0], started_at(600));
        assert_eq!(bank[1], started_at(0));
    }

    #[test]
    fn rebasing_saturates_instead_of_wrapping() {
        let mut late = [started_at(i32::MAX - 5)];
        shift_replayed_clock(&mut late, 100);
        assert_eq!(late[0].time, i32::MAX);
        let mut early = [started_at(i32::MIN + 5)];
        shift_replayed_clock(&mut early, -100);
        assert_eq!(early[0].time, i32::MIN);
    }
}
