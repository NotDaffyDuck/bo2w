//! The per-weapon tables the match runs on, built from the weapon registry:
//! index `n` of every table is weapon `n`, and index 0 is "no weapon".

use asset_core::AssetNamespace;
use asset_game::{WeaponBodyFacts, WeaponRegistry};
use weapon_t6::combat_facts::{
    AimAssistRanges, CapturedCombatInput, MissingCombatFacts, WeaponCombatFacts,
};
use weapon_t6::damage::location_damage_is_valid;
use weapon_t6::fire_type::FireType;
use weapon_t6::hitloc::{GLOBAL_START, HITLOC_COUNT, location_table};

/// How long a burst weapon rests between bursts.
const BURST_COOLDOWN_MS: i32 = 200;

/// The weapon type of a knife or other melee-only weapon.
const WEAP_TYPE_MELEE: i32 = 7;

/// One weapon index per table row, 0 included.
fn indices(weapons: &WeaponRegistry) -> impl Iterator<Item = u32> + use<> {
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
/// or invalid. `melee_charge_anim` is whether the weapon (or its melee
/// weapon) has a charge animation.
pub(crate) fn validated_facts(
    body: WeaponBodyFacts,
    melee_charge_anim: bool,
    global_location: Option<[f32; HITLOC_COUNT]>,
) -> Result<WeaponCombatFacts, MissingCombatFacts> {
    let fire_type =
        FireType::from_i32(body.fire_type).map_err(|_| MissingCombatFacts::UnknownFireType)?;
    if let Some(multipliers) = body.location_damage_mult
        && !location_damage_is_valid(&multipliers)
    {
        return Err(MissingCombatFacts::LocationDamage);
    }
    let segmented_reload = is_segmented(body.segmented_reload, body.reload_start_time_ms);
    let location_damage = location_table(
        body.weap_type,
        global_location.unwrap_or(GLOBAL_START),
        body.location_damage_mult,
    );
    WeaponCombatFacts::try_from_captured(CapturedCombatInput {
        // Selecting and switching
        raise_time_ms: body.raise_time_ms,
        drop_time_ms: body.drop_time_ms,
        quick_raise_time_ms: body.quick_raise_time_ms,
        quick_drop_time_ms: body.quick_drop_time_ms,
        sprint_raise_time_ms: body.sprint_raise_time_ms,
        sprint_drop_time_ms: body.sprint_drop_time_ms,
        alternate_weapon: 0,
        alternate_raise_time_ms: body.alternate_raise_time_ms,
        alternate_drop_time_ms: body.alternate_drop_time_ms,
        quick_switch: body.quick_switch,
        quick_switch_target: body.quick_switch_target,
        select_requires_ammo: body.select_requires_ammo,
        // What kind of weapon it is
        fire_type: body.fire_type,
        weap_type: body.weap_type,
        weap_class: body.weap_class,
        player_anim_type: body.player_anim_type,
        inventory_type: body.inventory_type,
        impact_type: body.impact_type,
        inherits_perks: body.inherits_perks,
        // Firing
        fire_time_ms: body.fire_time_ms,
        intro_fire_time_ms: body.intro_fire_time_ms,
        intro_fire_length: body.intro_fire_length,
        fire_delay_ms: body.fire_delay_ms,
        shots_per_fire: body.shots_per_fire,
        burst_cooldown_ms: if fire_type.is_burst() {
            BURST_COOLDOWN_MS
        } else {
            0
        },
        bolt_action: body.bolt_action,
        rechamber_time_ms: body.rechamber_time_ms,
        rechamber_bolt_time_ms: body.rechamber_bolt_time_ms,
        rechamber_bolt_delay_ms: body.rechamber_bolt_delay_ms,
        rechamber_while_ads: body.rechamber_while_ads,
        offhand_hold_is_cancelable: body.offhand_hold_is_cancelable,
        // Ammunition and reloading
        clip_size: body.clip_size,
        start_ammo: body.start_ammo_rounds(),
        max_ammo: body.max_ammo_rounds(),
        ammo_index: body.ammo_index,
        clip_index: body.clip_index,
        reload_time_ms: body.reload_time_ms,
        reload_empty_time_ms: body.reload_empty_time_ms,
        segmented_reload,
        reload_start_time_ms: body.reload_start_time_ms,
        reload_end_time_ms: body.reload_end_time_ms,
        reload_ammo_add: reload_step(body.reload_ammo_add, segmented_reload),
        reload_add_time_ms: body.reload_add_time_ms,
        reload_empty_add_time_ms: body.reload_empty_add_time_ms,
        reload_start_add_time_ms: body.reload_start_add_time_ms,
        reload_start_add: body.reload_start_add,
        no_partial_reload: body.no_partial_reload,
        dual_mag: body.dual_mag,
        reload_lowers_ads: body.reload_lowers_ads,
        position_reload_trans_ms: body.position_reload_trans_ms,
        // Damage
        damage: body.damage,
        min_damage: body.min_damage,
        max_damage_range: body.max_damage_range,
        min_damage_range: body.min_damage_range,
        damage_steps: body.damage_steps,
        location_damage,
        // Spread, aiming and kick
        spread_before_fire_add: body.spread_before_fire_add,
        hip_spread_stand_min: body.hip_spread_stand_min,
        hip_spread_ducked_min: body.hip_spread_ducked_min,
        hip_spread_prone_min: body.hip_spread_prone_min,
        hip_spread_stand_max: body.hip_spread_stand_max,
        hip_spread_ducked_max: body.hip_spread_ducked_max,
        hip_spread_prone_max: body.hip_spread_prone_max,
        hip_spread_decay_rate: body.hip_spread_decay_rate,
        hip_spread_fire_add: body.hip_spread_fire_add,
        hip_spread_turn_add: body.hip_spread_turn_add,
        hip_spread_move_add: body.hip_spread_move_add,
        hip_spread_ducked_decay: body.hip_spread_ducked_decay,
        hip_spread_prone_decay: body.hip_spread_prone_decay,
        ads_spread: body.ads_spread,
        aim_down_sight: body.aim_down_sight,
        no_ads_when_mag_empty: body.no_ads_when_mag_empty,
        ads_in_rate: body.ads_in_rate,
        ads_out_rate: body.ads_out_rate,
        ads_fire_only: body.ads_fire_only,
        optic_switch: body.optic_switch,
        overlay_reticle: body.overlay_reticle,
        ads_gun_kick_reduced_kick_bullets: body.kick.ads_gun_kick_reduced_kick_bullets,
        hip_gun_kick_reduced_kick_bullets: body.kick.hip_gun_kick_reduced_kick_bullets,
        // Melee
        melee_damage: body.melee_damage,
        melee_time_ms: body.melee_time_ms,
        melee_delay_ms: body.melee_delay_ms,
        melee_charge_time_ms: body.melee_charge_time_ms,
        melee_charge_delay_ms: body.melee_charge_delay_ms,
        melee_charge_anim,
        knife_model: body.knife_model,
    })
}

/// A body whose melee comes from another weapon (a gun's knife): the melee
/// numbers are the knife's, and the knife's id goes in `knife_model`, which
/// being nonzero sends the gun through its quick raise after the swing.
fn with_melee_of(
    mut body: WeaponBodyFacts,
    knife: &WeaponBodyFacts,
    knife_id: u32,
) -> WeaponBodyFacts {
    body.melee_damage = knife.melee_damage;
    body.melee_time_ms = knife.melee_time_ms;
    body.melee_delay_ms = knife.melee_delay_ms;
    body.melee_charge_time_ms = knife.melee_charge_time_ms;
    body.melee_charge_delay_ms = knife.melee_charge_delay_ms;
    body.knife_model = knife_id;
    body
}

/// Whether weapon `id` has an animation in the melee-charge slot.
fn has_charge_anim(weapons: &WeaponRegistry, id: u32) -> bool {
    const MELEE_CHARGE_SLOT: usize = 8;
    weapons
        .sz_xanims_of(id)
        .and_then(|anims| anims.get(MELEE_CHARGE_SLOT))
        .and_then(Option::as_ref)
        .is_some_and(|name| !name.is_empty())
}

/// One row of the combat table. A weapon whose facts are refused is added to
/// `refused` and gets the empty row.
fn combat_row(
    weapons: &WeaponRegistry,
    id: u32,
    global_location: Option<[f32; HITLOC_COUNT]>,
    refused: &mut Vec<String>,
) -> WeaponCombatFacts {
    if id == 0 {
        return WeaponCombatFacts::none();
    }
    if !weapons.configuration_supported(id) {
        diag::warn!(
            Sim,
            "unsupported dual-hand animation configuration: {}",
            weapons.name_of(id)
        );
        return WeaponCombatFacts::none();
    }
    let Some(own) = weapons.facts_of(id) else {
        return WeaponCombatFacts::none();
    };
    let knife_id = weapons.melee_weapon_of(id);
    let knife = (knife_id != id)
        .then(|| weapons.facts_of(knife_id))
        .flatten();
    let (body, charge_anim) = match knife {
        Some(knife) => (
            with_melee_of(own, &knife, knife_id),
            has_charge_anim(weapons, knife_id),
        ),
        None => (own, has_charge_anim(weapons, id)),
    };
    let mut facts = match validated_facts(body, charge_anim, global_location) {
        Ok(facts) => facts,
        Err(reason) => {
            refused.push(format!("{}({reason:?})", weapons.name_of(id)));
            WeaponCombatFacts::none()
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
    weapons: &WeaponRegistry,
    global_location: Option<[f32; HITLOC_COUNT]>,
) -> Vec<WeaponCombatFacts> {
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
fn is_melee_only(weap_type: i32, fire_time_ms: i32, raise_time_ms: i32, clip_size: i32) -> bool {
    weap_type == WEAP_TYPE_MELEE && fire_time_ms <= 0 && raise_time_ms <= 0 && clip_size <= 0
}

/// Which weapons are melee-only (knives and the like).
pub fn melee_only_from_registry(weapons: &WeaponRegistry) -> Vec<bool> {
    indices(weapons)
        .map(|id| {
            weapons.facts_of(id).is_some_and(|body| {
                is_melee_only(
                    body.weap_type,
                    body.fire_time_ms,
                    body.raise_time_ms,
                    body.clip_size,
                )
            })
        })
        .collect()
}

/// The sounds the match scripts play for each weapon.
pub fn script_sounds_from_registry(weapons: &WeaponRegistry) -> Vec<sim::WeaponScriptSounds> {
    indices(weapons)
        .map(|id| {
            let mut row =
                weapons
                    .sounds_of(id)
                    .map_or_else(sim::WeaponScriptSounds::default, |sounds| {
                        sim::WeaponScriptSounds {
                            fire: sounds.fire.clone(),
                            fire_player: sounds.fire_player.clone(),
                            pickup: sounds.pickup.clone(),
                            pickup_player: sounds.pickup_player.clone(),
                            proj_explosion: sounds.proj_explosion.clone(),
                            proj_explosion_fx: None,
                        }
                    });
            row.proj_explosion_fx = weapons
                .combat_fx_of(id)
                .and_then(|fx| fx.explosion_hint.clone());
            row
        })
        .collect()
}

/// How far each weapon's bullets go through surfaces.
pub fn pen_from_registry(weapons: &WeaponRegistry) -> Vec<weapon_t6::penetration::BulletPenFacts> {
    indices(weapons)
        .map(|id| {
            weapons.facts_of(id).map_or_else(
                weapon_t6::penetration::BulletPenFacts::default,
                |body| weapon_t6::penetration::BulletPenFacts {
                    penetrate_type: body.penetrate_type,
                    penetrate_multiplier: body.penetrate_multiplier,
                    rifle_bullet: body.rifle_bullet,
                    // The sim fills this in from the gun facts.
                    impact_explode: false,
                },
            )
        })
        .collect()
}

/// A projectile spins only when its weapon file says so, except for weapons
/// from outside the current game, which always spin.
fn spins(declared: bool, namespace: Option<AssetNamespace>) -> bool {
    declared || namespace != Some(AssetNamespace::T6)
}

/// The throwing knife sticks where it lands.
fn is_ballistic_blade(namespace: Option<AssetNamespace>, script_name: &str) -> bool {
    namespace.is_some() && script_name == "knife_ballistic_mp"
}

/// Grenades, mines and grenade-class projectiles hit the same target once
/// in a row.
fn limits_grenade_impacts(
    namespace: Option<AssetNamespace>,
    weap_type: i32,
    weap_class: i32,
) -> bool {
    const WEAP_TYPE_GRENADE: i32 = 1;
    const WEAP_TYPE_PROJECTILE: i32 = 2;
    const WEAP_CLASS_GRENADE: i32 = 6;
    namespace == Some(AssetNamespace::T6)
        && (weap_type == WEAP_TYPE_GRENADE
            || (weap_type == WEAP_TYPE_PROJECTILE && weap_class == WEAP_CLASS_GRENADE))
}

/// Runtime facts for throwables and projectiles, for every weapon.
pub fn equipment_from_registry(weapons: &WeaponRegistry) -> Vec<sim::EquipmentRuntimeFacts> {
    indices(weapons)
        .map(|id| {
            let Some(body) = weapons.facts_of(id) else {
                return sim::EquipmentRuntimeFacts::default();
            };
            let namespace = weapons.namespace_of(id);
            sim::EquipmentRuntimeFacts {
                offhand_class: body.offhand_class,
                start_ammo: body.start_ammo_rounds(),
                clip_size: body.clip_size,
                weap_type: body.weap_type,
                weap_class: body.weap_class,
                impact_damage: body.damage,
                // Fuse and detonation
                fuse_time_ms: body.fuse_time_ms,
                hold_fire_time_ms: body.hold_fire_time_ms,
                cook_off_hold: body.cook_off_hold,
                has_detonator: body.has_detonator,
                detonate_delay_ms: body.detonate_delay_ms,
                detonate_time_ms: body.detonate_time_ms,
                timed_detonation: body.timed_detonation,
                explode_on_ground: body.explode_on_ground,
                proj_impact_explode: body.proj_impact_explode,
                laser_sensor: body.laser_sensor,
                // Flight
                projectile_rotates: spins(body.projectile_rotates, namespace),
                projectile_rotate_type: body.projectile_rotate_type,
                projectile_speed: body.projectile_speed,
                projectile_speed_up: body.projectile_speed_up,
                projectile_speed_forward: body.projectile_speed_forward,
                projectile_speed_relative_up: body.projectile_speed_relative_up,
                projectile_parent_velocity: body.projectile_parent_velocity,
                projectile_activate_dist: body.projectile_activate_dist,
                projectile_explosion_type: body.projectile_explosion_type,
                missile_guidance: body.missile_guidance,
                max_steering_accel: body.max_steering_accel,
                ignition_delay_ms: body.ignition_delay_ms,
                require_lock_to_fire: body.require_lock_to_fire,
                // Landing and bouncing
                stickiness: body.stickiness,
                stick_to_players: body.stick_to_players,
                rolling_radius: body.rolling_radius,
                parallel_bounce: body.parallel_bounce,
                perpendicular_bounce: body.perpendicular_bounce,
                ballistic_blade: is_ballistic_blade(namespace, &weapons.script_name_of(id)),
                limit_grenade_impacts: limits_grenade_impacts(
                    namespace,
                    body.weap_type,
                    body.weap_class,
                ),
                // Explosion
                explosion_radius: body.explosion_radius,
                explosion_radius_min: body.explosion_radius_min,
                explosion_inner_damage: body.explosion_inner_damage,
                explosion_outer_damage: body.explosion_outer_damage,
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
        let body = WeaponBodyFacts {
            fire_type: 9999,
            ..WeaponBodyFacts::default()
        };
        assert_eq!(
            validated_facts(body, false, None).err(),
            Some(MissingCombatFacts::UnknownFireType)
        );
    }

    #[test]
    fn a_gun_takes_its_melee_from_its_knife() {
        let gun = WeaponBodyFacts {
            melee_damage: 1,
            melee_time_ms: 2,
            melee_delay_ms: 3,
            melee_charge_time_ms: 4,
            melee_charge_delay_ms: 5,
            knife_model: 0,
            damage: 40,
            ..WeaponBodyFacts::default()
        };
        let knife = WeaponBodyFacts {
            melee_damage: 100,
            melee_time_ms: 200,
            melee_delay_ms: 300,
            melee_charge_time_ms: 400,
            melee_charge_delay_ms: 500,
            ..WeaponBodyFacts::default()
        };
        let merged = with_melee_of(gun, &knife, 77);
        assert_eq!(
            (
                merged.melee_damage,
                merged.melee_time_ms,
                merged.melee_delay_ms,
                merged.melee_charge_time_ms,
                merged.melee_charge_delay_ms,
                merged.knife_model
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
        assert!(spins(true, Some(AssetNamespace::T6)));
        assert!(!spins(false, Some(AssetNamespace::T6)));
        assert!(spins(false, Some(AssetNamespace::T5)));
        assert!(spins(false, None));
    }

    #[test]
    fn only_the_ballistic_knife_script_makes_a_ballistic_blade() {
        assert!(is_ballistic_blade(
            Some(AssetNamespace::T6),
            "knife_ballistic_mp"
        ));
        assert!(!is_ballistic_blade(Some(AssetNamespace::T6), "knife_mp"));
        assert!(!is_ballistic_blade(None, "knife_ballistic_mp"));
    }

    #[test]
    fn grenades_and_grenade_class_projectiles_limit_their_impacts() {
        let ns = Some(AssetNamespace::T6);
        assert!(limits_grenade_impacts(ns, 1, 0));
        assert!(limits_grenade_impacts(ns, 2, 6));
        assert!(!limits_grenade_impacts(ns, 2, 5));
        assert!(!limits_grenade_impacts(ns, 0, 6));
        assert!(!limits_grenade_impacts(Some(AssetNamespace::T5), 1, 0));
    }

    #[test]
    fn an_empty_registry_has_just_the_no_weapon_row() {
        let registry = WeaponRegistry::default();
        let combat = from_registry(&registry, None);
        assert_eq!(combat.len(), 1);
        assert!(!combat[0].is_usable());
        assert_eq!(melee_only_from_registry(&registry), [false]);
        assert_eq!(script_sounds_from_registry(&registry).len(), 1);
        assert_eq!(pen_from_registry(&registry).len(), 1);
        assert_eq!(equipment_from_registry(&registry).len(), 1);
    }
}
