use hud_t6::hudelem::{
    GAME_HUDELEM_CAPACITY, HE_TYPE_FREE, HUDELEM_ALL_CLIENTS, HUDELEM_BANK_CAPACITY,
    HUDELEM_PULSE_SOUND_IDS, HudElem, flags,
};

use crate::world::ClientId;

pub const HUDELEM_UPDATE_ARCHIVAL: u8 = 1;

pub const HUDELEM_UPDATE_CURRENT: u8 = 2;

pub const HUDELEM_UPDATE_BOTH: u8 = HUDELEM_UPDATE_ARCHIVAL | HUDELEM_UPDATE_CURRENT;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameHudElemSlot {
    pub elem: HudElem,

    pub client_num: i32,

    pub team: i32,

    pub archived: i32,
}

impl GameHudElemSlot {
    #[must_use]
    pub fn occupied(&self) -> bool {
        self.elem.elem_type != HE_TYPE_FREE
    }
}

/// HudElem_Alloc: the lowest free slot, set to HudElem_SetDefaults (archived).
pub fn alloc_hud_elem(
    pool: &mut Vec<GameHudElemSlot>,
    client_num: i32,
    team: i32,
) -> Option<usize> {
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

pub fn free_hud_elem(pool: &mut [GameHudElemSlot], index: usize) {
    if let Some(slot) = pool.get_mut(index) {
        *slot = GameHudElemSlot::default();
    }
}

/// HudElem_UpdateClient: in slot order, every live non-debug elem for this client's team (or
/// every team) and for this client (or every client), archived ones to the archival bank and the
/// rest to the current bank, 31 each.
#[must_use]
pub fn hud_elem_update_client(
    pool: &[GameHudElemSlot],
    client: ClientId,
    team: i32,
    which: u8,
) -> (Vec<HudElem>, Vec<HudElem>) {
    let mut archival = Vec::new();
    let mut current = Vec::new();
    let client_num = i32::try_from(client.0).unwrap_or(i32::MAX);
    for slot in pool.iter().take(GAME_HUDELEM_CAPACITY) {
        if !slot.occupied() || slot.elem.flags & flags::DEBUG != 0 {
            continue;
        }
        if slot.team != 0 && slot.team != team {
            continue;
        }
        if slot.client_num != HUDELEM_ALL_CLIENTS && slot.client_num != client_num {
            continue;
        }
        if slot.archived != 0 {
            if (which & HUDELEM_UPDATE_ARCHIVAL) != 0 && archival.len() < HUDELEM_BANK_CAPACITY {
                archival.push(slot.elem);
            }
        } else if (which & HUDELEM_UPDATE_CURRENT) != 0 && current.len() < HUDELEM_BANK_CAPACITY {
            current.push(slot.elem);
        }
    }
    (archival, current)
}

/// Killcam playback: moves an archived bank's start times by `rebase_ms`.
pub fn rebase_hud_archival(bank: &mut [HudElem], rebase_ms: i32) {
    for elem in bank {
        for t in [
            &mut elem.time,
            &mut elem.fade_start_time,
            &mut elem.scale_start_time,
            &mut elem.move_start_time,
            &mut elem.font_scale_start_time,
        ] {
            if *t != 0 {
                *t = t.saturating_add(rebase_ms);
            }
        }
    }
}

const PULSE_FX_CLIENTS: usize = 64;

/// SetPulseFX's sound id, one counter per client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseFxSoundIds {
    last_assigned: [i32; PULSE_FX_CLIENTS],
}

impl Default for PulseFxSoundIds {
    fn default() -> Self {
        Self { last_assigned: [0; PULSE_FX_CLIENTS] }
    }
}

impl PulseFxSoundIds {
    fn next(&mut self, client: usize) -> i32 {
        let Some(last) = self.last_assigned.get_mut(client) else { return 1 };
        let mut id = (*last + 1).rem_euclid(HUDELEM_PULSE_SOUND_IDS);
        if id == 0 {
            id = 1;
        }
        *last = id;
        id
    }
}

/// SetPulseFX on a client's elem (the caller rejects an every-client elem).
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

    #[test]
    fn pulse_sound_ids_wrap_per_client() {
        let mut ids = PulseFxSoundIds::default();
        let a: Vec<i32> = (0..32).map(|_| ids.next(0)).collect();
        assert_eq!(a[0], 1);
        assert_eq!(a[30], 31);
        assert_eq!(a[31], 1);
        assert_eq!(ids.next(1), 1);
    }

    #[test]
    fn update_skips_debug_and_other_clients() {
        let mut pool = Vec::new();
        let all = alloc_hud_elem(&mut pool, HUDELEM_ALL_CLIENTS, 0).unwrap();
        let mine = alloc_hud_elem(&mut pool, 2, 0).unwrap();
        let theirs = alloc_hud_elem(&mut pool, 3, 0).unwrap();
        let debug = alloc_hud_elem(&mut pool, HUDELEM_ALL_CLIENTS, 0).unwrap();
        pool[debug].elem.flags |= flags::DEBUG;
        pool[mine].archived = 0;
        let _ = (all, theirs);
        let (archival, current) = hud_elem_update_client(&pool, ClientId(2), 1, HUDELEM_UPDATE_BOTH);
        assert_eq!((archival.len(), current.len()), (1, 1));
    }
}
