//! The weapon part of the content digest: combat rows, which weapons may run, and the
//! equipment (throwable) rows.
//!
//! Each row's fields are fed in a fixed order that is part of the digest format, so runs of
//! fields of one type are listed together here without changing that order. Fields a row has
//! that are not named below do not take part in the digest.

use super::digest::Fnv1a;
use crate::EquipmentRuntimeFacts;
use weapon_t6::combat_facts::WeaponCombatFacts;

/// `Option<bool>` as one byte: unset, false or true.
fn tri_state(flag: Option<bool>) -> u8 {
    match flag {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

/// A present flag and then the value, or the flag and a zero when it is absent.
fn put_optional_f32(h: &mut Fnv1a, value: Option<f32>) {
    h.put(value.is_some());
    h.put(value.unwrap_or(0.0));
}

pub(super) fn combat_rows(h: &mut Fnv1a, rows: &[WeaponCombatFacts]) {
    h.put_len(rows.len());
    for row in rows {
        combat_row(h, row);
    }
}

fn combat_row(h: &mut Fnv1a, row: &WeaponCombatFacts) {
    // Timing of the weapon itself, up to the weapon it switches to.
    h.put_all([
        row.fire_time_ms,
        row.intro_fire_time_ms,
        row.intro_fire_length,
        row.fire_delay_ms,
        row.raise_time_ms,
        row.drop_time_ms,
    ]);
    h.put(row.alternate_weapon);
    h.put_all([
        row.alternate_raise_time_ms,
        row.alternate_drop_time_ms,
        row.quick_drop_time_ms,
        row.inventory_type,
        row.weap_class,
        row.player_anim_type,
    ]);
    h.put(tri_state(row.select_requires_ammo));
    h.put(tri_state(row.offhand_hold_is_cancelable));

    // Ammunition and the shape of one trigger pull.
    h.put_all([
        row.reload_time_ms,
        row.reload_empty_time_ms,
        row.clip_size,
        row.start_ammo,
        row.max_ammo,
        row.ammo_index,
        row.clip_index,
        row.fire_type,
        row.impact_type,
        row.shots_per_fire,
        row.burst_cooldown_ms,
    ]);
    h.put(row.bolt_action);
    h.put_all([row.rechamber_time_ms, row.rechamber_bolt_time_ms, row.rechamber_bolt_delay_ms]);
    h.put(row.segmented_reload);
    h.put_all([
        row.reload_start_time_ms,
        row.reload_end_time_ms,
        row.reload_ammo_add,
        row.reload_add_time_ms,
        row.reload_empty_add_time_ms,
        row.reload_start_add_time_ms,
        row.reload_start_add,
    ]);
    h.put(row.no_partial_reload);
    h.put(row.dual_mag.is_some());
    if let Some(times) = row.dual_mag {
        h.put_all([times.reload_ms, times.reload_empty_ms, times.add_ms, times.empty_add_ms]);
    }
    h.put_all([row.inherits_perks, row.spread_before_fire_add]);

    // Damage.
    h.put_all([row.sprint_raise_time_ms, row.sprint_drop_time_ms, row.damage, row.min_damage]);
    h.put_all([row.max_damage_range, row.min_damage_range]);
    h.put(row.damage_steps.is_some());
    if let Some(steps) = row.damage_steps {
        for (damage, range) in steps.damage.into_iter().zip(steps.range) {
            h.put(damage);
            h.put(range);
        }
        h.put(steps.range_scale);
    }

    // Spread and sights.
    h.put_all([
        row.hip_spread_stand_min,
        row.hip_spread_ducked_min,
        row.hip_spread_prone_min,
        row.hip_spread_stand_max,
        row.hip_spread_ducked_max,
        row.hip_spread_prone_max,
        row.hip_spread_decay_rate,
        row.hip_spread_fire_add,
        row.hip_spread_turn_add,
        row.hip_spread_move_add,
        row.hip_spread_ducked_decay,
        row.hip_spread_prone_decay,
        row.ads_spread,
    ]);
    h.put_all([row.aim_down_sight, row.no_ads_when_mag_empty]);
    h.put_all([row.ads_in_rate, row.ads_out_rate]);
    h.put_all([row.rechamber_while_ads, row.reload_lowers_ads]);
    h.put(row.position_reload_trans_ms);
    h.put(row.ads_fire_only);

    // Melee and the rest.
    h.put_all([
        row.melee_damage,
        row.overlay_reticle,
        row.melee_time_ms,
        row.melee_delay_ms,
        row.melee_charge_time_ms,
        row.melee_charge_delay_ms,
    ]);
    h.put(row.knife_model);
    h.put(row.quick_raise_time_ms);
    h.put_all(row.location_damage);
}

/// Which weapons may run at all, and the transition group of each.
pub(super) fn admission(h: &mut Fnv1a, runnable: &[bool], transition_groups: &[u32]) {
    h.put_len(runnable.len());
    h.put_all(runnable.iter().copied());
    h.put_len(transition_groups.len());
    h.put_all(transition_groups.iter().copied());
}

pub(super) fn equipment_rows(h: &mut Fnv1a, rows: &[EquipmentRuntimeFacts]) {
    h.put_len(rows.len());
    for row in rows {
        equipment_row(h, row);
    }
}

fn equipment_row(h: &mut Fnv1a, row: &EquipmentRuntimeFacts) {
    h.put_all([
        row.offhand_class,
        row.start_ammo,
        row.clip_size,
        row.impact_damage,
        row.fuse_time_ms,
        row.hold_fire_time_ms,
    ]);
    h.put_all([row.cook_off_hold, row.has_detonator]);
    h.put_all([row.detonate_delay_ms, row.detonate_time_ms]);
    h.put(row.projectile_rotates);
    h.put_all([row.projectile_rotate_type, row.stickiness]);
    h.put_all([
        row.timed_detonation,
        row.explode_on_ground,
        row.proj_impact_explode,
        row.laser_sensor,
        row.stick_to_players,
    ]);
    h.put_all([
        row.explosion_radius,
        row.explosion_radius_min,
        row.explosion_inner_damage,
        row.explosion_outer_damage,
    ]);
    h.put(row.damage_cone_angle);
    h.put(row.missile_guidance);
    h.put(row.max_steering_accel);
    h.put(row.ignition_delay_ms);
    h.put(row.require_lock_to_fire);
    h.put_all([
        row.projectile_speed,
        row.projectile_speed_up,
        row.projectile_speed_forward,
        row.projectile_speed_relative_up,
    ]);
    put_optional_f32(h, row.projectile_parent_velocity);
    put_optional_f32(h, row.rolling_radius);
    h.put_all([row.projectile_activate_dist, row.projectile_explosion_type]);
    h.put(row.spawned_grenade_weapon);
    h.put_all([row.weap_type, row.weap_class]);
    for bounce in [row.parallel_bounce, row.perpendicular_bounce] {
        h.put(bounce.is_some());
        if let Some(coefficients) = bounce {
            h.put_all(coefficients);
        }
    }
}
