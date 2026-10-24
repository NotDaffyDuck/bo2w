//! What a player is carrying for a weapon: the rounds in each hand's magazine and the reserve behind them.

use playerstate_t6::game::PlayerState;
use weapon_t6::ammo_table::{
    ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, player_weapons_find_slot,
    set_ammo_not_in_clip, set_clip_for_hand,
};

use crate::frame::FrameWorld;

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
pub(super) fn held_rounds(world: &FrameWorld, ps: &PlayerState, weapon: u32) -> Rounds {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return Rounds::default();
    };
    let clip_key = clip_table_key(facts.clip_index, weapon);
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    Rounds {
        right: get_clip_for_hand(&ps.ammoclip, clip_key, 0),
        left: get_clip_for_hand(&ps.ammoclip, clip_key, 1),
        reserve: get_ammo_not_in_clip(&ps.ammo, ammo_key),
    }
}

/// Sets the rounds `ps` holds for `weapon`. A table slot of 0 means the weapon has no such pool and is left alone.
pub(super) fn set_held_rounds(world: &FrameWorld, ps: &mut PlayerState, weapon: u32, rounds: Rounds) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    let clip_key = clip_table_key(facts.clip_index, weapon);
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    if clip_key != 0 {
        let _ = set_clip_for_hand(&mut ps.ammoclip, clip_key, 0, rounds.right);
        let _ = set_clip_for_hand(&mut ps.ammoclip, clip_key, 1, rounds.left);
    }
    if ammo_key != 0 {
        let _ = set_ammo_not_in_clip(&mut ps.ammo, ammo_key, rounds.reserve);
    }
}

/// Pours everything found in a dropped copy of `weapon` into the reserve for it, up to the weapon's cap. The
/// magazines the player already holds stay as they are.
pub(super) fn pour_into_reserve(world: &FrameWorld, ps: &mut PlayerState, weapon: u32, found: Rounds) {
    let Some(facts) = world.combat_facts_for(weapon) else {
        return;
    };
    let have = held_rounds(world, ps, weapon);
    let reserve = have.reserve.saturating_add(found.total()).min(facts.max_ammo);
    set_held_rounds(world, ps, weapon, Rounds { reserve, ..have });
}

/// Empties every slot of `ps` that holds `weapon`, and the current-weapon fields if it was in hand.
pub(super) fn take_weapon(ps: &mut PlayerState, weapon: u32) {
    let wanted = weapon as i32;
    for slot in ps.weapons.iter_mut().filter(|slot| **slot == wanted) {
        *slot = 0;
    }
    if ps.weapon == weapon || ps.weapon_primary == weapon {
        ps.weapon = 0;
        ps.weapon_primary = 0;
    }
}

fn is_primary(world: &FrameWorld, weapon: u32) -> bool {
    world.combat_facts_for(weapon).is_some_and(|facts| facts.inventory_type == INVENTORY_PRIMARY)
}

/// How many primary weapons `ps` carries.
pub(super) fn primary_count(world: &FrameWorld, ps: &PlayerState) -> usize {
    ps.weapons.iter().filter(|&&weapon| weapon > 0 && is_primary(world, weapon as u32)).count()
}

/// The primary weapon the player has out, or the one an alt mode belongs to; 0 when neither is a carried primary.
pub(super) fn current_primary(world: &FrameWorld, ps: &PlayerState) -> u32 {
    let in_alt_mode =
        world.combat_facts_for(ps.weapon).is_some_and(|facts| facts.inventory_type == INVENTORY_ALT_MODE);
    let weapon = if in_alt_mode { ps.weapon_primary } else { ps.weapon };
    let carried = weapon != 0 && player_weapons_find_slot(&ps.weapons, weapon as i32) >= 0;
    if carried && is_primary(world, weapon) { weapon } else { 0 }
}
