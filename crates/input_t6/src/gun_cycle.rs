//! Picking the next weapon in the inventory.

/// The player state flags that decide whether the weapon may change now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectFlags {
    pub movement_state_bits: u32,
    pub weapon_state_bits: u32,
    pub secondary_state_bits: u32,
    pub entity_state_bits: u32,
    pub movement_mode: i32,
}

/// Movement flags that hold the weapon where it is.
const PM_BLOCKS: u32 = 0xc08;
/// Weapon flags that hold the weapon where it is.
const WEAP_BLOCKS: u32 = 0x880;
/// The other-flags bit that holds the weapon where it is.
const OTHER_BLOCKS: u32 = 0x2;
/// The other-flags bit set while the player is in the game.
const OTHER_PLAYING: u32 = 0x4;
/// Entity flags that hold the weapon where it is.
const E_BLOCKS: u32 = 0x10_0c00;
/// Movement types above this (dead, spectating and the like) cannot change weapon.
const PM_TYPE_LAST_ALIVE: i32 = 7;

/// A switch-weapon press may change the weapon now: nothing blocks it, the player is in the
/// game, the last change is `cycle_delay` ms old, and no override holds it until later.
pub fn may_switch_weapon(
    ps: SelectFlags,
    time: i32,
    last_switch_ms: i32,
    switch_hold_until: i32,
    switch_gap_ms: i32,
) -> bool {
    let blocked = ps.movement_state_bits & PM_BLOCKS != 0
        || ps.weapon_state_bits & WEAP_BLOCKS != 0
        || ps.secondary_state_bits & OTHER_BLOCKS != 0
        || ps.entity_state_bits & E_BLOCKS != 0
        || ps.movement_mode > PM_TYPE_LAST_ALIVE;
    !blocked
        && ps.secondary_state_bits & OTHER_PLAYING != 0
        && time.wrapping_sub(last_switch_ms) >= switch_gap_ms
        && switch_hold_until <= time
}

/// Inventory slots the switch-weapon keys walk.
const SLOTS: usize = 15;
/// A gun's alternate mode (a grenade launcher under the barrel): its kind is the gun it hangs
/// on.
const ALT_MODE: i32 = 3;
/// Weapons that cycle among their own kind only.
const SEPARATE_CYCLE: i32 = 4;

/// The weapon a switch-weapon press goes to from `selected`, walking the inventory `next`
/// (forward) or back. An alternate mode counts as the gun `mapped` it hangs on. A weapon held
/// outside the normal cycle goes back to the last primary when the player still has it.
/// Otherwise the walk takes the first weapon of the same cycle, `None` when there is none.
pub fn rotate_gun(
    gun_table: &[i32; SLOTS],
    current_gun: u32,
    mapped: u32,
    previous_main_weapon: u32,
    next: bool,
    mut carry_slot_kind: impl FnMut(u32) -> i32,
) -> Option<u32> {
    let held = if current_gun != 0 && carry_slot_kind(current_gun) == ALT_MODE {
        mapped
    } else {
        current_gun
    };
    let kind = if held == 0 { 0 } else { carry_slot_kind(held) };
    let has = |w: u32| gun_table.iter().any(|&slot| slot as u32 == w);
    if kind != 0 && kind != SEPARATE_CYCLE && previous_main_weapon != 0 && has(previous_main_weapon) {
        return Some(previous_main_weapon);
    }
    let wanted = if kind == SEPARATE_CYCLE {
        SEPARATE_CYCLE
    } else {
        0
    };
    let from = if held == 0 {
        0
    } else {
        gun_table.iter().position(|&w| w as u32 == held).unwrap_or(0)
    };
    (usize::from(held != 0)..SLOTS)
        .map(|d| {
            if next {
                (from + d) % SLOTS
            } else {
                (from + SLOTS - d) % SLOTS
            }
        })
        .map(|slot| gun_table[slot])
        .find(|&w| w > 0 && carry_slot_kind(w as u32) == wanted)
        .map(|w| w as u32)
}
