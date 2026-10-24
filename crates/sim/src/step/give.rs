//! Handing a weapon to a player who asked for one by number, and filling in the ammo tables of
//! the player's state for a weapon that was handed over.

use playerstate_t6::game::PlayerState;
use weapon_t6::ammo_table::{self, ammo_table_key, clip_table_key};
use weapon_t6::combat_facts::{WeaponCombatFacts, spawn_clip_stock, spawn_weapon_hand};

use crate::GiveRejectReason;
use crate::frame::FrameWorld;
use crate::match_state::{EventAudience, SimEvent};
use crate::world::{
    ClientId, Tick, give_weapon_to_ps_akimbo, gsc_give_weapon_is_akimbo, inventory_add_weapon,
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
    tick: Tick,
    client: ClientId,
    request_id: u32,
    weapon: u32,
}

impl GiveRequest {
    fn reject(&self, world: &mut FrameWorld, reason: GiveRejectReason) {
        let event = SimEvent::GiveRejected {
            request_id: self.request_id,
            weapon: self.weapon,
            reason,
        };
        world.push_event(self.tick, EventAudience::Client(self.client), event);
    }

    fn accept(&self, world: &mut FrameWorld) {
        let event = SimEvent::GiveAccepted {
            request_id: self.request_id,
            weapon: self.weapon,
        };
        world.push_event(self.tick, EventAudience::Client(self.client), event);
    }
}

/// Gives `weapon` to the player, replacing the weapon in hand when the slots are full, and
/// tells the client whether it worked.
pub(super) fn apply_give_weapon(world: &mut FrameWorld, tick: Tick, id: ClientId, request_id: u32, weapon: u32) {
    let request = GiveRequest { tick, client: id, request_id, weapon };

    let facts = match check_give(world, &request) {
        Ok(facts) => facts,
        Err(reason) => return request.reject(world, reason),
    };
    if world.equipment_facts_for(weapon).is_some_and(|eq| eq.is_offhand()) {
        return give_offhand(world, &request, &facts);
    }
    if facts.fire_time_ms <= 0 && facts.raise_time_ms <= 0 {
        return request.reject(world, GiveRejectReason::EmptyCombatProfile);
    }
    if world.weapon_script_name(weapon).starts_with("killstreak_") {
        return give_streak_item(world, &request, &facts);
    }
    give_held_weapon(world, &request, &facts);
}

/// The reason a weapon cannot be handed out at all, or the facts that make it possible.
fn check_give(world: &FrameWorld, request: &GiveRequest) -> Result<WeaponCombatFacts, GiveRejectReason> {
    let weapon = request.weapon;
    if !super::is_alive(world, request.client) {
        return Err(GiveRejectReason::NotAlive);
    }
    if weapon == 0 {
        return Err(GiveRejectReason::InvalidWeapon);
    }
    if weapon as usize >= world.weapon_combat_len() {
        return Err(GiveRejectReason::UnknownWeaponId);
    }
    if !world.weapon_runnable(weapon) {
        return Err(GiveRejectReason::UnsupportedWeapon);
    }
    world
        .combat_facts_for(weapon)
        .ok_or(GiveRejectReason::UnknownWeaponId)
}

/// A scorestreak's item goes into the inventory beside what is held and is not equipped.
fn give_streak_item(world: &mut FrameWorld, request: &GiveRequest, facts: &WeaponCombatFacts) {
    let weapon = request.weapon;
    let Some(ps) = world.player_mut(request.client) else {
        return request.reject(world, GiveRejectReason::NotAlive);
    };
    inventory_add_weapon(ps, weapon, false);
    if !ps.weapons.contains(&(weapon as i32)) {
        return request.reject(world, GiveRejectReason::InvalidWeapon);
    }
    let (clip, clip_alt, stock) = spawn_clip_stock(facts, 0);
    seed_ps_ammo_tables(ps, weapon, facts, clip, clip_alt, false, stock);
    world.client_meta_mut(request.client).set_ammo(weapon, clip, stock);
    request.accept(world);
}

/// An ordinary weapon: put it into the inventory (taking the slot of the weapon in hand when
/// nothing is free), make it the one in hand, and reset the state of the gun.
fn give_held_weapon(world: &mut FrameWorld, request: &GiveRequest, facts: &WeaponCombatFacts) {
    let (id, weapon) = (request.client, request.weapon);
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon));
    let Some(mut next) = world.player(id).copied() else {
        return request.reject(world, GiveRejectReason::NotAlive);
    };

    let outgoing = weapon_to_replace(world, &next);
    let replaced = replace_slot(&mut next, weapon, outgoing);
    if replaced {
        drop_unshared_ammo(world, &mut next, outgoing);
    }
    give_weapon_to_ps_akimbo(&mut next, weapon, akimbo);
    *world.player_mut(id).expect("validated alive player") = next;

    let (clip, stock) = world
        .player_mut(id)
        .map_or((0, 0), |ps| arm_held_weapon(ps, weapon, facts));

    let meta = world.client_meta_mut(id);
    if replaced {
        meta.ammo_by_weapon.retain(|row| row.0 != outgoing);
        meta.set_quick_reload_ready(outgoing, true);
    }
    meta.set_ammo(weapon, clip, stock);
    meta.set_quick_reload_ready(weapon, true);
    meta.mirror_held_ammo(weapon);
    meta.weapon_shot_count = 0;
    meta.burst_latch = false;
    meta.burst_latch_secondary = false;
    meta.rechamber_pending = false;
    meta.rechamber_pending_secondary = false;
    meta.t6_gun = None;
    request.accept(world);
}

/// The weapon that gives up its slot: the one in hand, or the one held before it when the one
/// in hand only borrows the grip.
fn weapon_to_replace(world: &FrameWorld, ps: &PlayerState) -> u32 {
    let borrows_grip = world
        .combat_facts_for(ps.weapon)
        .is_some_and(|facts| facts.inventory_type == INVENTORY_TYPE_ALTERNATE);
    if borrows_grip && ps.weapons.contains(&(ps.weapon_primary as i32)) {
        ps.weapon_primary
    } else {
        ps.weapon
    }
}

/// Puts `weapon` into the slot of `outgoing` unless the player already carries it; whether a
/// slot was taken over.
fn replace_slot(ps: &mut PlayerState, weapon: u32, outgoing: u32) -> bool {
    if ps.weapons.contains(&(weapon as i32)) {
        return false;
    }
    let Some(slot) = ps.weapons.iter().position(|&w| w != 0 && w == outgoing as i32) else {
        return false;
    };
    ps.weapons[slot] = weapon as i32;
    ps.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    true
}

/// Clears the ammo and clip rows `outgoing` used when no weapon the player still carries
/// shares them.
fn drop_unshared_ammo(world: &FrameWorld, ps: &mut PlayerState, outgoing: u32) {
    let facts = world
        .combat_facts_for(outgoing)
        .expect("held weapon combat facts");
    let carried: Vec<u32> = ps.weapons.iter().filter(|&&w| w > 0).map(|&w| w as u32).collect();
    let shared = |own_key: i32, key_of: &dyn Fn(&WeaponCombatFacts, u32) -> i32| {
        carried.iter().any(|&w| {
            world
                .combat_facts_for(w)
                .is_some_and(|f| key_of(&f, w) == own_key)
        })
    };

    let ammo_key = ammo_table_key(facts.ammo_index, outgoing);
    if !shared(ammo_key, &|f, w| ammo_table_key(f.ammo_index, w)) {
        clear_rows(&mut ps.ammo, AMMO_ROW_BYTES, ammo_key);
    }
    let clip_key = clip_table_key(facts.clip_index, outgoing);
    if !shared(clip_key, &|f, w| clip_table_key(f.clip_index, w)) {
        clear_rows(&mut ps.ammoclip, CLIP_ROW_BYTES, clip_key);
    }
}

/// Zeroes every row of `table` that starts with `key`.
pub(super) fn clear_rows(table: &mut [u8], row_bytes: usize, key: i32) {
    for row in table.chunks_exact_mut(row_bytes) {
        if row[..4] == key.to_le_bytes() {
            row.fill(0);
        }
    }
}

/// Equipment and grenades: they take the offhand slot of their class and replace what was there.
fn give_offhand(world: &mut FrameWorld, request: &GiveRequest, facts: &WeaponCombatFacts) {
    let (id, weapon) = (request.client, request.weapon);
    let Some(eq) = world.equipment_facts_for(weapon) else {
        return request.reject(world, GiveRejectReason::InvalidWeapon);
    };
    let Some(mut next) = world.player(id).copied() else {
        return request.reject(world, GiveRejectReason::NotAlive);
    };
    let class = eq.offhand_class;
    let same_slot_as_new = |other: i32| {
        (LETHAL_CLASSES.contains(&class) && LETHAL_CLASSES.contains(&other))
            || (TACTICAL_CLASSES.contains(&class) && TACTICAL_CLASSES.contains(&other))
    };
    for slot in &mut next.weapons {
        if *slot <= 0 || *slot == weapon as i32 {
            continue;
        }
        let shares = world
            .equipment_facts_for(*slot as u32)
            .is_some_and(|other| same_slot_as_new(other.offhand_class));
        if shares {
            *slot = 0;
        }
    }
    inventory_add_weapon(&mut next, weapon, false);
    if !next.weapons.contains(&(weapon as i32)) {
        return request.reject(world, GiveRejectReason::InvalidWeapon);
    }
    if LETHAL_CLASSES.contains(&class) {
        next.offhand_primary = class;
    } else if TACTICAL_CLASSES.contains(&class) {
        next.offhand_secondary = class;
    }
    let clip = eq.spawn_clip_count();
    seed_ps_ammo_tables(&mut next, weapon, facts, clip, 0, false, 0);
    *world.player_mut(id).expect("validated alive player") = next;

    let meta = world.client_meta_mut(id);
    meta.ammo_by_weapon
        .retain(|(held, _, _)| next.weapons.contains(&(*held as i32)));
    meta.set_ammo(weapon, clip, 0);
    if let Some(loadout) = meta.loadout.as_mut() {
        if LETHAL_CLASSES.contains(&class) {
            loadout.lethal = weapon;
        } else if TACTICAL_CLASSES.contains(&class) {
            loadout.tactical = weapon;
        }
    }
    request.accept(world);
}

/// Writes the ammo counts of a weapon into the ammo and clip tables of the player state:
/// `stock` outside the clip, `clip` in the main hand and `clip_alt` in the other when `dual`.
pub(crate) fn seed_ps_ammo_tables(
    ps: &mut PlayerState,
    weapon: u32,
    facts: &WeaponCombatFacts,
    clip: i32,
    clip_alt: i32,
    dual: bool,
    stock: i32,
) {
    let ammo_index = ammo_table_key(facts.ammo_index, weapon);
    let clip_index = clip_table_key(facts.clip_index, weapon);
    if ammo_index != 0 {
        let _ = ammo_table::set_ammo_not_in_clip(&mut ps.ammo, ammo_index, stock);
    }
    if clip_index == 0 {
        return;
    }
    let _ = ammo_table::set_clip_for_hand(&mut ps.ammoclip, clip_index, 0, clip);
    if dual {
        let _ = ammo_table::set_clip_for_hand(&mut ps.ammoclip, clip_index, 1, clip_alt);
    }
}

/// Puts `weapon` in the player's hands in its ready state with a full clip and the stock the
/// weapon starts with; returns the clip and the stock.
pub(crate) fn arm_held_weapon(ps: &mut PlayerState, weapon: u32, facts: &WeaponCombatFacts) -> (i32, i32) {
    ps.weapon_primary = weapon;
    let hands = ammo_table::num_hands_for_held(&ps.weapons, &ps.weapon_data, weapon);
    ps.last_weapon_hand = hands;
    let (clip, clip_alt, stock) = spawn_clip_stock(facts, hands);

    let ready = spawn_weapon_hand(weapon, facts);
    ps.weaponstate_primary = ready.weaponstate;
    ps.weapon_time = ready.weapon_time;
    ps.weapon_delay = ready.weapon_delay;
    ps.weap_anim = ready.weap_anim;
    if hands >= 1 {
        ps.weaponstate_secondary = ready.weaponstate;
        ps.weapon_time_secondary = ready.weapon_time;
        ps.weapon_delay_secondary = ready.weapon_delay;
        ps.weap_anim_secondary = ready.weap_anim;
    }
    seed_ps_ammo_tables(ps, weapon, facts, clip, clip_alt, hands >= 1, stock);
    (clip, stock)
}
