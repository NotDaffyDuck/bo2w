//! The per-weapon tables the match runs on, built from the weapon registry:
//! index `n` of every table is weapon `n`, and index 0 is "no weapon".

use asset_core::AssetCategoryScope;
use asset_game::{WeaponChassisSummary, WeaponLedger};
use weapon_t6::combat_facts::{
    AimAssistRanges, RecordedBattleFeed, AbsentBattleProfile, GunBattleProfile,
};
use weapon_t6::damage::location_damage_is_valid;
use weapon_t6::fire_type::TriggerPattern;
use weapon_t6::hitloc::{GLOBAL_START, HITLOC_COUNT, location_table};

/// How long a burst weapon rests between bursts.
const BURST_COOLDOWN_MS: i32 = 200;

/// BO2's own `weapType_t` number for a knife or other melee-only weapon. The remapped
/// `gun_family_code` never carries it (melee folds into the bullet type), so the checks that need
/// it read `t6_weap_type`.
const WEAP_TYPE_MELEE: i32 = weapon_t6::facts::gun_family_code::CLOSE_COMBAT as i32;

/// One weapon index per table row, 0 included.
fn indices(weapons: &WeaponLedger) -> impl Iterator<Item = u32> + use<> {
    0..=u32::try_from(weapons.len()).unwrap_or(u32::MAX)
}

/// A segmented reload only counts when it has a start animation to play.
fn is_segmented(flag: bool, start_time_ms: i32) -> bool {
    flag && start_time_ms > 0
}

/// Rounds added per reload step: what the weapon says, else one at a time for
/// a segmented reload, else nothing (the clip is refilled whole).
fn reload_step(declared: i32, segmented: bool) -> i32 {
    match (declared > 0, segmented) {
        (true, _) => declared,
        (false, true) => 1,
        (false, false) => 0,
    }
}

/// The combat facts for one weapon body, or the first fact that is missing
/// or invalid. `lunge_has_clip` is whether the weapon (or its melee
/// weapon) has a charge animation.
pub(crate) fn checked_facts(
    body: WeaponChassisSummary,
    lunge_has_clip: bool,
    global_location: Option<[f32; HITLOC_COUNT]>,
) -> Result<GunBattleProfile, AbsentBattleProfile> {
    let trigger =
        TriggerPattern::from_i32(body.trigger_mode).map_err(|_| AbsentBattleProfile::UnrecognisedTriggerPattern)?;
    if let Some(multipliers) = body.location_damage_mult
        && !location_damage_is_valid(&multipliers)
    {
        return Err(AbsentBattleProfile::LocationDamage);
    }
    let staged_reload = is_segmented(body.staged_reload, body.refill_begin_millis);
    // BO2 picks the weapon's own table by its raw type: gas, bomb, mine and melee fall back to
    // the global table, which the remapped type would hide.
    let location_damage = location_table(
        body.t6_weap_type,
        global_location.unwrap_or(GLOBAL_START),
        body.location_damage_mult,
    );
    let mut facts = GunBattleProfile::try_from_captured(RecordedBattleFeed {
        // Selecting and switching
        draw_ms: body.draw_ms,
        holster_ms: body.holster_ms,
        fast_draw_ms: body.fast_draw_ms,
        fast_holster_ms: body.fast_holster_ms,
        run_lift_millis: body.run_lift_millis,
        run_lower_millis: body.run_lower_millis,
        alternate_weapon: 0,
        alternate_raise_time_ms: body.alternate_raise_time_ms,
        alternate_drop_time_ms: body.alternate_drop_time_ms,
        quick_switch: body.quick_switch,
        quick_switch_target: body.quick_switch_target,
        select_requires_ammo: body.select_requires_ammo,
        // What kind of weapon it is
        trigger_mode: body.trigger_mode,
        gun_family_code: body.gun_family_code,
        gun_kind: body.gun_kind,
        player_motion_kind: body.player_motion_kind,
        carry_slot_kind: body.carry_slot_kind,
        impact_kind_id: body.impact_kind_id,
        carries_over_talents: body.carries_over_talents,
        // Firing
        shot_gap_ms: body.shot_gap_ms,
        intro_fire_time_ms: body.intro_fire_time_ms,
        intro_fire_length: body.intro_fire_length,
        shot_lead_ms: body.shot_lead_ms,
        rounds_each_trigger: body.rounds_each_trigger,
        volley_rest_millis: if trigger.fires_volleys() {
            BURST_COOLDOWN_MS
        } else {
            0
        },
        manual_cycle: body.manual_cycle,
        cycle_ms: body.cycle_ms,
        bolt_rack_duration_millis: body.bolt_rack_duration_millis,
        bolt_rack_wait_millis: body.bolt_rack_wait_millis,
        rack_bolt_when_scoped: body.rack_bolt_when_scoped,
        offhand_hold_is_cancelable: body.offhand_hold_is_cancelable,
        // Ammunition and reloading
        magazine_capacity: body.magazine_capacity,
        spawn_rounds: body.opening_rounds(),
        reserve_cap: body.rounds_ceiling(),
        ammo_type_slot: body.ammo_type_slot,
        motion_clip_slot: body.motion_clip_slot,
        reload_duration_millis: body.reload_duration_millis,
        refill_dry_millis: body.refill_dry_millis,
        staged_reload,
        refill_begin_millis: body.refill_begin_millis,
        refill_finish_millis: body.refill_finish_millis,
        reload_rounds_gained: reload_step(body.reload_rounds_gained, staged_reload),
        reload_refill_millis: body.reload_refill_millis,
        empty_reload_refill_millis: body.empty_reload_refill_millis,
        reload_begin_refill_millis: body.reload_begin_refill_millis,
        reload_begin_bonus: body.reload_begin_bonus,
        forbid_topup_reload: body.forbid_topup_reload,
        dual_mag: body.dual_mag,
        reload_lowers_ads: body.reload_lowers_ads,
        position_reload_trans_ms: body.position_reload_trans_ms,
        // Damage
        damage: body.damage,
        hurt_floor: body.hurt_floor,
        full_damage_reach: body.full_damage_reach,
        minimum_damage_reach: body.minimum_damage_reach,
        damage_steps: body.damage_steps,
        location_damage,
        // Spread, aiming and kick
        spread_before_fire_add: body.spread_before_fire_add,
        hip_cone_up_lo: body.hip_cone_up_lo,
        hip_cone_crouch_lo: body.hip_cone_crouch_lo,
        hip_cone_lie_lo: body.hip_cone_lie_lo,
        hip_cone_up_hi: body.hip_cone_up_hi,
        hip_cone_crouch_hi: body.hip_cone_crouch_hi,
        hip_cone_lie_hi: body.hip_cone_lie_hi,
        hip_cone_settle: body.hip_cone_settle,
        hip_cone_per_shot: body.hip_cone_per_shot,
        hip_cone_per_turn: body.hip_cone_per_turn,
        hip_cone_per_step: body.hip_cone_per_step,
        hip_cone_crouch_settle: body.hip_cone_crouch_settle,
        hip_cone_lie_settle: body.hip_cone_lie_settle,
        aimed_cone: body.aimed_cone,
        scoped_aiming: body.scoped_aiming,
        block_aim_if_clip_dry: body.block_aim_if_clip_dry,
        scope_raise_speed: body.scope_raise_speed,
        scope_lower_speed: body.scope_lower_speed,
        fire_only_when_scoped: body.fire_only_when_scoped,
        optic_switch: body.optic_switch,
        hud_crosshair_layer: body.hud_crosshair_layer,
        aim_soft_kick_shots: body.kick.aimed_soft_kick_rounds,
        hip_soft_kick_shots: body.kick.hip_soft_kick_rounds,
        // Melee
        bash_hurt: body.bash_hurt,
        bash_ms: body.bash_ms,
        strike_wait_millis: body.strike_wait_millis,
        bash_windup_ms: body.bash_windup_ms,
        lunge_windup_wait_millis: body.lunge_windup_wait_millis,
        lunge_has_clip,
        blade_mesh: body.blade_mesh,
        charge_shot: body.charge_shot,
        unlimited_ammo: body.unlimited_ammo,
        flat_spread: body.flat_spread,
    })?;
    facts.retrievable = body.retrievable;
    Ok(facts)
}

/// A body whose melee comes from another weapon (a gun's knife): the melee
/// numbers are the knife's, and the knife's id goes in `blade_mesh`, which
/// being nonzero sends the gun through its quick raise after the swing.
fn with_melee_of(
    mut body: WeaponChassisSummary,
    knife: &WeaponChassisSummary,
    knife_id: u32,
) -> WeaponChassisSummary {
    body.bash_hurt = knife.bash_hurt;
    body.bash_ms = knife.bash_ms;
    body.strike_wait_millis = knife.strike_wait_millis;
    body.bash_windup_ms = knife.bash_windup_ms;
    body.lunge_windup_wait_millis = knife.lunge_windup_wait_millis;
    body.blade_mesh = knife_id;
    body
}

/// Whether weapon `id` has an animation in the melee-charge slot.
fn has_charge_anim(weapons: &WeaponLedger, number: u32) -> bool {
    const MELEE_CHARGE_SLOT: usize = 8;
    weapons
        .clips_for(number)
        .and_then(|anims| anims.get(MELEE_CHARGE_SLOT))
        .and_then(Option::as_ref)
        .is_some_and(|clip| !clip.is_empty())
}

/// One row of the combat table. A weapon whose facts are refused is added to
/// `refused` and gets the empty row.
fn combat_row(
    weapons: &WeaponLedger,
    id: u32,
    global_location: Option<[f32; HITLOC_COUNT]>,
    refused: &mut Vec<String>,
) -> GunBattleProfile {
    if id == 0 {
        return GunBattleProfile::none();
    }
    if !weapons.setup_known(id) {
        diag::warn!(
            Sim,
            "unsupported dual-hand animation configuration: {}",
            weapons.label_for(id)
        );
        return GunBattleProfile::none();
    }
    let Some(own) = weapons.summary_for(id) else {
        return GunBattleProfile::none();
    };
    let knife_id = weapons.melee_weapon_of(id);
    let knife = (knife_id != id)
        .then(|| weapons.summary_for(knife_id))
        .flatten();
    let (body, charge_anim) = match knife {
        Some(knife) => (
            with_melee_of(own, &knife, knife_id),
            has_charge_anim(weapons, knife_id),
        ),
        None => (own, has_charge_anim(weapons, id)),
    };
    let mut facts = match checked_facts(body, charge_anim, global_location) {
        Ok(facts) => facts,
        Err(why) => {
            refused.push(format!("{}({why:?})", weapons.label_for(id)));
            GunBattleProfile::none()
        }
    };
    facts.sprint_cycle_scale = own.bob.sprint_cycle_scale;
    facts.ducked_sprint_cycle_scale = own.bob.ducked_sprint_cycle_scale;
    facts.dtp_cycle_scale = own.bob.dtp_cycle_scale;
    facts.alternate_weapon = weapons.alternate_of(id);
    facts.dual_wield_weapon = weapons.dual_wield_weapon_of(id);
    facts.aim_assist = AimAssistRanges {
        auto_aim: own.auto_aim_range,
        hip: own.aim_assist_range,
        ads: own.aim_assist_range_ads,
    };
    facts
}

/// Combat facts for every weapon in the registry.
pub fn from_registry(
    weapons: &WeaponLedger,
    global_location: Option<[f32; HITLOC_COUNT]>,
) -> Vec<GunBattleProfile> {
    let mut refused = Vec::new();
    let table: Vec<_> = indices(weapons)
        .map(|id| combat_row(weapons, id, global_location, &mut refused))
        .collect();
    if !refused.is_empty() {
        diag::info!(
            Sim,
            "combat facts refused for {} weapons: {}",
            refused.len(),
            refused.join(" ")
        );
    }
    table
}

/// A melee weapon with nothing a gun has: no fire time, raise time or clip.
fn is_melee_only(gun_family_code: i32, fire_ms: i32, raise_ms: i32, magazine_capacity: i32) -> bool {
    gun_family_code == WEAP_TYPE_MELEE && fire_ms <= 0 && raise_ms <= 0 && magazine_capacity <= 0
}

/// Whether one weapon body is melee-only, by BO2's raw type (the remapped `gun_family_code`
/// folds melee into the bullet type).
fn body_is_melee_only(body: &WeaponChassisSummary) -> bool {
    is_melee_only(body.t6_weap_type, body.shot_gap_ms, body.draw_ms, body.magazine_capacity)
}

/// Each weapon's class and fire type by BO2's numbers, for the scripts that ask by name.
/// Row 0 (no weapon) and a weapon with no facts answer -1, "none".
pub fn raw_kinds_from_registry(weapons: &WeaponLedger) -> Vec<sim::WeaponRawKinds> {
    const NONE: sim::WeaponRawKinds = sim::WeaponRawKinds { gun_kind: -1, fire_mode: -1 };
    indices(weapons)
        .map(|id| match weapons.summary_for(id) {
            Some(body) if id != 0 => sim::WeaponRawKinds {
                gun_kind: body.t6_weap_class,
                fire_mode: body.t6_fire_type,
            },
            _ => NONE,
        })
        .collect()
}

/// Which weapons are melee-only (knives and the like).
pub fn melee_only_from_registry(weapons: &WeaponLedger) -> Vec<bool> {
    indices(weapons)
        .map(|id| weapons.summary_for(id).is_some_and(|body| body_is_melee_only(&body)))
        .collect()
}

/// The sounds the match scripts play for each weapon.
pub fn script_sounds_from_registry(weapons: &WeaponLedger) -> Vec<sim::WeaponScriptSounds> {
    indices(weapons)
        .map(|id| {
            let mut row =
                weapons
                    .audio_for(id)
                    .map_or_else(sim::WeaponScriptSounds::default, |sounds| {
                        sim::WeaponScriptSounds {
                            shoot: sounds.fire.clone(),
                            trigger_sound_name: sounds.trigger_sound_name.clone(),
                            grab: sounds.pickup.clone(),
                            pickup_sound_name: sounds.pickup_sound_name.clone(),
                            projectile_blast_fx: sounds.projectile_blast_fx.clone(),
                            proj_explosion_fx: None,
                        }
                    });
            row.proj_explosion_fx = weapons
                .effects_for(id)
                .and_then(|fx| fx.blast_clue.clone());
            row
        })
        .collect()
}

/// How far each weapon's bullets go through surfaces.
pub fn pen_from_registry(weapons: &WeaponLedger) -> Vec<weapon_t6::penetration::PierceCapability> {
    indices(weapons)
        .map(|id| {
            weapons.summary_for(id).map_or_else(
                weapon_t6::penetration::PierceCapability::default,
                |body| weapon_t6::penetration::PierceCapability {
                    wall_pierce_class: body.wall_pierce_class,
                    wall_pierce_scale: body.wall_pierce_scale,
                    long_gun_round: body.long_gun_round,
                    // The sim fills this in from the gun facts.
                    impact_explode: false,
                },
            )
        })
        .collect()
}

/// A projectile spins only when its weapon file says so, except for weapons
/// from outside the current game, which always spin.
fn spins(declared: bool, namespace: Option<AssetCategoryScope>) -> bool {
    declared || namespace != Some(AssetCategoryScope::T6)
}

/// The throwing knife sticks where it lands.
fn is_ballistic_blade(namespace: Option<AssetCategoryScope>, script_name: &str) -> bool {
    namespace.is_some() && script_name == "knife_ballistic_mp"
}

/// Grenades, mines and grenade-class projectiles hit the same target once
/// in a row.
fn limits_grenade_impacts(
    namespace: Option<AssetCategoryScope>,
    gun_family_code: i32,
    class_code: i32,
) -> bool {
    const WEAP_TYPE_GRENADE: i32 = 1;
    const WEAP_TYPE_PROJECTILE: i32 = 2;
    const WEAP_CLASS_GRENADE: i32 = 6;
    namespace == Some(AssetCategoryScope::T6)
        && (gun_family_code == WEAP_TYPE_GRENADE
            || (gun_family_code == WEAP_TYPE_PROJECTILE && class_code == WEAP_CLASS_GRENADE))
}

/// Runtime facts for throwables and projectiles, for every weapon.
pub fn equipment_from_registry(weapons: &WeaponLedger) -> Vec<sim::GadgetLiveStats> {
    indices(weapons)
        .map(|id| {
            let Some(body) = weapons.summary_for(id) else {
                return sim::GadgetLiveStats::default();
            };
            let scope = weapons.scope_for(id);
            sim::GadgetLiveStats {
                secondary_hand_category: body.secondary_hand_category,
                offhand_slot: body.offhand_slot,
                opening_ammo: body.opening_rounds(),
                magazine_capacity: body.magazine_capacity,
                gun_family_code: body.gun_family_code,
                class_code: body.gun_kind,
                collision_harm: body.damage,
                // Fuse and detonation
                detonation_delay_millis: body.detonation_delay_millis,
                trigger_hold_millis: body.trigger_hold_millis,
                grenade_cookoff_wait: body.grenade_cookoff_wait,
                has_detonator: body.has_detonator,
                detonate_delay_ms: body.detonate_delay_ms,
                detonate_time_ms: body.detonate_time_ms,
                timed_detonation: body.timed_detonation,
                explode_on_ground: body.explode_on_ground,
                shell_detonate_on_hit: body.shell_detonate_on_hit,
                bullet_impact_explode: body.bullet_impact_explode,
                force_bounce: body.force_bounce,
                laser_sensor: body.laser_sensor,
                // Flight
                projectile_rotates: spins(body.projectile_rotates, scope),
                projectile_rotate_type: body.projectile_rotate_type,
                missile_velocity: body.missile_velocity,
                missile_upward_velocity: body.missile_upward_velocity,
                projectile_speed_forward: body.projectile_speed_forward,
                projectile_speed_relative_up: body.projectile_speed_relative_up,
                projectile_parent_velocity: body.projectile_parent_velocity,
                shell_arming_range: body.shell_arming_range,
                shell_blast_kind: body.shell_blast_kind,
                missile_guidance: body.missile_guidance,
                max_steering_accel: body.max_steering_accel,
                ignition_delay_ms: body.ignition_delay_ms,
                require_lock_to_fire: body.require_lock_to_fire,
                // Landing and bouncing
                stickiness: body.stickiness,
                adheres_to_bodies: body.adheres_to_bodies,
                rolling_radius: body.rolling_radius,
                along_rebound: body.along_rebound,
                across_rebound: body.across_rebound,
                ballistic_blade: is_ballistic_blade(scope, &weapons.gsc_name_for(id)),
                limit_grenade_impacts: limits_grenade_impacts(
                    scope,
                    body.gun_family_code,
                    body.gun_kind,
                ),
                // Explosion
                blast_outer_reach: body.blast_outer_reach,
                blast_reach_floor: body.blast_reach_floor,
                blast_core_damage: body.blast_core_damage,
                blast_rim_damage: body.blast_rim_damage,
                damage_cone_angle: body.damage_cone_angle,
                spawned_grenade_weapon: weapons.spawned_grenade_weapon_of(id),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segmented_reload_needs_a_start_animation() {
        assert!(is_segmented(true, 400));
        assert!(!is_segmented(true, 0));
        assert!(!is_segmented(false, 400));
    }

    #[test]
    fn a_declared_reload_step_wins() {
        assert_eq!(reload_step(3, true), 3);
        assert_eq!(reload_step(3, false), 3);
    }

    #[test]
    fn an_undeclared_step_is_one_when_segmented_and_none_otherwise() {
        assert_eq!(reload_step(0, true), 1);
        assert_eq!(reload_step(0, false), 0);
        assert_eq!(reload_step(-2, false), 0);
    }

    #[test]
    fn a_fire_type_nobody_knows_is_refused() {
        let body = WeaponChassisSummary {
            trigger_mode: 9999,
            ..WeaponChassisSummary::default()
        };
        assert_eq!(
            checked_facts(body, false, None).err(),
            Some(AbsentBattleProfile::UnrecognisedTriggerPattern)
        );
    }

    #[test]
    fn a_gun_takes_its_melee_from_its_knife() {
        let gun = WeaponChassisSummary {
            bash_hurt: 1,
            bash_ms: 2,
            strike_wait_millis: 3,
            bash_windup_ms: 4,
            lunge_windup_wait_millis: 5,
            blade_mesh: 0,
            damage: 40,
            ..WeaponChassisSummary::default()
        };
        let knife = WeaponChassisSummary {
            bash_hurt: 100,
            bash_ms: 200,
            strike_wait_millis: 300,
            bash_windup_ms: 400,
            lunge_windup_wait_millis: 500,
            ..WeaponChassisSummary::default()
        };
        let merged = with_melee_of(gun, &knife, 77);
        assert_eq!(
            (
                merged.bash_hurt,
                merged.bash_ms,
                merged.strike_wait_millis,
                merged.bash_windup_ms,
                merged.lunge_windup_wait_millis,
                merged.blade_mesh
            ),
            (100, 200, 300, 400, 500, 77)
        );
        assert_eq!(merged.damage, 40);
    }

    #[test]
    fn melee_only_means_a_melee_type_with_no_gun_fields() {
        assert!(is_melee_only(WEAP_TYPE_MELEE, 0, 0, 0));
        assert!(!is_melee_only(WEAP_TYPE_MELEE, 50, 0, 0));
        assert!(!is_melee_only(WEAP_TYPE_MELEE, 0, 300, 0));
        assert!(!is_melee_only(WEAP_TYPE_MELEE, 0, 0, 30));
        assert!(!is_melee_only(0, 0, 0, 0));
    }

    #[test]
    fn projectiles_spin_when_declared_or_when_foreign() {
        assert!(spins(true, Some(AssetCategoryScope::T6)));
        assert!(!spins(false, Some(AssetCategoryScope::T6)));
        assert!(spins(false, Some(AssetCategoryScope::T5)));
        assert!(spins(false, None));
    }

    #[test]
    fn only_the_ballistic_knife_script_makes_a_ballistic_blade() {
        assert!(is_ballistic_blade(
            Some(AssetCategoryScope::T6),
            "knife_ballistic_mp"
        ));
        assert!(!is_ballistic_blade(Some(AssetCategoryScope::T6), "knife_mp"));
        assert!(!is_ballistic_blade(None, "knife_ballistic_mp"));
    }

    #[test]
    fn grenades_and_grenade_class_projectiles_limit_their_impacts() {
        let ns = Some(AssetCategoryScope::T6);
        assert!(limits_grenade_impacts(ns, 1, 0));
        assert!(limits_grenade_impacts(ns, 2, 6));
        assert!(!limits_grenade_impacts(ns, 2, 5));
        assert!(!limits_grenade_impacts(ns, 0, 6));
        assert!(!limits_grenade_impacts(Some(AssetCategoryScope::T5), 1, 0));
    }

    #[test]
    fn a_bo2_knife_is_melee_only_though_the_remap_files_it_under_bullet() {
        // What the T6 reader produces for a knife: raw type 7, remapped to 0.
        let knife = WeaponChassisSummary { t6_weap_type: 7, gun_family_code: 0, ..WeaponChassisSummary::default() };
        assert!(body_is_melee_only(&knife));
        let gun = WeaponChassisSummary { t6_weap_type: 0, gun_family_code: 0, magazine_capacity: 30, ..WeaponChassisSummary::default() };
        assert!(!body_is_melee_only(&gun));
        let bullet_shaped = WeaponChassisSummary { t6_weap_type: 0, ..WeaponChassisSummary::default() };
        assert!(!body_is_melee_only(&bullet_shaped));
    }

    #[test]
    fn an_empty_registry_has_just_the_no_weapon_row() {
        let registry = WeaponLedger::default();
        let combat = from_registry(&registry, None);
        assert_eq!(combat.len(), 1);
        assert!(!combat[0].ready_for_use());
        assert_eq!(melee_only_from_registry(&registry), [false]);
        assert_eq!(raw_kinds_from_registry(&registry).len(), 1);
        assert_eq!(raw_kinds_from_registry(&registry)[0].gun_kind, -1);
        assert_eq!(script_sounds_from_registry(&registry).len(), 1);
        assert_eq!(pen_from_registry(&registry).len(), 1);
        assert_eq!(equipment_from_registry(&registry).len(), 1);
    }
}
