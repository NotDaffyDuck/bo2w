//! The pool of HUD elements created by the map scripts, and the choice of which ones each player receives.
//!
//! A script allocates an element, changes it with the host natives, and frees it. This pool only decides the slot an
//! element lives in and, for every client, which elements enter the snapshot banks.

use hud_t6::hudelem::{
    GAME_HUDELEM_CAPACITY, HE_TYPE_FREE, HUDELEM_ALL_CLIENTS, HUDELEM_BANK_CAPACITY,
    HUDELEM_PULSE_SOUND_IDS, HudElem, flags,
};

use crate::world::ClientId;

/// Mask bit for `hud_elem_update_client`: the archival bank, whose elements the killcam replays.
pub const HUDELEM_UPDATE_ARCHIVAL: u8 = 1;

/// Mask bit for `hud_elem_update_client`: the current bank, whose elements only the live view shows.
pub const HUDELEM_UPDATE_CURRENT: u8 = 2;

/// Mask for both banks at once.
pub const HUDELEM_UPDATE_BOTH: u8 = HUDELEM_UPDATE_ARCHIVAL | HUDELEM_UPDATE_CURRENT;

/// One slot of the pool: an element and the audience it is for. A slot holding the free type is empty.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameHudElemSlot {
    pub elem: HudElem,
    /// The single client that sees it, or `HUDELEM_ALL_CLIENTS`.
    pub client_num: i32,
    /// The single team that sees it; 0 means all teams.
    pub team: i32,
    /// Not zero when the killcam replays it.
    pub archived: i32,
}

impl GameHudElemSlot {
    /// Whether the slot holds an element.
    #[must_use]
    pub fn occupied(&self) -> bool {
        self.elem.elem_type != HE_TYPE_FREE
    }

    /// Whether the element is sent to client number `viewer`, who plays on `team`. Debug elements never are.
    fn visible_to(&self, viewer: i32, team: i32) -> bool {
        if !self.occupied() || self.elem.flags & flags::DEBUG != 0 {
            return false;
        }
        let team_ok = self.team == 0 || self.team == team;
        let client_ok = self.client_num == HUDELEM_ALL_CLIENTS || self.client_num == viewer;
        team_ok && client_ok
    }
}

/// Stores a new element, set to the script defaults and archived, in the first empty slot. When no slot is empty the
/// pool grows by one, up to its capacity; past that the result is `None`.
pub fn alloc_hud_elem(pool: &mut Vec<GameHudElemSlot>, client_num: i32, team: i32) -> Option<usize> {
    let slot = GameHudElemSlot {
        elem: HudElem::script_default(),
        client_num,
        team,
        archived: 1,
    };
    if let Some(index) = pool.iter().position(|s| !s.occupied()) {
        pool[index] = slot;
        return Some(index);
    }
    if pool.len() >= GAME_HUDELEM_CAPACITY {
        return None;
    }
    pool.push(slot);
    Some(pool.len() - 1)
}

/// Clears one slot. An index beyond the pool is ignored.
pub fn free_hud_elem(pool: &mut [GameHudElemSlot], index: usize) {
    if let Some(slot) = pool.get_mut(index) {
        *slot = GameHudElemSlot::default();
    }
}

/// Builds the two element banks one client receives, `(archival, current)`, in slot order. A bank never exceeds
/// `HUDELEM_BANK_CAPACITY`, and one that `which` does not ask for stays empty.
#[must_use]
pub fn hud_elem_update_client(
    pool: &[GameHudElemSlot],
    client: ClientId,
    team: i32,
    which: u8,
) -> (Vec<HudElem>, Vec<HudElem>) {
    let viewer = i32::try_from(client.0).unwrap_or(i32::MAX);
    let (mut archival, mut current) = (Vec::new(), Vec::new());
    let visible = pool
        .iter()
        .take(GAME_HUDELEM_CAPACITY)
        .filter(|slot| slot.visible_to(viewer, team));
    for slot in visible {
        let (bank, wanted) = match slot.archived {
            0 => (&mut current, HUDELEM_UPDATE_CURRENT),
            _ => (&mut archival, HUDELEM_UPDATE_ARCHIVAL),
        };
        if which & wanted != 0 && bank.len() < HUDELEM_BANK_CAPACITY {
            bank.push(slot.elem);
        }
    }
    (archival, current)
}

/// Shifts every start time of the elements in a replayed bank by `rebase_ms`, so the killcam sees them on its own
/// clock. Zero means the time was never started, and stays zero.
pub fn rebase_hud_archival(bank: &mut [HudElem], rebase_ms: i32) {
    let moved = |time: &mut i32| {
        if *time != 0 {
            *time = time.saturating_add(rebase_ms);
        }
    };
    for elem in bank {
        moved(&mut elem.time);
        moved(&mut elem.fade_start_time);
        moved(&mut elem.scale_start_time);
        moved(&mut elem.move_start_time);
        moved(&mut elem.font_scale_start_time);
    }
}

/// The number of clients that own a pulse sound counter.
const PULSE_COUNTER_CLIENTS: usize = 64;

/// Per client, the sound id given to the latest pulse-effect element. Ids count from 1 to
/// `HUDELEM_PULSE_SOUND_IDS - 1` and wrap, so elements next to each other sound different.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseFxSoundIds {
    last: [i32; PULSE_COUNTER_CLIENTS],
}

impl Default for PulseFxSoundIds {
    fn default() -> Self {
        Self {
            last: [0; PULSE_COUNTER_CLIENTS],
        }
    }
}

impl PulseFxSoundIds {
    /// The id to give `client` next. A client without a counter always receives 1.
    fn next(&mut self, client: usize) -> i32 {
        let Some(counter) = self.last.get_mut(client) else {
            return 1;
        };
        let advanced = (*counter + 1).rem_euclid(HUDELEM_PULSE_SOUND_IDS);
        *counter = advanced.max(1);
        *counter
    }
}

/// Turns the pulse effect on for a client's element: the start time, the letter speed, the decay timing and the next
/// sound id of that client. The caller has already turned down elements shown to every client.
pub fn set_pulse_fx(
    elem: &mut HudElem,
    client: usize,
    now_ms: i32,
    speed: i32,
    decay_start_time: i32,
    decay_duration: i32,
    ids: &mut PulseFxSoundIds,
) {
    elem.fx_birth_time = now_ms;
    elem.fx_letter_time = speed;
    elem.fx_decay_start_time = decay_start_time;
    elem.fx_decay_duration = decay_duration;
    elem.sound_id = ids.next(client);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool filled from `(client, team, archived)` triples.
    fn filled(rows: &[(i32, i32, i32)]) -> Vec<GameHudElemSlot> {
        let mut pool = Vec::new();
        for &(client, team, archived) in rows {
            let index = alloc_hud_elem(&mut pool, client, team).expect("the pool has room");
            pool[index].archived = archived;
        }
        pool
    }

    fn everyone(team: i32, archived: i32) -> (i32, i32, i32) {
        (HUDELEM_ALL_CLIENTS, team, archived)
    }

    fn started_at(time: i32) -> HudElem {
        HudElem {
            time,
            fade_start_time: time,
            scale_start_time: time,
            move_start_time: time,
            font_scale_start_time: time,
            ..HudElem::default()
        }
    }

    fn bank_sizes(banks: &(Vec<HudElem>, Vec<HudElem>)) -> (usize, usize) {
        (banks.0.len(), banks.1.len())
    }

    #[test]
    fn allocation_reuses_the_first_empty_slot() {
        let mut pool = Vec::new();
        for expected in 0..3 {
            assert_eq!(alloc_hud_elem(&mut pool, 0, 0), Some(expected));
        }
        free_hud_elem(&mut pool, 1);
        assert!(!pool[1].occupied());
        assert_eq!(alloc_hud_elem(&mut pool, 5, 2), Some(1));
        assert_eq!((pool[1].client_num, pool[1].team, pool[1].archived), (5, 2, 1));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn allocation_starts_from_the_script_defaults() {
        let mut pool = Vec::new();
        let index = alloc_hud_elem(&mut pool, 1, 0).unwrap();
        assert_eq!(pool[index].elem, HudElem::script_default());
    }

    #[test]
    fn the_pool_does_not_grow_past_its_capacity() {
        let mut pool = Vec::new();
        for _ in 0..GAME_HUDELEM_CAPACITY {
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
        assert!(pool[0].occupied());
    }

    #[test]
    fn a_client_receives_its_own_elements_and_the_shared_ones() {
        let pool = filled(&[everyone(0, 1), (2, 0, 1), (3, 0, 1)]);
        let banks = hud_elem_update_client(&pool, ClientId(2), 0, HUDELEM_UPDATE_BOTH);
        assert_eq!(bank_sizes(&banks), (2, 0));
    }

    #[test]
    fn team_elements_reach_that_team_only() {
        let pool = filled(&[everyone(1, 1), everyone(2, 1), everyone(0, 1)]);
        let allies = hud_elem_update_client(&pool, ClientId(0), 1, HUDELEM_UPDATE_BOTH);
        let spectators = hud_elem_update_client(&pool, ClientId(0), 3, HUDELEM_UPDATE_BOTH);
        assert_eq!(allies.0.len(), 2);
        assert_eq!(spectators.0.len(), 1);
    }

    #[test]
    fn debug_elements_and_empty_slots_stay_out() {
        let mut pool = filled(&[everyone(0, 1); 3]);
        pool[0].elem.flags |= flags::DEBUG;
        free_hud_elem(&mut pool, 2);
        let banks = hud_elem_update_client(&pool, ClientId(0), 0, HUDELEM_UPDATE_BOTH);
        assert_eq!(banks.0.len(), 1);
    }

    #[test]
    fn the_banks_follow_the_archived_flag_and_the_mask() {
        let pool = filled(&[everyone(0, 1), everyone(0, 0), everyone(0, 0)]);
        let ask = |mask| bank_sizes(&hud_elem_update_client(&pool, ClientId(0), 0, mask));
        assert_eq!(ask(HUDELEM_UPDATE_BOTH), (1, 2));
        assert_eq!(ask(HUDELEM_UPDATE_ARCHIVAL), (1, 0));
        assert_eq!(ask(HUDELEM_UPDATE_CURRENT), (0, 2));
        assert_eq!(ask(0), (0, 0));
    }

    #[test]
    fn a_bank_stops_at_its_capacity() {
        let pool = filled(&vec![everyone(0, 1); HUDELEM_BANK_CAPACITY + 10]);
        let banks = hud_elem_update_client(&pool, ClientId(0), 0, HUDELEM_UPDATE_BOTH);
        assert_eq!(banks.0.len(), HUDELEM_BANK_CAPACITY);
    }

    #[test]
    fn a_bank_keeps_the_slot_order() {
        let mut pool = filled(&[everyone(0, 1); 3]);
        for (rank, slot) in pool.iter_mut().enumerate() {
            slot.elem.sort = rank as f32;
        }
        let banks = hud_elem_update_client(&pool, ClientId(0), 0, HUDELEM_UPDATE_BOTH);
        let sorts: Vec<f32> = banks.0.iter().map(|elem| elem.sort).collect();
        assert_eq!(sorts, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn rebasing_shifts_started_times_and_keeps_zero() {
        let mut bank = [started_at(1000), started_at(0)];
        rebase_hud_archival(&mut bank, -400);
        assert_eq!(bank[0], started_at(600));
        assert_eq!(bank[1], started_at(0));
    }

    #[test]
    fn rebasing_saturates_instead_of_wrapping() {
        let mut late = [started_at(i32::MAX - 5)];
        rebase_hud_archival(&mut late, 100);
        assert_eq!(late[0].time, i32::MAX);
        let mut early = [started_at(i32::MIN + 5)];
        rebase_hud_archival(&mut early, -100);
        assert_eq!(early[0].time, i32::MIN);
    }

    #[test]
    fn pulse_ids_count_up_from_one_and_wrap() {
        let mut ids = PulseFxSoundIds::default();
        let run: Vec<i32> = (0..33).map(|_| ids.next(0)).collect();
        assert_eq!(&run[..2], &[1, 2]);
        assert_eq!(run[30], HUDELEM_PULSE_SOUND_IDS - 1);
        assert_eq!(&run[31..], &[1, 2]);
        assert!(!run.contains(&0));
    }

    #[test]
    fn every_client_has_its_own_pulse_counter() {
        let mut ids = PulseFxSoundIds::default();
        assert_eq!([ids.next(0), ids.next(0), ids.next(1)], [1, 2, 1]);
        assert_eq!(ids.next(PULSE_COUNTER_CLIENTS), 1, "past the counters the id is always 1");
        assert_eq!(ids.next(usize::MAX), 1);
    }

    #[test]
    fn the_pulse_fills_in_the_effect_fields() {
        let mut ids = PulseFxSoundIds::default();
        let mut elem = HudElem::script_default();
        set_pulse_fx(&mut elem, 3, 5000, 70, 200, 900, &mut ids);
        let fields = |e: &HudElem| (e.fx_birth_time, e.fx_letter_time, e.fx_decay_start_time, e.fx_decay_duration, e.sound_id);
        assert_eq!(fields(&elem), (5000, 70, 200, 900, 1));
        set_pulse_fx(&mut elem, 3, 6000, 70, 200, 900, &mut ids);
        assert_eq!((elem.fx_birth_time, elem.sound_id), (6000, 2));
    }
}
