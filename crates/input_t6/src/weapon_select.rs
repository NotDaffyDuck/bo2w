//! Picking the next weapon in the inventory.

/// The player state flags that decide whether the weapon may change now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectFlags {
    pub pm_flags: u32,
    pub weap_flags: u32,
    pub other_flags: u32,
    pub e_flags: u32,
    pub pm_type: i32,
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
pub fn weapon_cycle_allowed(
    ps: SelectFlags,
    time: i32,
    select_time: i32,
    select_time_override: i32,
    cycle_delay: i32,
) -> bool {
    let blocked = ps.pm_flags & PM_BLOCKS != 0
        || ps.weap_flags & WEAP_BLOCKS != 0
        || ps.other_flags & OTHER_BLOCKS != 0
        || ps.e_flags & E_BLOCKS != 0
        || ps.pm_type > PM_TYPE_LAST_ALIVE;
    !blocked
        && ps.other_flags & OTHER_PLAYING != 0
        && time.wrapping_sub(select_time) >= cycle_delay
        && select_time_override <= time
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
pub fn cycle_weapon(
    weapons: &[i32; SLOTS],
    selected: u32,
    mapped: u32,
    last_primary: u32,
    next: bool,
    mut inventory_type: impl FnMut(u32) -> i32,
) -> Option<u32> {
    let held = if selected != 0 && inventory_type(selected) == ALT_MODE {
        mapped
    } else {
        selected
    };
    let kind = if held == 0 { 0 } else { inventory_type(held) };
    let has = |w: u32| weapons.iter().any(|&slot| slot as u32 == w);
    if kind != 0 && kind != SEPARATE_CYCLE && last_primary != 0 && has(last_primary) {
        return Some(last_primary);
    }
    let wanted = if kind == SEPARATE_CYCLE {
        SEPARATE_CYCLE
    } else {
        0
    };
    let from = if held == 0 {
        0
    } else {
        weapons.iter().position(|&w| w as u32 == held).unwrap_or(0)
    };
    (usize::from(held != 0)..SLOTS)
        .map(|d| {
            if next {
                (from + d) % SLOTS
            } else {
                (from + SLOTS - d) % SLOTS
            }
        })
        .map(|slot| weapons[slot])
        .find(|&w| w > 0 && inventory_type(w as u32) == wanted)
        .map(|w| w as u32)
}
