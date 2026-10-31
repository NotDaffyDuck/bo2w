//! Which gun the player has asked for: the cycle keys, the scorestreak key, and the bookkeeping
//! that keeps the request in step with what the player is actually holding.

use bevy::prelude::*;
use input_t6::gun_cycle::{SelectFlags, rotate_gun, may_switch_weapon};
use playerstate_t6::game::AvatarState;
use weapon_t6::ammo_table::{ammo_table_key, clip_table_key, get_ammo_player_both_clips};
use weapon_t6::combat_facts::GunBattleProfile;

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
    pub life_no: u32,

    /// The gun the player held at the last look.
    pub retained_slot_position: u32,

    /// The last ordinary gun the cycle walked away from.
    pub previous_main_weapon: u32,

    /// The base gun when the request is an alt mode of it.
    pub mapped_index: u32,

    /// The inventory weapon was asked for and is not in the player's hands yet.
    pub switching_to_inventory: bool,

    /// Per slot in `ps.weapons`, the alt mode (select fire, the other optic, launcher) the gun
    /// was left in, so the cycle brings it back up in that mode.
    pub last_mode: [u32; 15],

    /// The last primary gun the player held (BO2's latest primary).
    pub latest_primary: u32,

    /// The gun in hand had nothing left in its clip or stock.
    pub held_empty: bool,
}

/// Keeps the request in step with the held gun. A fresh life, or the held gun changing away
/// from the one that was asked for, makes the held gun the request.
pub fn follow_held_weapon_select(select: &mut WeaponSelect, held: u32, life: u32, scene_clock_ms: i32) {
    // Empty hands (mid-swap) say nothing about the request.
    if held == 0 {
        return;
    }
    let new_life = select.life_no != life;
    let moved_off_request = select.index == select.retained_slot_position && held != select.retained_slot_position;
    if select.index == 0 || new_life || moved_off_request {
        let restart = WeaponSelect { index: held, mapped_index: held, previous_main_weapon: 0, time: scene_clock_ms, ..*select };
        *select = WeaponSelect { life_no: life, ..restart };
    }
    select.retained_slot_position = held;
}

/// BO2's client puts a gun back in the player's hands when the one he held is gone after he
/// emptied it: the latest primary he held if he still has it, else the first primary on from
/// there. TranZit leans on it: setting down the turbine, turret or trap fires the equipment's
/// one round, and the script takes it with no switch of its own. Offhands are left alone, as
/// BO2 leaves its equipment and specific-use slots.
pub fn out_of_ammo_change(gun_choice: &mut WeaponSelect, ps: &AvatarState, sim_ws: &sim::SimWorld, scene_clock_ms: i32) {
    let carry_slot_kind = |gun_id: u32| sim_ws.weapon_stats_entry(gun_id).map_or(-1, |facts| facts.carry_slot_kind);
    if ps.weapon != 0 {
        match carry_slot_kind(ps.weapon) {
            0 => gun_choice.latest_primary = ps.weapon,
            3 if ps.weapon_primary != 0 => gun_choice.latest_primary = ps.weapon_primary,
            _ => {}
        }
        gun_choice.held_empty = sim_ws
            .weapon_stats_entry(ps.weapon)
            .is_some_and(|facts| ammo_on_hand(ps, ps.weapon, &facts) == 0);
        return;
    }
    let gone = gun_choice.retained_slot_position;
    if gone == 0
        || gun_choice.index != gone
        || !gun_choice.held_empty
        || ps.weapons.contains(&(gone as i32))
        || carry_slot_kind(gone) == 1
        || ps.movement_mode >= playerstate_t6::game::MOVE_KIND_DEAD
    {
        return;
    }
    let owned = |gun_id: u32| gun_id != 0 && ps.weapons.contains(&(gun_id as i32));
    let target = if owned(gun_choice.latest_primary) {
        Some(gun_choice.latest_primary)
    } else {
        rotate_gun(&ps.weapons, 0, 0, 0, true, carry_slot_kind)
    };
    let Some(target) = target else {
        return;
    };
    // Held as well as asked for, so a switch the server makes next is followed rather than
    // fought.
    gun_choice.index = target;
    gun_choice.mapped_index = target;
    gun_choice.retained_slot_position = target;
    gun_choice.held_empty = false;
    gun_choice.time = scene_clock_ms;
}

pub(super) fn select_flags(own_state: &AvatarState) -> SelectFlags {
    SelectFlags {
        movement_state_bits: own_state.movement_state_bits,
        weapon_state_bits: own_state.weapon_state_bits,
        secondary_state_bits: own_state.secondary_state_bits,
        entity_state_bits: own_state.e_flags,
        movement_mode: own_state.movement_mode,
    }
}

/// How much ammo the player has for `weapon`, stock and every clip together.
pub(super) fn ammo_on_hand(ps: &AvatarState, gun_id: u32, facts: &GunBattleProfile) -> i32 {
    let ammo_key = ammo_table_key(facts.ammo_type_slot, gun_id);
    let clip_key = clip_table_key(facts.motion_clip_slot, gun_id);
    let AvatarState { weapons, weapon_data, ammo, magazine_rows, .. } = ps;
    get_ammo_player_both_clips(weapons, weapon_data, ammo, magazine_rows, gun_id, ammo_key, clip_key)
}

fn grenade_can_be_cancelled(sim_ws: &sim::SimWorld, grenade: u32) -> bool {
    // Only a gun row that says yes; no row, or no say, means no.
    sim_ws.weapon_stats_entry(grenade).and_then(|facts| facts.offhand_hold_is_cancelable) == Some(true)
}

/// BO2 asks before every cycle whether a grenade that can be cancelled is in hand; if so the
/// key press becomes the cancel button on the next command and the cycle is dropped. Returns
/// whether the press was used up that way.
fn press_cancels_grenade(input: &mut input_t6::PlayerIntentFeed, ps: &AvatarState, sim_world: &sim::SimWorld, t6_offhand: Option<(i32, u32)>) -> bool {
    let sim_ws = sim_world;
    match t6_offhand {
        // The client's own predicted grenade state.
        Some((held_state, grenade)) => {
            let cancel = OFFHAND_BUSY_STATES.contains(&held_state) && grenade_can_be_cancelled(sim_ws, grenade);
            if cancel {
                diag::info!(WireLink, "bo2mp cycle: grenade cancel (state {held_state}, grenade {grenade})");
                input.offhand_switch_t6 = true;
            }
            cancel
        }
        // No predicted grenade: go by the player state's own offhand init, prepare and hold.
        None if matches!(ps.weapon_state_t6, 22..=24 | 28) => {
            let cancel = grenade_can_be_cancelled(sim_ws, ps.secondary_hand_slot.max(0) as u32);
            if cancel {
                input.throw_grip_abort = true;
            }
            cancel
        }
        _ => false,
    }
}

/// The scorestreak weapons on the d-pad slots, cheapest first (the class script puts them on
/// slots 4, 2 and 1 by cost). Empty slots are left out.
pub(super) fn streak_slot_weapons(ps: &AvatarState) -> impl Iterator<Item = u32> + '_ {
    [3usize, 1, 0]
        .into_iter()
        .filter(|&slot| ps.dpad_kind[slot] == 1)
        .map(|slot| ps.action_slot_param[slot].max(0) as u32)
        .filter(|&gun_id| gun_id != 0)
}

/// Moves the request on by one gun in the cycle direction, the way the cycle key does.
pub fn cycle_weapon_select(
    gun_choice: &mut WeaponSelect,
    input: &mut input_t6::PlayerIntentFeed,
    own_state: &AvatarState,
    sim_world: &sim::SimWorld,
    now: i32,
    next: bool,
    t6_offhand: Option<(i32, u32)>,
) {
    let sim_ws = sim_world;
    if !may_switch_weapon(select_flags(own_state), now, gun_choice.time, 0, 0) {
        diag::info!(
            WireLink,
            "bo2mp cycle: refused (pm {:#x} weap {:#x} other {:#x} e {:#x} type {}, {} ms since the last pick)",
            own_state.movement_state_bits,
            own_state.weapon_state_bits,
            own_state.secondary_state_bits,
            own_state.e_flags,
            own_state.movement_mode,
            now.wrapping_sub(gun_choice.time)
        );
        return;
    }
    if press_cancels_grenade(input, own_state, sim_ws, t6_offhand) {
        return;
    }

    gun_choice.time = now;
    if sim_ws.weapon_stats_count() < 2 {
        return;
    }
    let facts_of = |gun_id: u32| sim_ws.weapon_stats_entry(gun_id).expect("owned weapon must have a captured catalog row");

    // An alt mode cycles back to its base gun, unless the mode disables switching; then the
    // walk moves on.
    let back_to_base = sim::alt_mode_cycle_base(sim_ws, gun_choice.index)
        .filter(|&base| own_state.weapons.contains(&(base as i32)));
    // Scorestreaks only come out on the streak key and the knife only on the melee key, so the
    // walk skips both.
    let walk = || {
        let skipped = |gun_id: u32| streak_slot_weapons(own_state).any(|w| w == gun_id) || sim::is_melee_knife(sim_ws, gun_id);
        let slot_kind = |gun_id: u32| if skipped(gun_id) { -1 } else { facts_of(gun_id).carry_slot_kind };
        let WeaponSelect { index: from, mapped_index, previous_main_weapon, .. } = *gun_choice;
        rotate_gun(&own_state.weapons, from, mapped_index, previous_main_weapon, next, slot_kind)
            .map(|gun| sim::remembered_mode(sim_ws, &own_state.weapons, &gun_choice.last_mode, gun))
    };
    let Some(target) = back_to_base.or_else(walk) else {
        diag::info!(WireLink, "bo2mp cycle: nothing to cycle to from {}", gun_choice.index);
        return;
    };
    diag::info!(WireLink, "bo2mp cycle: {} -> {target} (holding {})", gun_choice.index, own_state.weapon);

    let target_facts = facts_of(target);
    if target_facts.select_requires_ammo.unwrap_or(false) && ammo_on_hand(own_state, target, &target_facts) == 0 {
        return;
    }
    if gun_choice.index == target {
        return;
    }
    if gun_choice.index != 0 && facts_of(gun_choice.index).carry_slot_kind == 0 {
        gun_choice.previous_main_weapon = gun_choice.index;
    }
    // An alt mode's mapped gun is its base gun.
    let mapped = if target_facts.carry_slot_kind == 3 { target_facts.alternate_weapon } else { target };
    (gun_choice.index, gun_choice.mapped_index) = (target, mapped);
    sim::remember_mode(sim_ws, &own_state.weapons, &mut gun_choice.last_mode, target);
    input_t6::set_ads(input, false);
}

/// The scorestreak the streak key brings out: the cheapest one earned, or if one is already up,
/// the next earned one after it (wrapping round). The class script gives every streak weapon at
/// spawn with no ammo, so one that is not earned yet (empty) never comes out.
pub(super) fn next_earned_streak(ps: &AvatarState, sim_ws: &sim::SimWorld, held: u32) -> Option<u32> {
    let earned: Vec<u32> = streak_slot_weapons(ps)
        .filter(|&gun_id| ps.weapons.contains(&(gun_id as i32)))
        .filter(|&gun_id| {
            sim_ws
                .weapon_stats_entry(gun_id)
                .is_some_and(|facts| ammo_on_hand(ps, gun_id, &facts) > 0)
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
        WeaponSelect { index, retained_slot_position: held, life_no: life, ..Default::default() }
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
        assert_eq!(select.retained_slot_position, 12);
        assert_eq!(select.life_no, 3);
        assert_eq!(select.time, 900);
        assert_eq!(select.previous_main_weapon, 0);
    }

    #[test]
    fn a_new_life_starts_the_request_over_even_with_the_same_gun() {
        let mut gun_choice = selecting(12, 12, 3);
        gun_choice.previous_main_weapon = 5;
        gun_choice.time = 100;
        follow_held_weapon_select(&mut gun_choice, 12, 4, 800);
        assert_eq!((gun_choice.life_no, gun_choice.time, gun_choice.previous_main_weapon), (4, 800, 0));
    }

    #[test]
    fn a_request_the_player_already_holds_follows_the_held_gun() {
        let mut select = selecting(12, 12, 3);
        select.time = 100;
        follow_held_weapon_select(&mut select, 20, 3, 800);
        assert_eq!((select.index, select.mapped_index, select.retained_slot_position), (20, 20, 20));
        assert_eq!(select.time, 800);
    }

    #[test]
    fn a_request_still_in_flight_is_kept_while_another_gun_is_held() {
        let mut select = selecting(30, 12, 3);
        select.time = 100;
        follow_held_weapon_select(&mut select, 12, 3, 800);
        assert_eq!((select.index, select.time), (30, 100));
        assert_eq!(select.retained_slot_position, 12);
    }

    #[test]
    fn the_streak_slots_come_out_cheapest_first_and_skip_empty_ones() {
        let mut ps = AvatarState::ZERO;
        ps.dpad_kind = [1, 1, 0, 1];
        ps.action_slot_param = [40, 30, 99, 20];
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [20, 30, 40]);

        ps.action_slot_param[1] = 0;
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [20, 40]);

        ps.dpad_kind[3] = 2;
        assert_eq!(streak_slot_weapons(&ps).collect::<Vec<_>>(), [40]);
    }

    #[test]
    fn a_negative_slot_param_counts_as_no_weapon() {
        let mut ps = AvatarState::ZERO;
        ps.dpad_kind = [1, 0, 0, 0];
        ps.action_slot_param = [-5, 0, 0, 0];
        assert_eq!(streak_slot_weapons(&ps).count(), 0);
    }

    #[test]
    fn the_select_flags_copy_the_player_state_fields() {
        let mut ps = AvatarState::ZERO;
        ps.movement_state_bits = 0xc08;
        ps.weapon_state_bits = 0x880;
        ps.secondary_state_bits = 3;
        ps.e_flags = 9;
        ps.movement_mode = 2;
        let flags = select_flags(&ps);
        assert_eq!(
            (flags.movement_state_bits, flags.weapon_state_bits, flags.secondary_state_bits, flags.entity_state_bits, flags.movement_mode),
            (0xc08, 0x880, 3, 9, 2)
        );
    }
}
