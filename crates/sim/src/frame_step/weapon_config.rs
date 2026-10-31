//! Changing the configuration of the weapon in hand (a different attachment set of the same
//! gun) without the player losing its ammunition: the new variant takes over the slot and as
//! much of the old ammunition as it can hold.

use playerstate_t6::game::{AvatarState, weapon_state_bits};
use weapon_t6::ammo_table::{
    self, ammo_table_key, clip_table_key, get_ammo_not_in_clip, get_clip_for_hand, set_ammo_not_in_clip,
    set_clip_for_hand,
};
use weapon_t6::combat_facts::{GunBattleProfile, spawn_weapon_hand};

use super::give::{AMMO_ROW_BYTES, CLIP_ROW_BYTES, clear_rows};
use super::is_alive;
use crate::ConfigurationChangeRejectReason;
use crate::step_view::StepWorldView;
use crate::player_ledger::{EventRecipients, MatchOccurrence};
use crate::sim_parts::{SeatNo, StepNo, script_gun_wants_two_hands};

/// The keys of the rows a weapon uses in the ammo table and the clip table of a player.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RowKeys {
    ammo: i32,
    clip: i32,
}

impl RowKeys {
    fn of(facts: &GunBattleProfile, weapon: u32) -> Self {
        Self {
            ammo: ammo_table_key(facts.ammo_type_slot, weapon),
            clip: clip_table_key(facts.motion_clip_slot, weapon),
        }
    }
}

/// Spreads the rounds the player has over the clips and the stock of the new variant: the clips
/// are filled in order up to `clip_capacity`, and what does not fit goes to the stock, which
/// never holds more than `stock_cap`.
fn configuration_change_ammo(
    clip0: i32,
    clip1: i32,
    stock: i32,
    clip_capacity: i32,
    stock_cap: i32,
    dual: bool,
) -> (i32, i32, i32) {
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
    (first, second, (total - first - second).min(stock_cap.max(0)))
}

/// One request to turn the weapon `from` in hand into the variant `to`.
struct ChangeRequest {
    client: SeatNo,
    from: u32,
    to: u32,
}

/// Carries out the change and tells the client how it went.
pub(super) fn apply_configuration_change(
    level: &mut StepWorldView,
    step: StepNo,
    id: SeatNo,
    req_id: u32,
    from: u32,
    to: u32,
) {
    let request = ChangeRequest { client: id, from, to };
    let event = match try_change(level, &request) {
        Ok(()) => MatchOccurrence::ConfigurationChangeAccepted { request_id: req_id, from, to },
        Err(reason) => MatchOccurrence::ConfigurationChangeRejected { request_id: req_id, from, to, reason },
    };
    level.log_occurrence(step, EventRecipients::Client(id), event);
}

/// Everything is checked and worked out on a copy of the player; the player and the client
/// record are only touched once nothing can fail any more.
fn try_change(world: &mut StepWorldView, request: &ChangeRequest) -> Result<(), ConfigurationChangeRejectReason> {
    let ChangeRequest { client: id, from, to, .. } = *request;
    if !is_alive(world, id) {
        return Err(ConfigurationChangeRejectReason::NotAlive);
    }
    let ps = world.avatar(id).copied().ok_or(ConfigurationChangeRejectReason::NotAlive)?;
    if ps.weapon != from {
        return Err(ConfigurationChangeRejectReason::StaleSource);
    }
    if !world.can_transition_weapon(from, to) {
        return Err(ConfigurationChangeRejectReason::DifferentFamily);
    }
    let (Some(old), Some(new)) = (world.damage_profile_of(from), world.damage_profile_of(to)) else {
        return Err(ConfigurationChangeRejectReason::InvalidTarget);
    };
    if world.gadget_stats_lookup(to).is_some_and(|eq| eq.is_secondary_gear()) {
        return Err(ConfigurationChangeRejectReason::InvalidTarget);
    }
    if gun_is_busy(&ps) {
        return Err(ConfigurationChangeRejectReason::Busy);
    }
    let slot = ps
        .weapons
        .iter()
        .position(|&weapon| weapon == from as i32)
        .ok_or(ConfigurationChangeRejectReason::NoInventorySlot)?;
    if ps.weapons.contains(&(to as i32)) {
        return Err(ConfigurationChangeRejectReason::InvalidTarget);
    }

    let (old_keys, new_keys) = (RowKeys::of(&old, from), RowKeys::of(&new, to));
    let shares_rows = |key: i32, clip: bool| rows_used_by_others(world, &ps, from, key, clip);
    let clip0 = get_clip_for_hand(&ps.magazine_rows, old_keys.clip, 0);
    let raw_clip1 = get_clip_for_hand(&ps.magazine_rows, old_keys.clip, 1);
    let clip1 = if ps.previous_gun_hand >= 1 { raw_clip1 } else { 0 };
    let stock = get_ammo_not_in_clip(&ps.ammo, old_keys.ammo);
    let akimbo = script_gun_wants_two_hands(world.weapon_script_label(to));
    let (next_clip0, next_clip1, next_stock) =
        configuration_change_ammo(clip0, clip1, stock, new.magazine_capacity, new.reserve_cap, akimbo);

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
        return Err(ConfigurationChangeRejectReason::SharedAmmoConflict);
    }

    let mut updated = ps;
    updated.weapons[slot] = to as i32;
    let data_start = slot * 5;
    updated.weapon_data[data_start..data_start + 5].fill(0);
    ammo_table::latch_weapon_dual_wield(&updated.weapons, &mut updated.weapon_data, to, akimbo);
    updated.weapon = to;
    updated.weapon_primary = to;
    updated.previous_gun_hand = ammo_table::num_hands_for_held(&updated.weapons, &updated.weapon_data, to);
    retire_rows(&mut updated, old_keys, new_keys);
    let stored = set_ammo_not_in_clip(&mut updated.ammo, new_keys.ammo, next_stock)
        && set_clip_for_hand(&mut updated.magazine_rows, new_keys.clip, 0, next_clip0)
        && set_clip_for_hand(&mut updated.magazine_rows, new_keys.clip, 1, next_clip1);
    if !stored {
        return Err(ConfigurationChangeRejectReason::AmmoTableFull);
    }

    let alternate = carry_alternate_ammo(world, &ps, &mut updated, from, (&old, old_keys), (&new, new_keys))?;
    ready_the_new_variant(&mut updated, to, &new);

    let live_player = world.occupant_mut(id).expect("validated alive player");
    *live_player = updated;
    let record = world.client_meta_mut(id);
    if let Some(CarriedUnderbarrel { gun: alt_weapon, clip: alt_clip, stock: alt_stock }) = alternate {
        record.rounds_per_gun.retain(|row| row.0 != old.alternate_weapon);
        record.store_ammo_counts(alt_weapon, alt_clip, alt_stock);
    }
    let quick_reload_ready = record.quick_reload_ready(from);
    record.rounds_per_gun.retain(|row| row.0 != from);
    record.store_ammo_counts(to, next_clip0, next_stock);
    record.set_quick_reload_ready(from, true);
    record.set_quick_reload_ready(to, quick_reload_ready);
    record.copy_carried_ammo(to);
    record.shots_per_gun = 0;
    record.burst_held = false;
    record.burst_latch_secondary = false;
    record.bolt_due = false;
    record.rechamber_pending_secondary = false;
    record.t6_gun = None;
    Ok(())
}

/// Whether the gun is in the middle of something (reloading, firing, being raised, or the
/// player is using an offhand item), in which case the variant cannot be swapped.
fn gun_is_busy(ps: &AvatarState) -> bool {
    // The gun state counts as ready the way the old mirrored state did.
    let busy = |state: i32| !weapon_t6::state::is_ready_as_mirrored(state);
    busy(ps.weapon_state_t6)
        || ps.weapon_timer > 0
        || (ps.previous_gun_hand >= 1 && (busy(ps.weapon_state_left_t6) || ps.weapon_time_secondary > 0))
        || ps.weapon_state_bits & weapon_state_bits::OFFHAND_VIEW != 0
}

/// Whether a weapon other than `except` that the player carries uses the ammo row (or, with
/// `clip`, the clip row) with this key.
fn rows_used_by_others(world: &StepWorldView, pstate: &AvatarState, except: u32, key: i32, clip: bool) -> bool {
    pstate.weapons
        .iter()
        .filter(|&&weapon| weapon > 0 && weapon != except as i32)
        .any(|&weapon| {
            world.damage_profile_of(weapon as u32).is_some_and(|facts| {
                let keys = RowKeys::of(&facts, weapon as u32);
                key == if clip { keys.clip } else { keys.ammo }
            })
        })
}

/// Empties the rows the old variant used, where the new variant uses different ones.
fn retire_rows(next: &mut AvatarState, old: RowKeys, new: RowKeys) {
    if old.ammo != new.ammo {
        clear_rows(&mut next.ammo, AMMO_ROW_BYTES, old.ammo);
    }
    if old.clip != new.clip {
        clear_rows(&mut next.magazine_rows, CLIP_ROW_BYTES, old.clip);
    }
}

/// Variants whose underbarrel weapon is of the same kind carry the underbarrel ammunition over
/// as well. Returns what the underbarrel weapon holds afterwards, when it was carried over.
fn carry_alternate_ammo(
    world: &StepWorldView,
    ps: &AvatarState,
    next: &mut AvatarState,
    from: u32,
    (old, old_keys): (&GunBattleProfile, RowKeys),
    (new, new_keys): (&GunBattleProfile, RowKeys),
) -> Result<Option<CarriedUnderbarrel>, ConfigurationChangeRejectReason> {
    if old.alternate_weapon == 0 || new.alternate_weapon == 0 {
        return Ok(None);
    }
    let (Some(old_alt), Some(new_alt)) = (
        world.damage_profile_of(old.alternate_weapon),
        world.damage_profile_of(new.alternate_weapon),
    ) else {
        return Ok(None);
    };
    if old_alt.gun_family_code != new_alt.gun_family_code || old_alt.gun_kind != new_alt.gun_kind {
        return Ok(None);
    }
    let old_alt_keys = RowKeys::of(&old_alt, old.alternate_weapon);
    let new_alt_keys = RowKeys::of(&new_alt, new.alternate_weapon);
    let distinct_from_main = old_alt_keys.ammo != old_keys.ammo
        && old_alt_keys.clip != old_keys.clip
        && new_alt_keys.ammo != new_keys.ammo
        && new_alt_keys.clip != new_keys.clip;
    let present = ammo_table::ammo_row_present(&ps.ammo, old_alt_keys.ammo)
        && ammo_table::clip_row_present(&ps.magazine_rows, old_alt_keys.clip);
    if !distinct_from_main || !present {
        return Ok(None);
    }

    if alternate_rows_are_shared(world, ps, from, old_alt_keys, new_alt_keys) {
        return Err(ConfigurationChangeRejectReason::SharedAmmoConflict);
    }
    let (clip, _, stock) = configuration_change_ammo(
        get_clip_for_hand(&ps.magazine_rows, old_alt_keys.clip, 0),
        0,
        get_ammo_not_in_clip(&ps.ammo, old_alt_keys.ammo),
        new_alt.magazine_capacity,
        new_alt.reserve_cap,
        false,
    );
    retire_rows(next, old_alt_keys, new_alt_keys);
    let stored = set_ammo_not_in_clip(&mut next.ammo, new_alt_keys.ammo, stock)
        && set_clip_for_hand(&mut next.magazine_rows, new_alt_keys.clip, 0, clip);
    if !stored {
        return Err(ConfigurationChangeRejectReason::AmmoTableFull);
    }
    Ok(Some(CarriedUnderbarrel { gun: new.alternate_weapon, clip, stock }))
}

/// The underbarrel weapon and the rounds it holds after they were carried over.
struct CarriedUnderbarrel {
    gun: u32,
    clip: i32,
    stock: i32,
}

/// Whether any other weapon of the player (or its underbarrel weapon) uses a row that the
/// underbarrel change would move or empty.
fn alternate_rows_are_shared(
    world: &StepWorldView,
    pstate: &AvatarState,
    except: u32,
    old: RowKeys,
    new: RowKeys,
) -> bool {
    let touches_change = |facts: &GunBattleProfile, weapon: u32| {
        let keys = RowKeys::of(facts, weapon);
        (old.ammo != new.ammo && (keys.ammo == old.ammo || keys.ammo == new.ammo))
            || (old.clip != new.clip && (keys.clip == old.clip || keys.clip == new.clip))
    };
    pstate.weapons
        .iter()
        .filter(|&&weapon| weapon > 0 && weapon != except as i32)
        .any(|&weapon| {
            let underbarrel = world
                .damage_profile_of(weapon as u32)
                .map_or(0, |facts| facts.alternate_weapon);
            [weapon as u32, underbarrel]
                .into_iter()
                .filter(|&pick| pick != 0)
                .any(|pick| {
                    world
                        .damage_profile_of(pick)
                        .is_some_and(|facts| touches_change(&facts, pick))
                })
        })
}

/// Puts the new variant in the ready state with nothing fired yet.
fn ready_the_new_variant(next: &mut AvatarState, gun: u32, facts: &GunBattleProfile) {
    let ready = spawn_weapon_hand(gun, facts);
    next.weaponstate_primary = ready.gun_phase;
    next.weapon_timer = ready.gun_timer_ms;
    next.weapon_ready_wait = ready.weapon_ready_wait;
    next.weapon_anim_id = ready.gun_clip;
    next.weaponstate_secondary = ready.gun_phase;
    next.weapon_time_secondary = ready.gun_timer_ms;
    next.weapon_delay_secondary = ready.weapon_ready_wait;
    next.weap_anim_secondary = ready.gun_clip;
    next.shots_fired = 0;
    next.weapon_shot_count_secondary = 0;
    next.weapon_state_bits &= !(weapon_state_bits::AIM_DISABLED_BIT | weapon_state_bits::DOUBLEBARREL_RECOIL | weapon_state_bits::RECOIL_SCALE);
}
