//! What a weapon file says about how the gun fights, checked as it is taken in, and the
//! hand state a weapon starts in.

use crate::damage::catalog::WEAPCLASS_SPREAD;
use crate::fire_type::FireType;

/// Fast reload (perks bit).
pub const PERK_FASTRELOAD: u32 = 4;

/// Sprint recovery (specialty_sprintrecovery).
pub const PERK_SPRINTRECOVERY: u32 = 1 << 10;

/// What a perk-trimmed reload multiplies by when the weapon file gives none.
pub const PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT: f32 = 0.5;

/// A weapon speeds its reload up with the fast reload perk only if it inherits perks.
pub fn perk_fastreload_eligible(perks0: u32, inherits_perks: bool) -> bool {
    inherits_perks && perks0 & PERK_FASTRELOAD != 0
}

/// What a weapon file left out or got wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingCombatFacts {
    /// A burst weapon with no time between bursts.
    BurstCooldown,
    /// A segmented reload that adds no rounds a step.
    SegmentedReloadAmmoAdd,
    /// A bullet weapon that reaches nowhere.
    BulletRange,
    UnknownFireType,
    /// A hit location table with a value that cannot scale damage.
    LocationDamage,
}

/// A weapon file's combat fields as read, before they are checked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapturedCombatInput {
    /// Between shots.
    pub fire_time_ms: i32,
    /// bo2zm: Black Ops II's faster first shots of a trigger pull
    /// (`iIntroFireTime`, `iIntroFireLength`): the HAMR, AN-94 and B23R.
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// From the trigger to the shot.
    pub fire_delay_ms: i32,
    pub raise_time_ms: i32,
    pub drop_time_ms: i32,
    pub alternate_weapon: u32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    pub clip_size: i32,
    pub start_ammo: i32,
    pub max_ammo: i32,
    /// Key of the stock row in the ammo table (0: the weapon itself).
    pub ammo_index: i32,
    /// Key of the clip row in the clip table (0: the weapon itself).
    pub clip_index: i32,
    /// A [`FireType`] number.
    pub fire_type: i32,
    pub weap_type: i32,
    pub weap_class: i32,
    pub player_anim_type: i32,
    pub inventory_type: i32,
    /// bo2mp: switches to and from it are quick (BO2 `BG_IsQuickSwitch`).
    pub quick_switch: bool,
    /// bo2mp: a switch to it is quick (pistol, pistol spread, melee).
    pub quick_switch_target: bool,
    pub impact_type: i32,
    pub shots_per_fire: i32,
    pub burst_cooldown_ms: i32,
    pub bolt_action: bool,
    pub rechamber_time_ms: i32,
    pub rechamber_bolt_time_ms: i32,
    pub rechamber_bolt_delay_ms: i32,
    pub segmented_reload: bool,
    pub reload_start_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_ammo_add: i32,
    pub reload_add_time_ms: i32,
    pub reload_empty_add_time_ms: i32,
    pub reload_start_add_time_ms: i32,
    pub reload_start_add: i32,
    pub no_partial_reload: bool,
    pub inherits_perks: bool,
    pub sprint_raise_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    pub damage: i32,
    pub min_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
    pub damage_steps: Option<crate::damage::DamageSteps>,
    pub hip_spread_stand_min: f32,
    pub hip_spread_ducked_min: f32,
    pub hip_spread_prone_min: f32,
    pub hip_spread_stand_max: f32,
    pub hip_spread_ducked_max: f32,
    pub hip_spread_prone_max: f32,
    pub hip_spread_decay_rate: f32,
    pub hip_spread_fire_add: f32,
    pub hip_spread_turn_add: f32,
    pub hip_spread_move_add: f32,
    pub hip_spread_ducked_decay: f32,
    pub hip_spread_prone_decay: f32,
    pub ads_spread: f32,
    pub aim_down_sight: bool,
    pub no_ads_when_mag_empty: bool,
    pub ads_in_rate: f32,
    pub ads_out_rate: f32,
    pub rechamber_while_ads: bool,
    /// BO2: a reload lowers the sights (!bReloadWhileAds).
    pub reload_lowers_ads: bool,
    /// BO2 iPositionReloadTransTime: the sights come back up when this
    /// much of the reload is left.
    pub position_reload_trans_ms: i32,
    pub ads_fire_only: bool,
    /// bo2mp: Hybrid Optic gun: its alternate weapon is the other sight.
    pub optic_switch: bool,
    pub melee_damage: i32,
    pub overlay_reticle: i32,
    pub melee_time_ms: i32,
    pub melee_delay_ms: i32,
    pub melee_charge_time_ms: i32,
    pub melee_charge_delay_ms: i32,
    pub melee_charge_anim: bool,
    pub knife_model: u32,
    pub quick_raise_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub ads_gun_kick_reduced_kick_bullets: i32,
    pub hip_gun_kick_reduced_kick_bullets: i32,
    pub location_damage: [f32; crate::hitloc::HITLOC_COUNT],
    pub dual_mag: Option<crate::dual_mag::DualMagTimes>,
    /// bo2zm fix list 2: aim each shot with the spread before its own
    /// fire add (Black Ops II); false = Modern Warfare 2's order.
    pub spread_before_fire_add: bool,
}

/// How far each kind of aim assist reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimAssistRanges {
    pub auto_aim: f32,
    pub hip: f32,
    pub ads: f32,
}

impl AimAssistRanges {
    pub const NONE: Self = Self {
        auto_aim: 0.0,
        hip: 0.0,
        ads: 0.0,
    };
}

/// A weapon's combat fields, checked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponCombatFacts {
    pub aim_assist: AimAssistRanges,
    /// bo2zm: Black Ops II footstep rhythm scales while sprinting, sprinting
    /// crouched and diving (1.0: unchanged).
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,
    /// Between shots.
    pub fire_time_ms: i32,
    /// bo2zm: the first `intro_fire_length` shots of a trigger pull come
    /// `intro_fire_time_ms` apart (Black Ops II's HAMR, AN-94, B23R).
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// From the trigger to the shot.
    pub fire_delay_ms: i32,
    pub raise_time_ms: i32,
    pub drop_time_ms: i32,
    pub alternate_weapon: u32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    pub clip_size: i32,
    pub start_ammo: i32,
    pub max_ammo: i32,
    /// Key of the stock row in the ammo table (0: the weapon itself).
    pub ammo_index: i32,
    /// Key of the clip row in the clip table (0: the weapon itself).
    pub clip_index: i32,
    /// A [`FireType`] number.
    pub fire_type: i32,
    pub weap_type: i32,
    pub weap_class: i32,
    pub player_anim_type: i32,
    pub inventory_type: i32,
    /// bo2mp: switches to and from it are quick (BO2 `BG_IsQuickSwitch`).
    pub quick_switch: bool,
    /// bo2mp: a switch to it is quick (pistol, pistol spread, melee).
    pub quick_switch_target: bool,
    pub impact_type: i32,
    pub shots_per_fire: i32,
    pub burst_cooldown_ms: i32,
    pub bolt_action: bool,
    pub rechamber_time_ms: i32,
    pub rechamber_bolt_time_ms: i32,
    pub rechamber_bolt_delay_ms: i32,
    pub segmented_reload: bool,
    pub reload_start_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_ammo_add: i32,
    pub reload_add_time_ms: i32,
    pub reload_empty_add_time_ms: i32,
    pub reload_start_add_time_ms: i32,
    pub reload_start_add: i32,
    pub no_partial_reload: bool,
    pub inherits_perks: bool,
    pub sprint_raise_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    pub damage: i32,
    pub min_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
    pub damage_steps: Option<crate::damage::DamageSteps>,
    pub hip_spread_stand_min: f32,
    pub hip_spread_ducked_min: f32,
    pub hip_spread_prone_min: f32,
    pub hip_spread_stand_max: f32,
    pub hip_spread_ducked_max: f32,
    pub hip_spread_prone_max: f32,
    pub hip_spread_decay_rate: f32,
    pub hip_spread_fire_add: f32,
    pub hip_spread_turn_add: f32,
    pub hip_spread_move_add: f32,
    pub hip_spread_ducked_decay: f32,
    pub hip_spread_prone_decay: f32,
    pub ads_spread: f32,
    pub aim_down_sight: bool,
    pub no_ads_when_mag_empty: bool,
    pub ads_in_rate: f32,
    pub ads_out_rate: f32,
    pub rechamber_while_ads: bool,
    /// BO2: a reload lowers the sights (!bReloadWhileAds).
    pub reload_lowers_ads: bool,
    /// BO2 iPositionReloadTransTime: the sights come back up when this
    /// much of the reload is left.
    pub position_reload_trans_ms: i32,
    pub ads_fire_only: bool,
    pub optic_switch: bool,
    pub melee_damage: i32,
    pub overlay_reticle: i32,
    pub melee_time_ms: i32,
    pub melee_delay_ms: i32,
    pub melee_charge_time_ms: i32,
    pub melee_charge_delay_ms: i32,
    pub melee_charge_anim: bool,
    pub knife_model: u32,
    pub quick_raise_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub ads_gun_kick_reduced_kick_bullets: i32,
    pub hip_gun_kick_reduced_kick_bullets: i32,
    pub location_damage: [f32; crate::hitloc::HITLOC_COUNT],
    pub dual_mag: Option<crate::dual_mag::DualMagTimes>,
    /// bo2zm fix list 2: see `CapturedCombatInput::spread_before_fire_add`.
    pub spread_before_fire_add: bool,
    /// bo2zm: a Black Ops II dual-wield weapon's left-hand weapon (0: not
    /// dual wield): given, it holds two clips and fires from both hands.
    pub dual_wield_weapon: u32,
}

impl Default for WeaponCombatFacts {
    fn default() -> Self {
        Self::none()
    }
}

impl WeaponCombatFacts {
    /// A weapon that does nothing.
    pub const fn none() -> Self {
        Self {
            aim_assist: AimAssistRanges::NONE,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
            fire_time_ms: 0,
            intro_fire_time_ms: 0,
            intro_fire_length: 0,
            fire_delay_ms: 0,
            raise_time_ms: 0,
            drop_time_ms: 0,
            alternate_weapon: 0,
            alternate_raise_time_ms: 0,
            alternate_drop_time_ms: 0,
            reload_time_ms: 0,
            reload_empty_time_ms: 0,
            clip_size: 0,
            start_ammo: 0,
            max_ammo: 0,
            ammo_index: 0,
            clip_index: 0,
            fire_type: 0,
            weap_type: 0,
            weap_class: 0,
            player_anim_type: 0,
            inventory_type: 0,
            quick_switch: false,
            quick_switch_target: false,
            impact_type: 0,
            shots_per_fire: 0,
            burst_cooldown_ms: 0,
            bolt_action: false,
            rechamber_time_ms: 0,
            rechamber_bolt_time_ms: 0,
            rechamber_bolt_delay_ms: 0,
            segmented_reload: false,
            reload_start_time_ms: 0,
            reload_end_time_ms: 0,
            reload_ammo_add: 0,
            reload_add_time_ms: 0,
            reload_empty_add_time_ms: 0,
            reload_start_add_time_ms: 0,
            reload_start_add: 0,
            no_partial_reload: false,
            inherits_perks: false,
            sprint_raise_time_ms: 0,
            sprint_drop_time_ms: 0,
            damage: 0,
            min_damage: 0,
            max_damage_range: 0.0,
            min_damage_range: 0.0,
            damage_steps: None,
            hip_spread_stand_min: 0.0,
            hip_spread_ducked_min: 0.0,
            hip_spread_prone_min: 0.0,
            hip_spread_stand_max: 0.0,
            hip_spread_ducked_max: 0.0,
            hip_spread_prone_max: 0.0,
            hip_spread_decay_rate: 0.0,
            hip_spread_fire_add: 0.0,
            hip_spread_turn_add: 0.0,
            hip_spread_move_add: 0.0,
            hip_spread_ducked_decay: 0.0,
            hip_spread_prone_decay: 0.0,
            ads_spread: 0.0,
            aim_down_sight: false,
            no_ads_when_mag_empty: false,
            ads_in_rate: 0.0,
            ads_out_rate: 0.0,
            rechamber_while_ads: true,
            reload_lowers_ads: false,
            position_reload_trans_ms: 0,
            ads_fire_only: false,
            optic_switch: false,
            melee_damage: 0,
            overlay_reticle: 0,
            melee_time_ms: 0,
            melee_delay_ms: 0,
            melee_charge_time_ms: 0,
            melee_charge_delay_ms: 0,
            melee_charge_anim: false,
            knife_model: 0,
            quick_raise_time_ms: 0,
            quick_drop_time_ms: 0,
            select_requires_ammo: Some(false),
            offhand_hold_is_cancelable: Some(false),
            ads_gun_kick_reduced_kick_bullets: 0,
            hip_gun_kick_reduced_kick_bullets: 0,
            location_damage: crate::damage::LOCATION_DAMAGE_IDENTITY,
            dual_mag: None,
            spread_before_fire_add: false,
            dual_wield_weapon: 0,
        }
    }

    /// The captured fields checked and taken in. A weapon with no fire time, raise time,
    /// damage or clip is [`Self::none`].
    pub fn try_from_captured(input: CapturedCombatInput) -> Result<Self, MissingCombatFacts> {
        let nothing = input.fire_time_ms <= 0
            && input.raise_time_ms <= 0
            && input.damage <= 0
            && input.clip_size <= 0;
        if nothing {
            return Ok(Self::none());
        }
        let fire_type =
            FireType::from_i32(input.fire_type).map_err(|_| MissingCombatFacts::UnknownFireType)?;
        if fire_type.is_burst() && input.burst_cooldown_ms <= 0 {
            return Err(MissingCombatFacts::BurstCooldown);
        }
        if input.segmented_reload && input.reload_ammo_add <= 0 {
            return Err(MissingCombatFacts::SegmentedReloadAmmoAdd);
        }
        if input.damage > 0 && input.max_damage_range <= 0.0 && input.min_damage_range <= 0.0 {
            return Err(MissingCombatFacts::BulletRange);
        }
        if !crate::damage::location_damage_is_valid(&input.location_damage) {
            return Err(MissingCombatFacts::LocationDamage);
        }
        Ok(Self {
            aim_assist: AimAssistRanges::NONE,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
            fire_time_ms: input.fire_time_ms,
            intro_fire_time_ms: input.intro_fire_time_ms,
            intro_fire_length: input.intro_fire_length,
            fire_delay_ms: input.fire_delay_ms,
            raise_time_ms: input.raise_time_ms,
            drop_time_ms: input.drop_time_ms,
            alternate_weapon: input.alternate_weapon,
            alternate_raise_time_ms: input.alternate_raise_time_ms,
            alternate_drop_time_ms: input.alternate_drop_time_ms,
            reload_time_ms: input.reload_time_ms,
            reload_empty_time_ms: input.reload_empty_time_ms,
            clip_size: input.clip_size,
            start_ammo: input.start_ammo,
            max_ammo: input.max_ammo,
            ammo_index: input.ammo_index,
            clip_index: input.clip_index,
            fire_type: input.fire_type,
            weap_type: input.weap_type,
            weap_class: input.weap_class,
            player_anim_type: input.player_anim_type,
            inventory_type: input.inventory_type,
            quick_switch: input.quick_switch,
            quick_switch_target: input.quick_switch_target,
            impact_type: input.impact_type,
            shots_per_fire: input.shots_per_fire,
            burst_cooldown_ms: input.burst_cooldown_ms,
            bolt_action: input.bolt_action,
            rechamber_time_ms: input.rechamber_time_ms,
            rechamber_bolt_time_ms: input.rechamber_bolt_time_ms,
            rechamber_bolt_delay_ms: input.rechamber_bolt_delay_ms,
            segmented_reload: input.segmented_reload,
            reload_start_time_ms: input.reload_start_time_ms,
            reload_end_time_ms: input.reload_end_time_ms,
            reload_ammo_add: input.reload_ammo_add,
            reload_add_time_ms: input.reload_add_time_ms,
            reload_empty_add_time_ms: input.reload_empty_add_time_ms,
            reload_start_add_time_ms: input.reload_start_add_time_ms,
            reload_start_add: input.reload_start_add,
            no_partial_reload: input.no_partial_reload,
            inherits_perks: input.inherits_perks,
            sprint_raise_time_ms: input.sprint_raise_time_ms,
            sprint_drop_time_ms: input.sprint_drop_time_ms,
            damage: input.damage,
            min_damage: input.min_damage,
            max_damage_range: input.max_damage_range,
            min_damage_range: input.min_damage_range,
            damage_steps: input.damage_steps,
            hip_spread_stand_min: input.hip_spread_stand_min,
            hip_spread_ducked_min: input.hip_spread_ducked_min,
            hip_spread_prone_min: input.hip_spread_prone_min,
            hip_spread_stand_max: input.hip_spread_stand_max,
            hip_spread_ducked_max: input.hip_spread_ducked_max,
            hip_spread_prone_max: input.hip_spread_prone_max,
            hip_spread_decay_rate: input.hip_spread_decay_rate,
            hip_spread_fire_add: input.hip_spread_fire_add,
            hip_spread_turn_add: input.hip_spread_turn_add,
            hip_spread_move_add: input.hip_spread_move_add,
            hip_spread_ducked_decay: input.hip_spread_ducked_decay,
            hip_spread_prone_decay: input.hip_spread_prone_decay,
            ads_spread: input.ads_spread,
            aim_down_sight: input.aim_down_sight,
            no_ads_when_mag_empty: input.no_ads_when_mag_empty,
            ads_in_rate: input.ads_in_rate,
            ads_out_rate: input.ads_out_rate,
            rechamber_while_ads: input.rechamber_while_ads,
            reload_lowers_ads: input.reload_lowers_ads,
            position_reload_trans_ms: input.position_reload_trans_ms,
            ads_fire_only: input.ads_fire_only,
            optic_switch: input.optic_switch,
            melee_damage: input.melee_damage,
            overlay_reticle: input.overlay_reticle,
            melee_time_ms: input.melee_time_ms,
            melee_delay_ms: input.melee_delay_ms,
            melee_charge_time_ms: input.melee_charge_time_ms,
            melee_charge_delay_ms: input.melee_charge_delay_ms,
            melee_charge_anim: input.melee_charge_anim,
            knife_model: input.knife_model,
            quick_raise_time_ms: input.quick_raise_time_ms,
            quick_drop_time_ms: input.quick_drop_time_ms,
            select_requires_ammo: input.select_requires_ammo,
            offhand_hold_is_cancelable: input.offhand_hold_is_cancelable,
            ads_gun_kick_reduced_kick_bullets: input.ads_gun_kick_reduced_kick_bullets,
            hip_gun_kick_reduced_kick_bullets: input.hip_gun_kick_reduced_kick_bullets,
            location_damage: input.location_damage,
            dual_mag: input.dual_mag,
            spread_before_fire_add: input.spread_before_fire_add,
            dual_wield_weapon: 0,
        })
    }

    pub fn is_none(self) -> bool {
        self.fire_time_ms <= 0 && self.raise_time_ms <= 0 && self.damage <= 0 && self.clip_size <= 0
    }

    pub fn is_usable(self) -> bool {
        !self.is_none()
    }

    /// Bullets a shot sends: a spread weapon's pellets, else 1.
    pub fn pellet_count(self) -> i32 {
        if self.weap_class == WEAPCLASS_SPREAD {
            self.shots_per_fire.max(1)
        } else {
            1
        }
    }

    /// How far a bullet reaches: a spread weapon's own range, else the world's.
    pub fn bullet_range(self) -> f32 {
        if self.weap_class == WEAPCLASS_SPREAD {
            self.min_damage_range
        } else {
            crate::damage::BULLET_MAX_RANGE
        }
    }

    /// Damage scale for a hit on `hitloc`.
    pub fn location_scale(self, hitloc: u8) -> f32 {
        crate::hitloc::location_scale(&self.location_damage, hitloc)
    }

    pub fn ads_allow_facts(self) -> crate::ads_allow::AdsAllowWeaponFacts {
        crate::ads_allow::AdsAllowWeaponFacts {
            aim_down_sight: self.aim_down_sight,
            no_ads_when_mag_empty: self.no_ads_when_mag_empty,
            clip_index: self.clip_index,
            optic_switch: self.optic_switch,
        }
    }

    /// What moves the sights: (rate in, rate out, rechamber while aimed, aimed fire only).
    pub fn ads_frac_context(self) -> (f32, f32, bool, bool) {
        (
            self.ads_in_rate,
            self.ads_out_rate,
            self.rechamber_while_ads,
            self.ads_fire_only,
        )
    }

    /// BO2's reload rule for the sights: (a reload lowers them, how much of
    /// the reload is left when they come back up, segmented reload).
    pub fn ads_reload_rule(self) -> (bool, i32, bool) {
        (
            self.reload_lowers_ads,
            self.position_reload_trans_ms,
            self.segmented_reload,
        )
    }
}

/// A hand's weapon, its state and its rounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponHandState {
    pub weapon: u32,
    pub weaponstate: i32,
    pub weapon_time: i32,
    pub weapon_delay: i32,
    pub weap_anim: i32,
    pub hand_index: u8,
    pub clip: i32,
    pub stock: i32,
    pub shot_count: u8,
    pub burst_latch: bool,
    pub rechamber_pending: bool,
    pub delayed_rechamber: bool,
    pub weapon_restrict_kick_time: i32,
    pub quick_reload: bool,
}

/// The rounds a weapon spawns with, as (right clip, left clip, stock). The left clip is
/// only filled once the left hand has been used.
pub fn spawn_clip_stock(facts: &WeaponCombatFacts, last_hand: i32) -> (i32, i32, i32) {
    let total = facts.start_ammo.max(0);
    let mag = facts.clip_size.max(0);
    let right = if mag > 0 { total.min(mag) } else { 0 };
    let rest = total - right;
    let left = if last_hand >= 1 && mag > 0 {
        rest.min(mag)
    } else {
        0
    };
    (right, left, rest - left)
}

/// A hand just given `weapon`: raising it if the file has a raise time, else ready.
pub fn spawn_weapon_hand(weapon: u32, facts: &WeaponCombatFacts) -> WeaponHandState {
    let total = facts.start_ammo.max(0);
    let clip = if facts.clip_size > 0 {
        total.min(facts.clip_size)
    } else {
        0
    };
    let raise = facts.raise_time_ms.max(0);
    let mut weap_anim = 0;
    if raise > 0 {
        crate::weap_anim::start_weapon_anim(
            &mut weap_anim,
            crate::weap_anim::weap_anim_event::RAISE,
        );
    }
    WeaponHandState {
        weapon,
        weaponstate: if raise > 0 {
            crate::state::RAISING
        } else {
            crate::state::READY
        },
        weapon_time: raise,
        weap_anim,
        clip,
        stock: total - clip,
        quick_reload: facts.dual_mag.is_some(),
        ..WeaponHandState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> CapturedCombatInput {
        CapturedCombatInput {
            fire_time_ms: 90,
            raise_time_ms: 400,
            damage: 30,
            clip_size: 30,
            start_ammo: 90,
            max_damage_range: 1000.0,
            min_damage_range: 2000.0,
            fire_type: 0,
            location_damage: crate::damage::LOCATION_DAMAGE_IDENTITY,
            ..CapturedCombatInput::default()
        }
    }

    #[test]
    fn an_empty_weapon_file_is_nothing() {
        let facts = WeaponCombatFacts::try_from_captured(CapturedCombatInput::default()).unwrap();
        assert!(facts.is_none() && !facts.is_usable());
        assert_eq!(facts, WeaponCombatFacts::none());
    }

    #[test]
    fn a_rifle_is_taken_in_and_a_bad_file_names_what_is_missing() {
        let facts = WeaponCombatFacts::try_from_captured(rifle()).unwrap();
        assert!(facts.is_usable());
        assert_eq!((facts.fire_time_ms, facts.clip_size), (90, 30));
        assert_eq!(facts.pellet_count(), 1);
        assert_eq!(facts.bullet_range(), crate::damage::BULLET_MAX_RANGE);

        let burst = CapturedCombatInput {
            fire_type: 2,
            ..rifle()
        };
        assert_eq!(
            WeaponCombatFacts::try_from_captured(burst),
            Err(MissingCombatFacts::BurstCooldown)
        );
        let odd = CapturedCombatInput {
            fire_type: 9,
            ..rifle()
        };
        assert_eq!(
            WeaponCombatFacts::try_from_captured(odd),
            Err(MissingCombatFacts::UnknownFireType)
        );
        let shells = CapturedCombatInput {
            segmented_reload: true,
            ..rifle()
        };
        assert_eq!(
            WeaponCombatFacts::try_from_captured(shells),
            Err(MissingCombatFacts::SegmentedReloadAmmoAdd)
        );
        let short = CapturedCombatInput {
            max_damage_range: 0.0,
            min_damage_range: 0.0,
            ..rifle()
        };
        assert_eq!(
            WeaponCombatFacts::try_from_captured(short),
            Err(MissingCombatFacts::BulletRange)
        );
        let mut bad = rifle();
        bad.location_damage[0] = f32::NAN;
        assert_eq!(
            WeaponCombatFacts::try_from_captured(bad),
            Err(MissingCombatFacts::LocationDamage)
        );
    }

    #[test]
    fn a_shotgun_sends_its_pellets_and_reaches_its_own_range() {
        let spread = CapturedCombatInput {
            weap_class: WEAPCLASS_SPREAD,
            shots_per_fire: 8,
            ..rifle()
        };
        let facts = WeaponCombatFacts::try_from_captured(spread).unwrap();
        assert_eq!(facts.pellet_count(), 8);
        assert_eq!(facts.bullet_range(), 2000.0);
        let none = WeaponCombatFacts {
            weap_class: WEAPCLASS_SPREAD,
            ..facts
        };
        assert_eq!(
            WeaponCombatFacts {
                shots_per_fire: 0,
                ..none
            }
            .pellet_count(),
            1
        );
    }

    #[test]
    fn spawn_ammo_fills_the_clip_then_the_left_clip_then_stock() {
        let facts = WeaponCombatFacts {
            start_ammo: 100,
            clip_size: 30,
            ..WeaponCombatFacts::none()
        };
        assert_eq!(spawn_clip_stock(&facts, 0), (30, 0, 70));
        assert_eq!(spawn_clip_stock(&facts, 1), (30, 30, 40));
        let small = WeaponCombatFacts {
            start_ammo: 40,
            ..facts
        };
        assert_eq!(spawn_clip_stock(&small, 1), (30, 10, 0));
        assert_eq!(spawn_clip_stock(&WeaponCombatFacts::none(), 1), (0, 0, 0));
    }

    #[test]
    fn a_spawned_hand_raises_the_weapon_if_it_has_a_raise_time() {
        let facts = WeaponCombatFacts::try_from_captured(rifle()).unwrap();
        let hand = spawn_weapon_hand(7, &facts);
        assert_eq!(
            (hand.weapon, hand.weaponstate, hand.weapon_time),
            (7, crate::state::RAISING, 400)
        );
        assert_eq!((hand.clip, hand.stock), (30, 60));
        assert_ne!(hand.weap_anim, 0);

        let ready = spawn_weapon_hand(
            7,
            &WeaponCombatFacts {
                raise_time_ms: 0,
                ..facts
            },
        );
        assert_eq!(
            (ready.weaponstate, ready.weapon_time, ready.weap_anim),
            (crate::state::READY, 0, 0)
        );
    }

    #[test]
    fn fast_reload_needs_the_perk_and_a_weapon_that_inherits_perks() {
        assert!(perk_fastreload_eligible(PERK_FASTRELOAD, true));
        assert!(!perk_fastreload_eligible(PERK_FASTRELOAD, false));
        assert!(!perk_fastreload_eligible(0, true));
    }
}
