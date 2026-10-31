//! Taking a gun (or a scavenger bag) off the ground into a player's hands.

use entity_t6::EventCode;
use entity_t6::Trajectory;
use entity_t6::flight::path_kind::AT_REST;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState};
use weapon_t6::ammo_table::{latch_weapon_dual_wield, num_hands_for_held, player_weapons_find_slot};
use weapon_t6::combat_facts::spawn_weapon_hand;

use super::inventory::{Rounds, current_primary, held_rounds, pour_into_reserve, main_light_total, set_held_rounds, take_weapon};
use super::{LootOnGround, announce_pickup, has_scavenger_perk, launch, store_player};
use crate::GrabbedItemNote;
use crate::step_view::StepWorldView;
use crate::sim_parts::{SeatNo, StepNo, grant_dual_wield_weapon, script_gun_wants_two_hands};

/// A player carrying two primaries who takes up a third puts the one in their hands down.
const PRIMARY_LIMIT: usize = 2;

/// Removes the item from the world and gives its entity number back.
fn lift_off_ground(world: &mut StepWorldView, slot: i32, lifted: &LootOnGround) {
    let freed_number = lifted.state.slot_index;
    world.take_loot_at_slot(slot);
    world.free_dynamic_entity_number(freed_number);
}

/// The weapon index stored on a dropped item, 0 when it holds none.
fn weapon_on(item: &LootOnGround) -> u32 {
    u32::try_from(item.state.index).unwrap_or_default()
}

fn is_akimbo_weapon(world: &StepWorldView, gun: u32) -> bool {
    script_gun_wants_two_hands(world.weapon_script_label(gun))
}

/// A pickup log entry for `walker` taking `item`; `rounds` is what they gained and `swapped_entnum` the item they
/// left behind, if any.
fn pickup_record(walker: SeatNo, ps: &AvatarState, taken: &LootOnGround, gun: u32, rounds: Rounds, left_item: i32) -> GrabbedItemNote {
    let picker = walker.0 as i32;
    let Rounds { right, left, reserve } = rounds;
    GrabbedItemNote {
        swapped_entnum: left_item,
        picker,
        from_entnum: taken.state.slot_index,
        weapon: gun,
        clip_r: right,
        clip_l: left,
        stock: reserve,
        picker_pm_type: ps.movement_mode,
        ammo_only: false,
    }
}

/// The player sets their current primary down where the gun they are taking lay. Returns the new item's number, or
/// `NO_ENTITY_SLOT` if it could not be placed (the player keeps the weapon then).
fn set_down_primary(world: &mut StepWorldView, walker: SeatNo, gun: u32, spot: [f32; 3], aim: [f32; 3]) -> i32 {
    let Some(carrier) = world.avatar(walker).copied() else { return NO_ENTITY_SLOT };
    let Rounds { right, left, reserve } = held_rounds(world, &carrier, gun);
    let resting = |base| Trajectory { tr_type: AT_REST, base_point: base, ..Trajectory::default() };
    let owner_num = walker.0 as i32;
    let placed = launch::push_dropped_item(
        world,
        gun,
        spot,
        resting(spot),
        resting(aim),
        owner_num,
        right,
        left,
        reserve,
        false,
        false,
    );
    if placed == NO_ENTITY_SLOT {
        return placed;
    }
    if let Some(carrier_mut) = world.occupant_mut(walker) {
        take_weapon(carrier_mut, gun);
    }
    placed
}

/// A weapon the player already has: both magazines and the reserve of the one on the ground go into its reserve.
fn top_up(world: &mut StepWorldView, walker: SeatNo, mut after: AvatarState, gun: u32, item: &LootOnGround) {
    let akimbo = is_akimbo_weapon(world, gun);
    latch_weapon_dual_wield(&after.weapons, &mut after.weapon_data, gun, akimbo);
    if gun == after.weapon {
        after.previous_gun_hand = num_hands_for_held(&after.weapons, &after.weapon_data, gun);
    }
    let found = Rounds { right: item.right_mag, left: item.left_mag, reserve: item.spare };
    pour_into_reserve(world, &mut after, gun, found);
    store_player(world, walker, after);
}

/// A weapon the player does not have: it comes into their hands, ready, with the rounds it was dropped with. A player
/// already carrying two primaries leaves their current one behind; returns its item number (or `NO_ENTITY_SLOT`).
fn take_up(world: &mut StepWorldView, walker: SeatNo, ps: &AvatarState, gun: u32, entry: &LootOnGround) -> i32 {
    let in_hands = current_primary(world, ps);
    let must_drop = in_hands != 0 && main_light_total(world, ps) >= PRIMARY_LIMIT;
    let left_behind = must_drop
        .then(|| set_down_primary(world, walker, in_hands, entry.origin, entry.state.lerp.apos.base_point))
        .unwrap_or(NO_ENTITY_SLOT);
    let Some(mut next) = world.avatar(walker).copied() else {
        return left_behind;
    };
    grant_dual_wield_weapon(&mut next, gun, is_akimbo_weapon(world, gun));
    if let Some(table) = world.damage_profile_of(gun) {
        let grip = spawn_weapon_hand(gun, &table);
        let (state, time, delay, anim) = (grip.gun_phase, grip.gun_timer_ms, grip.weapon_ready_wait, grip.gun_clip);
        (next.weaponstate_primary, next.weapon_timer, next.weapon_ready_wait, next.weapon_anim_id) = (state, time, delay, anim);
        (next.weaponstate_secondary, next.weapon_time_secondary, next.weapon_delay_secondary, next.weap_anim_secondary) =
            (state, time, delay, anim);
    }
    let dropped_with = Rounds { right: entry.right_mag, left: entry.left_mag, reserve: entry.spare };
    set_held_rounds(world, &mut next, gun, dropped_with);
    store_player(world, walker, next);
    left_behind
}

/// Brings the client's match record in step with what they now carry.
fn sync_client_record(world: &mut StepWorldView, walker: SeatNo, gun: u32, newly_carried: bool) {
    let Some(now) = world.avatar(walker).copied() else {
        return;
    };
    let held = held_rounds(world, &now, gun);
    let record = world.client_meta_mut(walker);
    record.rounds_per_gun.retain(|(id, _, _)| now.weapons.contains(&(*id as i32)));
    record.taped_mag_spent.retain(|id| now.weapons.contains(&(*id as i32)));
    record.store_ammo_counts(gun, held.right, held.reserve);
    record.copy_carried_ammo(now.weapon);
    if !newly_carried {
        return;
    }
    record.set_quick_reload_ready(gun, true);
    record.shots_per_gun = 0;
    record.t6_gun = None;
    (record.burst_held, record.burst_latch_secondary) = (false, false);
    (record.bolt_due, record.rechamber_pending_secondary) = (false, false);
}

/// The player takes the dropped gun `number`, as a touch or the use button decides. A scavenger bag goes to
/// `grab_scavenger`.
pub(crate) fn grab_number(world: &mut StepWorldView, walker: SeatNo, slot_no: i32) {
    let Some(ground) = world.loot_at_slot(slot_no) else {
        return;
    };
    if ground.scavenge {
        return grab_scavenger(world, walker, slot_no);
    }
    let gun = weapon_on(&ground);
    let Some(taker) = world.avatar(walker).copied() else {
        return;
    };
    if gun == 0 {
        return;
    }

    let already_owned = player_weapons_find_slot(&taker.weapons, gun as i32) >= 0;
    lift_off_ground(world, slot_no, &ground);
    let mut left_item = NO_ENTITY_SLOT;
    if already_owned {
        top_up(world, walker, taker, gun, &ground);
    } else {
        left_item = take_up(world, walker, &taker, gun, &ground);
    }
    sync_client_record(world, walker, gun, !already_owned);

    let gained = Rounds { right: ground.right_mag, left: ground.left_mag, reserve: ground.spare };
    let record = pickup_record(walker, &taker, &ground, gun, gained, left_item);
    world.item_pickups_mut().push(record);
    let clock_millis = world.entity_kernel().level_time_ms();
    let tick = StepNo((clock_millis / crate::SIM_STEP_MILLIS as i32) as u32);
    announce_pickup(world, tick, EventCode::ITEM_PICKUP, walker, gun, taker.origin);
    perf::pickup(taker.movement_mode);
}

/// A scavenger bag goes to a player with the perk; the refill itself is the script's.
pub(crate) fn grab_scavenger(world: &mut StepWorldView, walker: SeatNo, slot_no: i32) {
    let Some(bag) = world.loot_at_slot(slot_no) else {
        return;
    };
    let Some(taker) = world.avatar(walker).copied() else {
        return;
    };
    if !has_scavenger_perk(&taker) {
        return;
    }
    lift_off_ground(world, slot_no, &bag);
    let nothing = Rounds { right: 0, left: 0, reserve: 0 };
    let record = pickup_record(walker, &taker, &bag, weapon_on(&bag), nothing, NO_ENTITY_SLOT);
    world.item_pickups_mut().push(record);
    perf::pickup(taker.movement_mode);
}
