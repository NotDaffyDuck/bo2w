//! The weapon part of the content digest: combat rows, which weapons may run, and the
//! equipment (throwable) rows.
//!
//! Each row's fields are fed in a fixed order that is part of the digest format, so runs of
//! fields of one type are listed together here without changing that order. Fields a row has
//! that are not named below do not take part in the digest.

use super::digest::Fnv1a;
use crate::GadgetLiveStats;
use weapon_t6::combat_facts::GunBattleProfile;

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

pub(super) fn combat_rows(h: &mut Fnv1a, rows: &[GunBattleProfile]) {
    h.put_len(rows.len());
    for entry in rows {
        combat_row(h, entry);
    }
}

fn combat_row(h: &mut Fnv1a, row: &GunBattleProfile) {
    // Timing of the weapon itself, up to the weapon it switches to.
    h.put_all([
        row.shot_gap_ms,
        row.intro_fire_time_ms,
        row.intro_fire_length,
        i32::from(row.charge_shot),
        row.shot_lead_ms,
        row.draw_ms,
        row.holster_ms,
    ]);
    h.put(row.alternate_weapon);
    h.put_all([
        row.alternate_raise_time_ms,
        row.alternate_drop_time_ms,
        row.fast_holster_ms,
        row.carry_slot_kind,
        row.gun_kind,
        row.player_motion_kind,
    ]);
    h.put(tri_state(row.select_requires_ammo));
    h.put(tri_state(row.offhand_hold_is_cancelable));

    // Ammunition and the shape of one trigger pull.
    h.put_all([
        row.reload_duration_millis,
        row.refill_dry_millis,
        row.magazine_capacity,
        row.spawn_rounds,
        row.reserve_cap,
        row.ammo_type_slot,
        row.motion_clip_slot,
        row.trigger_mode,
        row.impact_kind_id,
        row.rounds_each_trigger,
        row.volley_rest_millis,
    ]);
    h.put(row.manual_cycle);
    h.put_all([row.cycle_ms, row.bolt_rack_duration_millis, row.bolt_rack_wait_millis]);
    h.put(row.staged_reload);
    h.put_all([
        row.refill_begin_millis,
        row.refill_finish_millis,
        row.reload_rounds_gained,
        row.reload_refill_millis,
        row.empty_reload_refill_millis,
        row.reload_begin_refill_millis,
        row.reload_begin_bonus,
    ]);
    h.put(row.forbid_topup_reload);
    h.put(row.unlimited_ammo);
    h.put(row.dual_mag.is_some());
    if let Some(times) = row.dual_mag {
        h.put_all([times.reload_ms, times.reload_empty_ms, times.add_ms, times.empty_add_ms]);
    }
    h.put_all([row.carries_over_talents, row.spread_before_fire_add, row.flat_spread]);

    // Damage.
    h.put_all([row.run_lift_millis, row.run_lower_millis, row.damage, row.hurt_floor]);
    h.put_all([row.full_damage_reach, row.minimum_damage_reach]);
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
        row.hip_cone_up_lo,
        row.hip_cone_crouch_lo,
        row.hip_cone_lie_lo,
        row.hip_cone_up_hi,
        row.hip_cone_crouch_hi,
        row.hip_cone_lie_hi,
        row.hip_cone_settle,
        row.hip_cone_per_shot,
        row.hip_cone_per_turn,
        row.hip_cone_per_step,
        row.hip_cone_crouch_settle,
        row.hip_cone_lie_settle,
        row.aimed_cone,
    ]);
    h.put_all([row.scoped_aiming, row.block_aim_if_clip_dry]);
    h.put_all([row.scope_raise_speed, row.scope_lower_speed]);
    h.put_all([row.rack_bolt_when_scoped, row.reload_lowers_ads]);
    h.put(row.position_reload_trans_ms);
    h.put(row.fire_only_when_scoped);

    // Melee and the rest.
    h.put_all([
        row.bash_hurt,
        row.hud_crosshair_layer,
        row.bash_ms,
        row.strike_wait_millis,
        row.bash_windup_ms,
        row.lunge_windup_wait_millis,
    ]);
    h.put(row.blade_mesh);
    h.put(row.fast_draw_ms);
    h.put_all(row.location_damage);
}

/// Which weapons may run at all, and the transition group of each.
pub(super) fn admission(h: &mut Fnv1a, runnable: &[bool], transition_groups: &[u32]) {
    h.put_len(runnable.len());
    h.put_all(runnable.iter().copied());
    h.put_len(transition_groups.len());
    h.put_all(transition_groups.iter().copied());
}

pub(super) fn equipment_rows(h: &mut Fnv1a, rows: &[GadgetLiveStats]) {
    h.put_len(rows.len());
    for entry in rows {
        equipment_row(h, entry);
    }
}

fn equipment_row(h: &mut Fnv1a, row: &GadgetLiveStats) {
    h.put_all([
        row.secondary_hand_category,
        row.opening_ammo,
        row.magazine_capacity,
        row.collision_harm,
        row.detonation_delay_millis,
        row.trigger_hold_millis,
    ]);
    h.put_all([row.grenade_cookoff_wait, row.has_detonator]);
    h.put_all([row.detonate_delay_ms, row.detonate_time_ms]);
    h.put(row.projectile_rotates);
    h.put_all([row.projectile_rotate_type, row.stickiness]);
    h.put_all([
        row.timed_detonation,
        row.explode_on_ground,
        row.shell_detonate_on_hit,
        row.bullet_impact_explode,
        row.force_bounce,
        row.laser_sensor,
        row.adheres_to_bodies,
    ]);
    h.put_all([
        row.blast_outer_reach,
        row.blast_reach_floor,
        row.blast_core_damage,
        row.blast_rim_damage,
    ]);
    h.put(row.damage_cone_angle);
    h.put(row.missile_guidance);
    h.put(row.max_steering_accel);
    h.put(row.ignition_delay_ms);
    h.put(row.require_lock_to_fire);
    h.put_all([
        row.missile_velocity,
        row.missile_upward_velocity,
        row.projectile_speed_forward,
        row.projectile_speed_relative_up,
    ]);
    put_optional_f32(h, row.projectile_parent_velocity);
    put_optional_f32(h, row.rolling_radius);
    h.put_all([row.shell_arming_range, row.shell_blast_kind]);
    h.put(row.spawned_grenade_weapon);
    h.put_all([row.gun_family_code, row.class_code]);
    for bounce in [row.along_rebound, row.across_rebound] {
        h.put(bounce.is_some());
        if let Some(coefficients) = bounce {
            h.put_all(coefficients);
        }
    }
}
