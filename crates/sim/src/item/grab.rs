//! Taking a gun (or a scavenger bag) off the ground into a player's hands.

use entity_t6::EntityEventKind;
use entity_t6::Trajectory;
use entity_t6::trajectory::tr_type::STATIONARY;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};
use weapon_t6::ammo_table::{latch_weapon_dual_wield, num_hands_for_held, player_weapons_find_slot};
use weapon_t6::combat_facts::spawn_weapon_hand;

use super::inventory::{Rounds, current_primary, held_rounds, pour_into_reserve, primary_count, set_held_rounds, take_weapon};
use super::{DroppedItem, announce_pickup, has_scavenger_perk, launch, store_player};
use crate::ItemPickupRecord;
use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick, give_weapon_to_ps_akimbo, gsc_give_weapon_is_akimbo};

/// A player carrying two primaries who takes up a third puts the one in their hands down.
const PRIMARY_LIMIT: usize = 2;

/// Removes the item from the world and gives its entity number back.
fn lift_off_ground(world: &mut FrameWorld, number: i32, item: &DroppedItem) {
    world.remove_dropped_item_by_number(number);
    world.free_dynamic_entity_number(item.state.number);
}

/// The player sets their current primary down where the gun they are taking lay. Returns the new item's number, or
/// `ENTITYNUM_NONE` if it could not be placed (the player keeps the weapon then).
fn set_down_primary(world: &mut FrameWorld, walker: ClientId, weapon: u32, origin: [f32; 3], angles: [f32; 3]) -> i32 {
    let Some(ps) = world.player(walker).copied() else {
        return ENTITYNUM_NONE;
    };
    let Rounds { right, left, reserve } = held_rounds(world, &ps, weapon);
    let at_rest = |base| Trajectory { tr_type: STATIONARY, tr_base: base, ..Trajectory::default() };
    let number = launch::push_dropped_item(
        world,
        weapon,
        origin,
        at_rest(origin),
        at_rest(angles),
        walker.0 as i32,
        right,
        left,
        reserve,
        false,
        false,
    );
    if number != ENTITYNUM_NONE
        && let Some(ps) = world.player_mut(walker)
    {
        take_weapon(ps, weapon);
    }
    number
}

/// A weapon the player already has: both magazines and the reserve of the one on the ground go into its reserve.
fn top_up(world: &mut FrameWorld, walker: ClientId, mut next: PlayerState, weapon: u32, item: &DroppedItem) {
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon));
    latch_weapon_dual_wield(&next.weapons, &mut next.weapon_data, weapon, akimbo);
    if next.weapon == weapon {
        next.last_weapon_hand = num_hands_for_held(&next.weapons, &next.weapon_data, weapon);
    }
    let found = Rounds { right: item.clip_r, left: item.clip_l, reserve: item.stock };
    pour_into_reserve(world, &mut next, weapon, found);
    store_player(world, walker, next);
}

/// A weapon the player does not have: it comes into their hands, ready, with the rounds it was dropped with. A player
/// already carrying two primaries leaves their current one behind; returns its item number (or `ENTITYNUM_NONE`).
fn take_up(world: &mut FrameWorld, walker: ClientId, ps: &PlayerState, weapon: u32, item: &DroppedItem) -> i32 {
    let mut left_behind = ENTITYNUM_NONE;
    let current = current_primary(world, ps);
    if current != 0 && primary_count(world, ps) >= PRIMARY_LIMIT {
        left_behind = set_down_primary(world, walker, current, item.origin, item.state.lerp.apos.tr_base);
    }
    let Some(mut next) = world.player(walker).copied() else {
        return left_behind;
    };
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon));
    give_weapon_to_ps_akimbo(&mut next, weapon, akimbo);
    if let Some(facts) = world.combat_facts_for(weapon) {
        let hand = spawn_weapon_hand(weapon, &facts);
        next.weaponstate_primary = hand.weaponstate;
        next.weapon_time = hand.weapon_time;
        next.weapon_delay = hand.weapon_delay;
        next.weap_anim = hand.weap_anim;
        next.weaponstate_secondary = hand.weaponstate;
        next.weapon_time_secondary = hand.weapon_time;
        next.weapon_delay_secondary = hand.weapon_delay;
        next.weap_anim_secondary = hand.weap_anim;
    }
    set_held_rounds(world, &mut next, weapon, Rounds { right: item.clip_r, left: item.clip_l, reserve: item.stock });
    store_player(world, walker, next);
    left_behind
}

/// Brings the client's match record in step with what they now carry.
fn sync_client_record(world: &mut FrameWorld, walker: ClientId, weapon: u32, newly_carried: bool) {
    let Some(now) = world.player(walker).copied() else {
        return;
    };
    let held = held_rounds(world, &now, weapon);
    let meta = world.client_meta_mut(walker);
    meta.ammo_by_weapon.retain(|(id, _, _)| now.weapons.contains(&(*id as i32)));
    meta.taped_mag_spent.retain(|id| now.weapons.contains(&(*id as i32)));
    meta.set_ammo(weapon, held.right, held.reserve);
    meta.mirror_held_ammo(now.weapon);
    if newly_carried {
        meta.set_quick_reload_ready(weapon, true);
        meta.weapon_shot_count = 0;
        meta.burst_latch = false;
        meta.burst_latch_secondary = false;
        meta.rechamber_pending = false;
        meta.rechamber_pending_secondary = false;
        meta.t6_gun = None;
    }
}

/// The player takes the dropped gun `number`, as a touch or the use button decides. A scavenger bag goes to
/// `grab_scavenger`.
pub(crate) fn grab_number(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    if item.scavenger {
        grab_scavenger(world, walker, number);
        return;
    }
    let weapon = u32::try_from(item.state.index).unwrap_or(0);
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    if weapon == 0 {
        return;
    }

    let had_it = player_weapons_find_slot(&ps.weapons, weapon as i32) >= 0;
    lift_off_ground(world, number, &item);
    let swapped_entnum = if had_it {
        top_up(world, walker, ps, weapon, &item);
        ENTITYNUM_NONE
    } else {
        take_up(world, walker, &ps, weapon, &item)
    };
    sync_client_record(world, walker, weapon, !had_it);

    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon,
        from_entnum: item.state.number,
        clip_r: item.clip_r,
        clip_l: item.clip_l,
        stock: item.stock,
        swapped_entnum,
        picker_pm_type: ps.pm_type,
        ammo_only: false,
    });
    let tick = Tick((world.entity_kernel().level_time_ms() / crate::MATCH_TICK_MS as i32) as u32);
    announce_pickup(world, tick, EntityEventKind::ITEM_PICKUP, walker, weapon, ps.origin);
    perf::pickup(ps.pm_type);
}

/// A scavenger bag goes to a player with the perk; the refill itself is the script's.
pub(crate) fn grab_scavenger(world: &mut FrameWorld, walker: ClientId, number: i32) {
    let Some(item) = world.dropped_item_by_number(number) else {
        return;
    };
    let Some(ps) = world.player(walker).copied() else {
        return;
    };
    if !has_scavenger_perk(&ps) {
        return;
    }
    lift_off_ground(world, number, &item);
    world.item_pickups_mut().push(ItemPickupRecord {
        picker: walker.0 as i32,
        weapon: u32::try_from(item.state.index).unwrap_or(0),
        from_entnum: item.state.number,
        clip_r: 0,
        clip_l: 0,
        stock: 0,
        swapped_entnum: ENTITYNUM_NONE,
        picker_pm_type: ps.pm_type,
        ammo_only: false,
    });
    perf::pickup(ps.pm_type);
}
