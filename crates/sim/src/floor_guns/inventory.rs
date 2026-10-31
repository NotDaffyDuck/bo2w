//! What a player is carrying for a weapon: the rounds in each hand's magazine and the reserve behind them.

use playerstate_t6::game::AvatarState;
use weapon_t6::ammo_table::{
    ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, player_weapons_find_slot,
    set_ammo_not_in_clip, set_clip_for_hand,
};

use crate::step_view::StepWorldView;

/// A weapon's inventory type in the weapon table. Only primaries can be dropped or swapped for.
pub(super) const INVENTORY_PRIMARY: i32 = 0;
/// An alternate fire mode rides on its parent primary and is never carried alone.
pub(super) const INVENTORY_ALT_MODE: i32 = 3;

/// Right-hand magazine, left-hand magazine and reserve for one weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Rounds {
    pub right: i32,
    pub left: i32,
    pub reserve: i32,
}

impl Rounds {
    pub(super) fn total(&self) -> i32 {
        self.right + self.left + self.reserve
    }
}

/// The rounds `ps` holds for `weapon`; nothing for a weapon the tables do not know.
pub(super) fn held_rounds(world: &StepWorldView, ps: &AvatarState, weapon: u32) -> Rounds {
    let Some(facts) = world.damage_profile_of(weapon) else {
        return Rounds::default();
    };
    let clip_slot = clip_table_key(facts.motion_clip_slot, weapon);
    let ammo_slot = ammo_table_key(facts.ammo_type_slot, weapon);
    Rounds {
        right: get_clip_for_hand(&ps.magazine_rows, clip_slot, 0),
        left: get_clip_for_hand(&ps.magazine_rows, clip_slot, 1),
        reserve: get_ammo_not_in_clip(&ps.ammo, ammo_slot),
    }
}

/// Sets the rounds `ps` holds for `weapon`. A table slot of 0 means the weapon has no such pool and is left alone.
pub(super) fn set_held_rounds(world: &StepWorldView, ps: &mut AvatarState, gun: u32, rounds: Rounds) {
    let Some(facts) = world.damage_profile_of(gun) else {
        return;
    };
    let clip_slot = clip_table_key(facts.motion_clip_slot, gun);
    let ammo_slot = ammo_table_key(facts.ammo_type_slot, gun);
    if clip_slot != 0 {
        let _ = set_clip_for_hand(&mut ps.magazine_rows, clip_slot, 0, rounds.right);
        let _ = set_clip_for_hand(&mut ps.magazine_rows, clip_slot, 1, rounds.left);
    }
    if ammo_slot != 0 {
        let _ = set_ammo_not_in_clip(&mut ps.ammo, ammo_slot, rounds.reserve);
    }
}

/// Pours everything found in a dropped copy of `weapon` into the reserve for it, up to the weapon's cap. The
/// magazines the player already holds stay as they are.
pub(super) fn pour_into_reserve(world: &StepWorldView, ps: &mut AvatarState, gun: u32, found: Rounds) {
    let Some(facts) = world.damage_profile_of(gun) else {
        return;
    };
    let have = held_rounds(world, ps, gun);
    let reserve = have.reserve.saturating_add(found.total()).min(facts.reserve_cap);
    set_held_rounds(world, ps, gun, Rounds { reserve, ..have });
}

/// Empties every slot of `ps` that holds `weapon`, and the current-weapon fields if it was in hand.
pub(super) fn take_weapon(ps: &mut AvatarState, gun: u32) {
    let wanted = gun as i32;
    for slot in ps.weapons.iter_mut().filter(|slot| **slot == wanted) {
        *slot = 0;
    }
    if ps.weapon == gun || ps.weapon_primary == gun {
        (ps.weapon, ps.weapon_primary) = (0, 0);
    }
}

fn is_primary(world: &StepWorldView, gun: u32) -> bool {
    world.damage_profile_of(gun).is_some_and(|facts| facts.carry_slot_kind == INVENTORY_PRIMARY)
}

/// How many primary weapons `ps` carries.
pub(super) fn main_light_total(world: &StepWorldView, ps: &AvatarState) -> usize {
    ps.weapons.iter().filter(|&&weapon| weapon > 0 && is_primary(world, weapon as u32)).count()
}

/// The primary weapon the player has out, or the one an alt mode belongs to; 0 when neither is a carried primary.
pub(super) fn current_primary(world: &StepWorldView, ps: &AvatarState) -> u32 {
    let in_alt_mode =
        world.damage_profile_of(ps.weapon).is_some_and(|facts| facts.carry_slot_kind == INVENTORY_ALT_MODE);
    let gun = if in_alt_mode { ps.weapon_primary } else { ps.weapon };
    let carried = gun != 0 && player_weapons_find_slot(&ps.weapons, gun as i32) >= 0;
    if carried && is_primary(world, gun) { gun } else { 0 }
}
