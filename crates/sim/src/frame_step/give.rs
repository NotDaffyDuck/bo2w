//! Handing a weapon to a player who asked for one by number, and filling in the ammo tables of
//! the player's state for a weapon that was handed over.

use playerstate_t6::game::AvatarState;
use weapon_t6::ammo_table::{self, ammo_table_key, clip_table_key};
use weapon_t6::combat_facts::{GunBattleProfile, spawn_clip_stock, spawn_weapon_hand};

use crate::HandoutRefusal;
use crate::step_view::StepWorldView;
use crate::player_ledger::{EventRecipients, MatchOccurrence, PlayerMatchRecord};
use crate::sim_parts::{
    SeatNo, StepNo, grant_dual_wield_weapon, script_gun_wants_two_hands, pack_weapon,
};

/// Bytes of one ammo row (the key and the count) and of one clip row in the player state.
pub(super) const AMMO_ROW_BYTES: usize = 8;
pub(super) const CLIP_ROW_BYTES: usize = 12;

/// The inventory type of weapons that only borrow the grip from the weapon held before them.
const INVENTORY_TYPE_ALTERNATE: i32 = 3;

/// Offhand classes: lethal grenades and equipment share one slot, tactical items the other.
const LETHAL_CLASSES: [i32; 3] = [1, 4, 5];
const TACTICAL_CLASSES: [i32; 2] = [2, 3];

/// One request to be given a weapon, with the means to answer it.
struct GiveRequest {
    tick: StepNo,
    client: SeatNo,
    ask_no: u32,
    gun: u32,
}

impl GiveRequest {
    fn turn_down(&self, world: &mut StepWorldView, reason: HandoutRefusal) {
        let event = MatchOccurrence::HandoutRefused {
            request_id: self.ask_no,
            gun: self.gun,
            reason,
        };
        world.log_occurrence(self.tick, EventRecipients::Client(self.client), event);
    }

    fn accept(&self, world: &mut StepWorldView) {
        let event = MatchOccurrence::HandoutTaken {
            request_id: self.ask_no,
            gun: self.gun,
        };
        world.log_occurrence(self.tick, EventRecipients::Client(self.client), event);
    }
}

/// Gives `weapon` to the player, replacing the weapon in hand when the slots are full, and
/// tells the client whether it worked.
pub(super) fn apply_give_weapon(world: &mut StepWorldView, tick: StepNo, who_no: SeatNo, ticket: u32, gun_no: u32) {
    let request = GiveRequest { tick, client: who_no, ask_no: ticket, gun: gun_no };

    let stats = match check_give(world, &request) {
        Ok(stats) => stats,
        Err(reason) => return request.turn_down(world, reason),
    };
    if world.gadget_stats_lookup(gun_no).is_some_and(|eq| eq.is_secondary_gear()) {
        return give_offhand(world, &request, &stats);
    }
    if stats.shot_gap_ms <= 0 && stats.draw_ms <= 0 {
        return request.turn_down(world, HandoutRefusal::EmptyCombatProfile);
    }
    if world.weapon_script_label(gun_no).starts_with("killstreak_") {
        return give_streak_item(world, &request, &stats);
    }
    give_held_weapon(world, &request, &stats);
}

/// The reason a weapon cannot be handed out at all, or the facts that make it possible.
fn check_give(world: &StepWorldView, request: &GiveRequest) -> Result<GunBattleProfile, HandoutRefusal> {
    let gun = request.gun;
    if !super::is_alive(world, request.client) {
        return Err(HandoutRefusal::NotAlive);
    }
    if gun == 0 {
        return Err(HandoutRefusal::InvalidWeapon);
    }
    if gun as usize >= world.weapon_stats_count() {
        return Err(HandoutRefusal::NoSuchGun);
    }
    if !world.weapon_runnable(gun) {
        return Err(HandoutRefusal::UnsupportedWeapon);
    }
    world
        .damage_profile_of(gun)
        .ok_or(HandoutRefusal::NoSuchGun)
}

/// A scorestreak's item goes into the inventory beside what is held and is not equipped.
fn give_streak_item(world: &mut StepWorldView, request: &GiveRequest, facts: &GunBattleProfile) {
    let weapon = request.gun;
    let Some(pstate) = world.occupant_mut(request.client) else {
        return request.turn_down(world, HandoutRefusal::NotAlive);
    };
    pack_weapon(pstate, weapon, false);
    if !carries(pstate, weapon) {
        return request.turn_down(world, HandoutRefusal::InvalidWeapon);
    }
    let (clip, alt_magazine, stock) = spawn_clip_stock(facts, 0);
    seed_ps_ammo_tables(pstate, weapon, facts, clip, alt_magazine, false, stock);
    world.client_meta_mut(request.client).store_ammo_counts(weapon, clip, stock);
    request.accept(world);
}

/// An ordinary weapon: put it into the inventory (taking the slot of the weapon in hand when
/// nothing is free), make it the one in hand, and reset the state of the gun.
fn give_held_weapon(level: &mut StepWorldView, request: &GiveRequest, facts: &GunBattleProfile) {
    let (seat, weapon) = (request.client, request.gun);
    let akimbo = script_gun_wants_two_hands(level.weapon_script_label(weapon));
    let Some(mut next) = level.avatar(seat).copied() else {
        return request.turn_down(level, HandoutRefusal::NotAlive);
    };

    let outgoing = weapon_to_replace(level, &next);
    let swapped = replace_slot(&mut next, weapon, outgoing);
    if swapped {
        drop_unshared_ammo(level, &mut next, outgoing);
    }
    grant_dual_wield_weapon(&mut next, weapon, akimbo);
    *level.occupant_mut(seat).expect("validated alive player") = next;

    let (clip, stock) = level
        .occupant_mut(seat)
        .map_or((0, 0), |ps| arm_held_weapon(ps, weapon, facts));

    let meta = level.client_meta_mut(seat);
    if swapped {
        meta.rounds_per_gun.retain(|&(gun, _, _)| gun != outgoing);
        meta.set_quick_reload_ready(outgoing, true);
    }
    meta.store_ammo_counts(weapon, clip, stock);
    meta.set_quick_reload_ready(weapon, true);
    meta.copy_carried_ammo(weapon);
    fresh_trigger(meta);
    request.accept(level);
}

/// A gun handed over fresh: no shots counted, no burst held, nothing waiting to rechamber, and the weapon brain
/// rebuilt from the held weapons on the next tick.
fn fresh_trigger(meta: &mut PlayerMatchRecord) {
    let PlayerMatchRecord {
        shots_per_gun: weapon_shot_count, burst_held: burst_latch, burst_latch_secondary, bolt_due: rechamber_pending, rechamber_pending_secondary, t6_gun, ..
    } = meta;
    *weapon_shot_count = 0;
    (*burst_latch, *burst_latch_secondary) = (false, false);
    (*rechamber_pending, *rechamber_pending_secondary) = (false, false);
    *t6_gun = None;
}

/// Whether weapon sits in one of the player's weapon slots.
fn carries(ps: &AvatarState, gun: u32) -> bool {
    ps.weapons.contains(&(gun as i32))
}

/// The weapon that gives up its slot: the one in hand, or the one held before it when the one
/// in hand only borrows the grip.
fn weapon_to_replace(world: &StepWorldView, ps: &AvatarState) -> u32 {
    let borrows_grip = world
        .damage_profile_of(ps.weapon)
        .is_some_and(|facts| facts.carry_slot_kind == INVENTORY_TYPE_ALTERNATE);
    if borrows_grip && ps.weapons.contains(&(ps.weapon_primary as i32)) {
        ps.weapon_primary
    } else {
        ps.weapon
    }
}

/// Puts `weapon` into the slot of `outgoing` unless the player already carries it; whether a
/// slot was taken over.
fn replace_slot(ps: &mut AvatarState, gun: u32, outgoing: u32) -> bool {
    if carries(ps, gun) {
        return false;
    }
    let Some(slot) = ps.weapons.iter().position(|&w| w != 0 && w == outgoing as i32) else {
        return false;
    };
    ps.weapons[slot] = gun as i32;
    ps.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    true
}

/// Clears the ammo and clip rows `outgoing` used when no weapon the player still carries
/// shares them.
fn drop_unshared_ammo(world: &StepWorldView, ps: &mut AvatarState, outgoing: u32) {
    let facts = world.damage_profile_of(outgoing).expect("the weapon given up has combat facts");
    let carried: Vec<u32> = ps.weapons.iter().filter(|&&w| w > 0).map(|&w| w as u32).collect();
    let shared = |own_key: i32, key_of: &dyn Fn(&GunBattleProfile, u32) -> i32| {
        carried.iter().any(|&w| {
            world
                .damage_profile_of(w)
                .is_some_and(|f| key_of(&f, w) == own_key)
        })
    };

    let ammo_key = ammo_table_key(facts.ammo_type_slot, outgoing);
    if !shared(ammo_key, &|f, w| ammo_table_key(f.ammo_type_slot, w)) {
        clear_rows(&mut ps.ammo, AMMO_ROW_BYTES, ammo_key);
    }
    let clip_key = clip_table_key(facts.motion_clip_slot, outgoing);
    if !shared(clip_key, &|f, w| clip_table_key(f.motion_clip_slot, w)) {
        clear_rows(&mut ps.magazine_rows, CLIP_ROW_BYTES, clip_key);
    }
}

/// Zeroes every row of `table` that starts with `key`.
pub(super) fn clear_rows(table: &mut [u8], stride_len: usize, key: i32) {
    let tag = key.to_le_bytes();
    table.chunks_exact_mut(stride_len).filter(|row| row.starts_with(&tag)).for_each(|row| row.fill(0));
}

/// Equipment and grenades: they take the offhand slot of their class and replace what was there.
fn give_offhand(level: &mut StepWorldView, request: &GiveRequest, facts: &GunBattleProfile) {
    let (seat, weapon) = (request.client, request.gun);
    let Some(eq) = level.gadget_stats_lookup(weapon) else {
        return request.turn_down(level, HandoutRefusal::InvalidWeapon);
    };
    let Some(mut next) = level.avatar(seat).copied() else {
        return request.turn_down(level, HandoutRefusal::NotAlive);
    };
    let class = eq.secondary_hand_category;
    let same_slot_as_new = |other: i32| {
        (LETHAL_CLASSES.contains(&class) && LETHAL_CLASSES.contains(&other))
            || (TACTICAL_CLASSES.contains(&class) && TACTICAL_CLASSES.contains(&other))
    };
    for slot in &mut next.weapons {
        if *slot <= 0 || *slot == weapon as i32 {
            continue;
        }
        let shares = level
            .gadget_stats_lookup(*slot as u32)
            .is_some_and(|other| same_slot_as_new(other.secondary_hand_category));
        if shares {
            *slot = 0;
        }
    }
    pack_weapon(&mut next, weapon, false);
    if !carries(&next, weapon) {
        return request.turn_down(level, HandoutRefusal::InvalidWeapon);
    }
    if LETHAL_CLASSES.contains(&class) {
        next.secondary_gear_main = class;
    } else if TACTICAL_CLASSES.contains(&class) {
        next.secondary_gear_alt = class;
    } else if eq.select_class() == 3 {
        next.secondary_gear_alt = 3;
    }
    let clip = eq.starting_magazine_count();
    seed_ps_ammo_tables(&mut next, weapon, facts, clip, 0, false, 0);
    *level.occupant_mut(seat).expect("validated alive player") = next;

    let meta = level.client_meta_mut(seat);
    // Ammo rows survive only for guns still carried after the swap.
    meta.rounds_per_gun.retain(|&(gun, _, _)| carries(&next, gun));
    meta.store_ammo_counts(weapon, clip, 0);
    if let Some(loadout) = meta.kit.as_mut() {
        if LETHAL_CLASSES.contains(&class) {
            loadout.lethal = weapon;
        } else if TACTICAL_CLASSES.contains(&class) {
            loadout.tactical = weapon;
        }
    }
    request.accept(level);
}

/// Writes the ammo counts of a weapon into the ammo and clip tables of the player state:
/// `stock` outside the clip, `clip` in the main hand and `alt_magazine` in the other when `dual`.
pub(crate) fn seed_ps_ammo_tables(
    ps: &mut AvatarState,
    gun: u32,
    facts: &GunBattleProfile,
    clip: i32,
    alt_magazine: i32,
    dual: bool,
    stock: i32,
) {
    let ammo_type_slot = ammo_table_key(facts.ammo_type_slot, gun);
    let motion_clip_slot = clip_table_key(facts.motion_clip_slot, gun);
    if ammo_type_slot != 0 {
        let _ = ammo_table::set_ammo_not_in_clip(&mut ps.ammo, ammo_type_slot, stock);
    }
    if motion_clip_slot == 0 {
        return;
    }
    let _ = ammo_table::set_clip_for_hand(&mut ps.magazine_rows, motion_clip_slot, 0, clip);
    if dual {
        let _ = ammo_table::set_clip_for_hand(&mut ps.magazine_rows, motion_clip_slot, 1, alt_magazine);
    }
}

/// Puts `weapon` in the player's hands in its ready state with a full clip and the stock the
/// weapon starts with; returns the clip and the stock.
pub(crate) fn arm_held_weapon(pstate: &mut AvatarState, gun: u32, facts: &GunBattleProfile) -> (i32, i32) {
    pstate.weapon_primary = gun;
    let hands = ammo_table::num_hands_for_held(&pstate.weapons, &pstate.weapon_data, gun);
    pstate.previous_gun_hand = hands;
    let (clip, alt_magazine, stock) = spawn_clip_stock(facts, hands);

    let ready = spawn_weapon_hand(gun, facts);
    pstate.weaponstate_primary = ready.gun_phase;
    pstate.weapon_timer = ready.gun_timer_ms;
    pstate.weapon_ready_wait = ready.weapon_ready_wait;
    pstate.weapon_anim_id = ready.gun_clip;
    if hands >= 1 {
        pstate.weaponstate_secondary = ready.gun_phase;
        pstate.weapon_time_secondary = ready.gun_timer_ms;
        pstate.weapon_delay_secondary = ready.weapon_ready_wait;
        pstate.weap_anim_secondary = ready.gun_clip;
    }
    seed_ps_ammo_tables(pstate, gun, facts, clip, alt_magazine, hands >= 1, stock);
    (clip, stock)
}
