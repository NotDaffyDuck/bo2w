//! Which gun the player has asked for: the cycle keys, the scorestreak key, and the bookkeeping
//! that keeps the request in step with what the player is actually holding.

use bevy::prelude::*;
use input_t6::weapon_select::{SelectFlags, cycle_weapon, weapon_cycle_allowed};
use playerstate_t6::game::PlayerState;
use weapon_t6::ammo_table::{ammo_table_key, clip_table_key, get_ammo_player_both_clips};
use weapon_t6::combat_facts::WeaponCombatFacts;

/// BO2 weapon states (offhand init up to detonate/throw) in which a pressed cycle key cancels
/// the grenade instead of switching gun.
const OFFHAND_BUSY_STATES: std::ops::RangeInclusive<i32> = 22..=27;

/// The weapon the client last asked for, and what it needs to ask again next time.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponSelect {
    /// The requested gun.
    pub index: u32,

    /// When the request was made, in client time.
    pub time: i32,

    /// The life the request belongs to; a new life starts the request over.
    pub life_sequence: u32,

    /// The gun the player held at the last look.
    pub held_index: u32,

    /// The last ordinary gun the cycle walked away from.
    pub last_primary: u32,

    /// The base gun when the request is an alt mode of it.
    pub mapped_index: u32,

    /// The inventory weapon was asked for and is not in the player's hands yet.
    pub switching_to_inventory: bool,

    /// Per slot in `ps.weapons`, the alt mode (select fire, the other optic, launcher) the gun
    /// was left in, so the cycle brings it back up in that mode.
    pub last_mode: [u32; 15],
}

/// Keeps the request in step with the held gun. A fresh life, or the held gun changing away
/// from the one that was asked for, makes the held gun the request.
pub fn follow_held_weapon_select(
    select: &mut WeaponSelect,
    ps_weapon: u32,
    life_sequence: u32,
    cg_time: i32,
) {
    if ps_weapon == 0 {
        return;
    }
    let request_is_stale = select.index == 0
        || select.life_sequence != life_sequence
        || (select.index == select.held_index && ps_weapon != select.held_index);
    if request_is_stale {
        select.index = ps_weapon;
        select.mapped_index = ps_weapon;
        select.last_primary = 0;
        select.life_sequence = life_sequence;
        select.time = cg_time;
    }
    select.held_index = ps_weapon;
}

pub(super) fn select_flags(ps: &PlayerState) -> SelectFlags {
    SelectFlags {
        pm_flags: ps.pm_flags,
        weap_flags: ps.weap_flags,
        other_flags: ps.other_flags,
        e_flags: ps.e_flags,
        pm_type: ps.pm_type,
    }
}

/// How much ammo the player has for `weapon`, stock and every clip together.
pub(super) fn ammo_on_hand(ps: &PlayerState, weapon: u32, facts: &WeaponCombatFacts) -> i32 {
    let ammo_key = ammo_table_key(facts.ammo_index, weapon);
    let clip_key = clip_table_key(facts.clip_index, weapon);
    get_ammo_player_both_clips(
        &ps.weapons,
        &ps.weapon_data,
        &ps.ammo,
        &ps.ammoclip,
        weapon,
        ammo_key,
        clip_key,
    )
}

fn grenade_can_be_cancelled(world: &sim::SimWorld, grenade: u32) -> bool {
    world
        .weapon_combat_row(grenade)
        .and_then(|facts| facts.offhand_hold_is_cancelable)
        .unwrap_or(false)
}

/// BO2 asks before every cycle whether a grenade that can be cancelled is in hand; if so the
/// key press becomes the cancel button on the next command and the cycle is dropped. Returns
/// whether the press was used up that way.
fn press_cancels_grenade(
    input: &mut input_t6::ClientInput,
    ps: &PlayerState,
    world: &sim::SimWorld,
    t6_offhand: Option<(i32, u32)>,
) -> bool {
    match t6_offhand {
        // The client's own predicted grenade state.
        Some((state, grenade)) => {
            let cancel = OFFHAND_BUSY_STATES.contains(&state) && grenade_can_be_cancelled(world, grenade);
            if cancel {
                diag::info!(Net, "bo2mp cycle: grenade cancel (state {state}, grenade {grenade})");
                input.offhand_switch_t6 = true;
            }
            cancel
        }
        // No predicted grenade: go by the player state's own offhand init, prepare and hold.
        None if matches!(ps.weapon_state_t6, 22..=24 | 28) => {
            let cancel = grenade_can_be_cancelled(world, ps.off_hand_index.max(0) as u32);
            if cancel {
                input.offhand_hold_cancel = true;
            }
            cancel
        }
        None => false,
    }
}

/// The scorestreak weapons on the d-pad slots, cheapest first (the class script puts them on
/// slots 4, 2 and 1 by cost). Empty slots are left out.
pub(super) fn streak_slot_weapons(ps: &PlayerState) -> impl Iterator<Item = u32> + '_ {
    [3usize, 1, 0]
        .into_iter()
        .filter(|&slot| ps.action_slot_type[slot] == 1)
        .map(|slot| ps.action_slot_param[slot].max(0) as u32)
        .filter(|&weapon| weapon != 0)
}

/// Moves the request on by one gun in the cycle direction, the way the cycle key does.
pub fn cycle_weapon_select(
    select: &mut WeaponSelect,
    input: &mut input_t6::ClientInput,
    ps: &PlayerState,
    world: &sim::SimWorld,
    time: i32,
    next: bool,
    t6_offhand: Option<(i32, u32)>,
) {
    if !weapon_cycle_allowed(select_flags(ps), time, select.time, 0, 0) {
        diag::info!(
            Net,
            "bo2mp cycle: refused (pm {:#x} weap {:#x} other {:#x} e {:#x} type {}, {} ms since the last pick)",
            ps.pm_flags,
            ps.weap_flags,
            ps.other_flags,
            ps.e_flags,
            ps.pm_type,
            time.wrapping_sub(select.time)
        );
        return;
    }
    if press_cancels_grenade(input, ps, world, t6_offhand) {
        return;
    }

    select.time = time;
    if world.weapon_combat_len() < 2 {
        return;
    }
    let facts_of = |weapon: u32| {
        world
            .weapon_combat_row(weapon)
            .expect("owned weapon must have a captured catalog row")
    };

    // An alt mode cycles back to its base gun, unless the mode disables switching; then the
    // walk moves on.
    let back_to_base = sim::alt_mode_cycle_base(world, select.index)
        .filter(|&base| ps.weapons.contains(&(base as i32)));
    // Scorestreaks only come out on the streak key and the knife only on the melee key, so the
    // walk skips both.
    let walk = || {
        let skipped = |weapon: u32| streak_slot_weapons(ps).any(|w| w == weapon) || sim::is_melee_knife(world, weapon);
        cycle_weapon(
            &ps.weapons,
            select.index,
            select.mapped_index,
            select.last_primary,
            next,
            |weapon| if skipped(weapon) { -1 } else { facts_of(weapon).inventory_type },
        )
        .map(|gun| sim::remembered_mode(world, &ps.weapons, &select.last_mode, gun))
    };
    let Some(target) = back_to_base.or_else(walk) else {
        diag::info!(Net, "bo2mp cycle: nothing to cycle to from {}", select.index);
        return;
    };
    diag::info!(Net, "bo2mp cycle: {} -> {target} (holding {})", select.index, ps.weapon);

    let target_facts = facts_of(target);
    if target_facts.select_requires_ammo.unwrap_or(false) && ammo_on_hand(ps, target, &target_facts) == 0 {
        return;
    }
    if select.index == target {
        return;
    }
    if select.index != 0 && facts_of(select.index).inventory_type == 0 {
        select.last_primary = select.index;
    }
    // An alt mode's mapped gun is its base gun.
    select.mapped_index = if target_facts.inventory_type == 3 {
        target_facts.alternate_weapon
    } else {
        target
    };
    select.index = target;
    sim::remember_mode(world, &ps.weapons, &mut select.last_mode, target);
    input_t6::set_ads(input, false);
}

/// The scorestreak the streak key brings out: the cheapest one earned, or if one is already up,
/// the next earned one after it (wrapping round). The class script gives every streak weapon at
/// spawn with no ammo, so one that is not earned yet (empty) never comes out.
pub(super) fn next_earned_streak(ps: &PlayerState, world: &sim::SimWorld, held: u32) -> Option<u32> {
    let earned: Vec<u32> = streak_slot_weapons(ps)
        .filter(|&weapon| ps.weapons.contains(&(weapon as i32)))
        .filter(|&weapon| {
            world
                .weapon_combat_row(weapon)
                .is_some_and(|facts| ammo_on_hand(ps, weapon, &facts) > 0)
        })
        .collect();
    let after_held = earned.iter().position(|&w| w == held).map(|at| at + 1);
    after_held
        .and_then(|at| earned.get(at))
        .or(earned.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selecting(index: u32, held: u32, life: u32) -> WeaponSelect {
        WeaponSelect {
            index,
            held_index: held,
            life_sequence: life,
            ..WeaponSelect::default()
        }
    }

    #[test]
    fn holding_nothing_leaves_the_request_alone() {
        let mut select = selecting(7, 7, 1);
        follow_held_weapon_select(&mut select, 0, 2, 500);
        assert_eq!(select, selecting(7, 7, 1));
    }

    #[test]
    fn the_first_held_gun_becomes_the_request() {
        let mut select = WeaponSelect::default();
        follow_held_weapon_select(&mut select, 12, 3, 900);
        assert_eq!(select.index, 12);
        assert_eq!(select.mapped_index, 12);
        assert_eq!(select.held_index, 12);
        assert_eq!(select.life_sequence, 3);
        assert_eq!(select.time, 900);
        assert_eq!(select.last_primary, 0);
    }

    #[test]
    fn a_new_life_starts_the_request_over_even_with_the_same_gun() {
        let mut select = selecting(12, 12, 3);
        select.last_primary = 5;
        select.time = 100;
        follow_held_weapon_select(&mut select, 12, 4, 800);
        assert_eq!((select.life_sequence, select.time, select.last_primary), (4, 800, 0));
    }

    #[test]
    fn a_request_the_player_already_holds_follows_the_held_gun() {
        let mut select = selecting(12, 12, 3);
        select.time = 100;
        follow_held_weapon_select(&mut select, 20, 3, 800);
        assert_eq!((select.index, select.mapped_index, select.held_index), (20, 20, 20));
        assert_eq!(select.time, 800);
    }

    #[test]
    fn a_request_still_in_flight_is_kept_while_another_gun_is_held() {
        let mut select = selecting(30, 12, 3);
        select.time = 100;
        follow_held_weapon_select(&mut select, 12, 3, 800);
        assert_eq!((select.index, select.time), (30, 100));
        assert_eq!(select.held_index, 12);
    }

    #[test]
    fn the_streak_slots_come_out_cheapest_first_and_skip_empty_ones() {
        let mut ps = PlayerState::ZERO;
        ps.action_slot_type = [1, 1, 0, 1];
        ps.action_slot_param = [40, 30, 99, 20];
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [20, 30, 40]);

        ps.action_slot_param[1] = 0;
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [20, 40]);

        ps.action_slot_type[3] = 2;
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [40]);
    }

    #[test]
    fn a_negative_slot_param_counts_as_no_weapon() {
        let mut ps = PlayerState::ZERO;
        ps.action_slot_type = [1, 0, 0, 0];
        ps.action_slot_param = [-5, 0, 0, 0];
        assert_eq!(streak_slot_weapons(&ps).count(), 0);
    }

    #[test]
    fn the_select_flags_copy_the_player_state_fields() {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = 0xc08;
        ps.weap_flags = 0x880;
        ps.other_flags = 3;
        ps.e_flags = 9;
        ps.pm_type = 2;
        let flags = select_flags(&ps);
        assert_eq!(
            (flags.pm_flags, flags.weap_flags, flags.other_flags, flags.e_flags, flags.pm_type),
            (0xc08, 0x880, 3, 9, 2)
        );
    }
}
