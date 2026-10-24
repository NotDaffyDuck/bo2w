//! Changing the configuration of the weapon in hand (a different attachment set of the same
//! gun) without the player losing its ammunition: the new variant takes over the slot and as
//! much of the old ammunition as it can hold.

use playerstate_t6::game::{PlayerState, weap_flags};
use weapon_t6::ammo_table::{
    self, ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, set_ammo_not_in_clip,
    set_clip_for_hand,
};
use weapon_t6::combat_facts::{WeaponCombatFacts, spawn_weapon_hand};

use super::give::{AMMO_ROW_BYTES, CLIP_ROW_BYTES, clear_rows};
use super::is_alive;
use crate::ConfigurationChangeRejectReason as Reason;
use crate::frame::FrameWorld;
use crate::match_state::{EventAudience, SimEvent};
use crate::world::{ClientId, Tick, gsc_give_weapon_is_akimbo};

/// The keys of the rows a weapon uses in the ammo table and the clip table of a player.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RowKeys {
    ammo: i32,
    clip: i32,
}

impl RowKeys {
    fn of(facts: &WeaponCombatFacts, weapon: u32) -> Self {
        Self {
            ammo: ammo_table_key(facts.ammo_index, weapon),
            clip: clip_table_key(facts.clip_index, weapon),
        }
    }
}

/// Ammunition after the change: rounds in the first clip, in the second clip, and in the stock.
type Ammo = (i32, i32, i32);

/// Spreads the rounds the player has over the clips and the stock of the new variant: the clips
/// are filled in order up to `clip_capacity`, and what does not fit goes to the stock, which
/// never holds more than `max_ammo`.
fn configuration_change_ammo(
    clip0: i32,
    clip1: i32,
    stock: i32,
    clip_capacity: i32,
    max_ammo: i32,
    dual: bool,
) -> Ammo {
    let total = clip0
        .max(0)
        .saturating_add(clip1.max(0))
        .saturating_add(stock.max(0));
    let capacity = clip_capacity.max(0);
    let first = clip0.max(0).min(capacity).min(total);
    let second = if dual {
        clip1.max(0).min(capacity).min(total - first)
    } else {
        0
    };
    (first, second, (total - first - second).min(max_ammo.max(0)))
}

/// One request to turn the weapon `from` in hand into the variant `to`.
struct ChangeRequest {
    client: ClientId,
    from: u32,
    to: u32,
}

/// Carries out the change and tells the client how it went.
pub(super) fn apply_configuration_change(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    from: u32,
    to: u32,
) {
    let request = ChangeRequest { client: id, from, to };
    let event = match try_change(world, &request) {
        Ok(()) => SimEvent::ConfigurationChangeAccepted { request_id, from, to },
        Err(reason) => SimEvent::ConfigurationChangeRejected { request_id, from, to, reason },
    };
    world.push_event(tick, EventAudience::Client(id), event);
}

/// Everything is checked and worked out on a copy of the player; the player and the client
/// record are only touched once nothing can fail any more.
fn try_change(world: &mut FrameWorld, request: &ChangeRequest) -> Result<(), Reason> {
    let ChangeRequest { client: id, from, to, .. } = *request;
    if !is_alive(world, id) {
        return Err(Reason::NotAlive);
    }
    let ps = world.player(id).copied().ok_or(Reason::NotAlive)?;
    if ps.weapon != from {
        return Err(Reason::StaleSource);
    }
    if !world.can_transition_weapon(from, to) {
        return Err(Reason::DifferentFamily);
    }
    let (Some(old), Some(new)) = (world.combat_facts_for(from), world.combat_facts_for(to)) else {
        return Err(Reason::InvalidTarget);
    };
    if world.equipment_facts_for(to).is_some_and(|eq| eq.is_offhand()) {
        return Err(Reason::InvalidTarget);
    }
    if gun_is_busy(&ps) {
        return Err(Reason::Busy);
    }
    let slot = ps
        .weapons
        .iter()
        .position(|&weapon| weapon == from as i32)
        .ok_or(Reason::NoInventorySlot)?;
    if ps.weapons.contains(&(to as i32)) {
        return Err(Reason::InvalidTarget);
    }

    let (old_keys, new_keys) = (RowKeys::of(&old, from), RowKeys::of(&new, to));
    let shares_rows = |key: i32, clip: bool| rows_used_by_others(world, &ps, from, key, clip);
    let clip0 = get_clip_for_hand(&ps.ammoclip, old_keys.clip, 0);
    let raw_clip1 = get_clip_for_hand(&ps.ammoclip, old_keys.clip, 1);
    let clip1 = if ps.last_weapon_hand >= 1 { raw_clip1 } else { 0 };
    let stock = get_ammo_not_in_clip(&ps.ammo, old_keys.ammo);
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(to));
    let (next_clip0, next_clip1, next_stock) =
        configuration_change_ammo(clip0, clip1, stock, new.clip_size, new.max_ammo, akimbo);

    // Rows that another weapon of the player also uses cannot change under that weapon.
    let ammo_conflict = if old_keys.ammo != new_keys.ammo {
        shares_rows(old_keys.ammo, false) || shares_rows(new_keys.ammo, false)
    } else {
        next_stock != stock && shares_rows(old_keys.ammo, false)
    };
    let clip_conflict = if old_keys.clip != new_keys.clip {
        shares_rows(old_keys.clip, true) || shares_rows(new_keys.clip, true)
    } else {
        (next_clip0 != clip0 || next_clip1 != raw_clip1) && shares_rows(old_keys.clip, true)
    };
    if ammo_conflict || clip_conflict {
        return Err(Reason::SharedAmmoConflict);
    }

    let mut next = ps;
    next.weapons[slot] = to as i32;
    next.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    ammo_table::latch_weapon_dual_wield(&next.weapons, &mut next.weapon_data, to, akimbo);
    next.weapon = to;
    next.weapon_primary = to;
    next.last_weapon_hand = ammo_table::num_hands_for_held(&next.weapons, &next.weapon_data, to);
    retire_rows(&mut next, old_keys, new_keys);
    let stored = set_ammo_not_in_clip(&mut next.ammo, new_keys.ammo, next_stock)
        && set_clip_for_hand(&mut next.ammoclip, new_keys.clip, 0, next_clip0)
        && set_clip_for_hand(&mut next.ammoclip, new_keys.clip, 1, next_clip1);
    if !stored {
        return Err(Reason::AmmoTableFull);
    }

    let alternate = carry_alternate_ammo(world, &ps, &mut next, from, (&old, old_keys), (&new, new_keys))?;
    ready_the_new_variant(&mut next, to, &new);

    *world.player_mut(id).expect("validated alive player") = next;
    let meta = world.client_meta_mut(id);
    if let Some((weapon, clip, stock)) = alternate {
        meta.ammo_by_weapon.retain(|row| row.0 != old.alternate_weapon);
        meta.set_ammo(weapon, clip, stock);
    }
    let quick_reload_ready = meta.quick_reload_ready(from);
    meta.ammo_by_weapon.retain(|row| row.0 != from);
    meta.set_ammo(to, next_clip0, next_stock);
    meta.set_quick_reload_ready(from, true);
    meta.set_quick_reload_ready(to, quick_reload_ready);
    meta.mirror_held_ammo(to);
    meta.weapon_shot_count = 0;
    meta.burst_latch = false;
    meta.burst_latch_secondary = false;
    meta.rechamber_pending = false;
    meta.rechamber_pending_secondary = false;
    meta.t6_gun = None;
    Ok(())
}

/// Whether the gun is in the middle of something (reloading, firing, being raised, or the
/// player is using an offhand item), in which case the variant cannot be swapped.
fn gun_is_busy(ps: &PlayerState) -> bool {
    // The gun state counts as ready the way the old mirrored state did.
    let busy = |state: i32| !weapon_t6::state::is_ready_as_mirrored(state);
    busy(ps.weapon_state_t6)
        || ps.weapon_time > 0
        || (ps.last_weapon_hand >= 1 && (busy(ps.weapon_state_left_t6) || ps.weapon_time_secondary > 0))
        || ps.weap_flags & weap_flags::OFFHAND_VIEW != 0
}

/// Whether a weapon other than `except` that the player carries uses the ammo row (or, with
/// `clip`, the clip row) with this key.
fn rows_used_by_others(world: &FrameWorld, ps: &PlayerState, except: u32, key: i32, clip: bool) -> bool {
    ps.weapons
        .iter()
        .filter(|&&weapon| weapon > 0 && weapon != except as i32)
        .any(|&weapon| {
            world.combat_facts_for(weapon as u32).is_some_and(|facts| {
                let keys = RowKeys::of(&facts, weapon as u32);
                key == if clip { keys.clip } else { keys.ammo }
            })
        })
}

/// Empties the rows the old variant used, where the new variant uses different ones.
fn retire_rows(next: &mut PlayerState, old: RowKeys, new: RowKeys) {
    if old.ammo != new.ammo {
        clear_rows(&mut next.ammo, AMMO_ROW_BYTES, old.ammo);
    }
    if old.clip != new.clip {
        clear_rows(&mut next.ammoclip, CLIP_ROW_BYTES, old.clip);
    }
}

/// Variants whose underbarrel weapon is of the same kind carry the underbarrel ammunition over
/// as well. Returns what the underbarrel weapon holds afterwards, when it was carried over.
fn carry_alternate_ammo(
    world: &FrameWorld,
    ps: &PlayerState,
    next: &mut PlayerState,
    from: u32,
    (old, old_keys): (&WeaponCombatFacts, RowKeys),
    (new, new_keys): (&WeaponCombatFacts, RowKeys),
) -> Result<Option<(u32, i32, i32)>, Reason> {
    if old.alternate_weapon == 0 || new.alternate_weapon == 0 {
        return Ok(None);
    }
    let (Some(old_alt), Some(new_alt)) = (
        world.combat_facts_for(old.alternate_weapon),
        world.combat_facts_for(new.alternate_weapon),
    ) else {
        return Ok(None);
    };
    if old_alt.weap_type != new_alt.weap_type || old_alt.weap_class != new_alt.weap_class {
        return Ok(None);
    }
    let old_alt_keys = RowKeys::of(&old_alt, old.alternate_weapon);
    let new_alt_keys = RowKeys::of(&new_alt, new.alternate_weapon);
    let distinct_from_main = old_alt_keys.ammo != old_keys.ammo
        && old_alt_keys.clip != old_keys.clip
        && new_alt_keys.ammo != new_keys.ammo
        && new_alt_keys.clip != new_keys.clip;
    let present = ammo_table::ammo_row_present(&ps.ammo, old_alt_keys.ammo)
        && ammo_table::clip_row_present(&ps.ammoclip, old_alt_keys.clip);
    if !distinct_from_main || !present {
        return Ok(None);
    }

    if alternate_rows_are_shared(world, ps, from, old_alt_keys, new_alt_keys) {
        return Err(Reason::SharedAmmoConflict);
    }
    let (clip, _, stock) = configuration_change_ammo(
        get_clip_for_hand(&ps.ammoclip, old_alt_keys.clip, 0),
        0,
        get_ammo_not_in_clip(&ps.ammo, old_alt_keys.ammo),
        new_alt.clip_size,
        new_alt.max_ammo,
        false,
    );
    retire_rows(next, old_alt_keys, new_alt_keys);
    let stored = set_ammo_not_in_clip(&mut next.ammo, new_alt_keys.ammo, stock)
        && set_clip_for_hand(&mut next.ammoclip, new_alt_keys.clip, 0, clip);
    if !stored {
        return Err(Reason::AmmoTableFull);
    }
    Ok(Some((new.alternate_weapon, clip, stock)))
}

/// Whether any other weapon of the player (or its underbarrel weapon) uses a row that the
/// underbarrel change would move or empty.
fn alternate_rows_are_shared(
    world: &FrameWorld,
    ps: &PlayerState,
    except: u32,
    old: RowKeys,
    new: RowKeys,
) -> bool {
    let touches_change = |facts: &WeaponCombatFacts, weapon: u32| {
        let keys = RowKeys::of(facts, weapon);
        (old.ammo != new.ammo && (keys.ammo == old.ammo || keys.ammo == new.ammo))
            || (old.clip != new.clip && (keys.clip == old.clip || keys.clip == new.clip))
    };
    ps.weapons
        .iter()
        .filter(|&&weapon| weapon > 0 && weapon != except as i32)
        .any(|&weapon| {
            let underbarrel = world
                .combat_facts_for(weapon as u32)
                .map_or(0, |facts| facts.alternate_weapon);
            [weapon as u32, underbarrel]
                .into_iter()
                .filter(|&candidate| candidate != 0)
                .any(|candidate| {
                    world
                        .combat_facts_for(candidate)
                        .is_some_and(|facts| touches_change(&facts, candidate))
                })
        })
}

/// Puts the new variant in the ready state with nothing fired yet.
fn ready_the_new_variant(next: &mut PlayerState, weapon: u32, facts: &WeaponCombatFacts) {
    let ready = spawn_weapon_hand(weapon, facts);
    next.weaponstate_primary = ready.weaponstate;
    next.weapon_time = ready.weapon_time;
    next.weapon_delay = ready.weapon_delay;
    next.weap_anim = ready.weap_anim;
    next.weaponstate_secondary = ready.weaponstate;
    next.weapon_time_secondary = ready.weapon_time;
    next.weapon_delay_secondary = ready.weapon_delay;
    next.weap_anim_secondary = ready.weap_anim;
    next.weapon_shot_count = 0;
    next.weapon_shot_count_secondary = 0;
    next.weap_flags &= !(weap_flags::NO_ADS | weap_flags::DOUBLEBARREL_RECOIL | weap_flags::RECOIL_SCALE);
}
