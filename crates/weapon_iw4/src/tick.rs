use crate::weaponstate::{FireType, WeaponDecodeError, WeaponState};

pub const PERK_FASTRELOAD: u32 = 4;

/// Sprint recovery (specialty_sprintrecovery).
pub const PERK_SPRINTRECOVERY: u32 = 1 << 10;

pub const PERK_WEAP_RELOAD_MULTIPLIER_DEFAULT: f32 = 0.5;

pub fn perk_fastreload_eligible(perks0: u32, inherits_perks: bool) -> bool {
    (perks0 & PERK_FASTRELOAD) != 0 && inherits_perks
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingCombatFacts {
    BurstCooldown,

    SegmentedReloadAmmoAdd,

    BulletRange,

    UnknownFireType,

    LocationDamage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapturedCombatInput {
    pub fire_time_ms: i32,
    /// bo2zm: Black Ops II's faster first shots of a trigger pull
    /// (`iIntroFireTime`, `iIntroFireLength`): the HAMR, AN-94 and B23R.
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// bo2zm: Black Ops II's charge shot (fire type 8).
    pub charge_shot: bool,
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
    pub ammo_index: i32,
    pub clip_index: i32,
    pub fire_type: i32,

    pub weap_type: i32,

    pub weap_class: i32,

    pub player_anim_type: i32,

    pub inventory_type: i32,

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

    pub unlimited_ammo: bool,
    pub sprint_raise_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    pub damage: i32,
    pub min_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
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

    pub inherits_perks: bool,

    pub ads_in_rate: f32,

    pub ads_out_rate: f32,

    pub rechamber_while_ads: bool,

    /// BO2: a reload lowers the sights (!bReloadWhileAds).
    pub reload_lowers_ads: bool,

    /// BO2 iPositionReloadTransTime: the sights come back up when this
    /// much of the reload is left.
    pub position_reload_trans_ms: i32,

    pub ads_fire_only: bool,

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

    pub location_damage: [f32; crate::HITLOC_COUNT],

    pub dual_mag: Option<crate::reload::DualMagTimes>,

    /// bo2zm fix list 2: aim each shot with the spread before its own
    /// fire add (Black Ops II); false = Modern Warfare 2's order.
    pub spread_before_fire_add: bool,
    /// bo2zm: pellets in a flat line across the aim (Black Ops II Zombies'
    /// accuracy-perk weapons, the Blundergat).
    pub flat_spread: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponCombatFacts {
    pub aim_assist: AimAssistRanges,
    /// bo2zm: Black Ops II footstep rhythm scales while sprinting, sprinting
    /// crouched and diving (1.0: unchanged).
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,
    pub fire_time_ms: i32,
    /// bo2zm: the first `intro_fire_length` shots of a trigger pull come
    /// `intro_fire_time_ms` apart (Black Ops II's HAMR, AN-94, B23R).
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// bo2zm: Black Ops II's charge shot (fire type 8).
    pub charge_shot: bool,
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

    pub ammo_index: i32,

    pub clip_index: i32,
    pub fire_type: i32,

    pub weap_type: i32,

    pub weap_class: i32,

    pub player_anim_type: i32,

    pub inventory_type: i32,

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

    pub unlimited_ammo: bool,

    pub inherits_perks: bool,

    pub sprint_raise_time_ms: i32,

    pub sprint_drop_time_ms: i32,
    pub damage: i32,
    pub min_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
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

    pub location_damage: [f32; crate::HITLOC_COUNT],

    pub dual_mag: Option<crate::reload::DualMagTimes>,

    /// bo2zm fix list 2: see `CapturedCombatInput::spread_before_fire_add`.
    pub spread_before_fire_add: bool,
    /// bo2zm: see `CapturedCombatInput::flat_spread`.
    pub flat_spread: bool,
    /// bo2zm: a Black Ops II dual-wield weapon's left-hand weapon (0: not
    /// dual wield): given, it holds two clips and fires from both hands.
    pub dual_wield_weapon: u32,
    /// bo2zm: the grenade a Black Ops II gun's shot becomes for the
    /// scripts (`szSpawnedGrenadeWeaponName`: the Sliquifier's slip bolt);
    /// 0: none.
    pub spawned_grenade: u32,
    /// bo2zm: picked back up once placed (Black Ops II `bRetrievable`).
    pub retrievable: bool,
}

impl Default for WeaponCombatFacts {
    fn default() -> Self {
        Self::none()
    }
}

impl WeaponCombatFacts {
    pub const fn none() -> Self {
        Self {
            aim_assist: AimAssistRanges::NONE,
            fire_time_ms: 0,
            intro_fire_time_ms: 0,
            intro_fire_length: 0,
            charge_shot: false,
            fire_delay_ms: 0,
            raise_time_ms: 0,
            drop_time_ms: 0,
            alternate_weapon: 0,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
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
            unlimited_ammo: false,
            sprint_raise_time_ms: 0,
            sprint_drop_time_ms: 0,
            damage: 0,
            min_damage: 0,
            max_damage_range: 0.0,
            min_damage_range: 0.0,
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
            inherits_perks: false,
            ads_in_rate: 0.0,
            ads_out_rate: 0.0,
            rechamber_while_ads: true,
            reload_lowers_ads: false,
            position_reload_trans_ms: 0,
            ads_fire_only: false,
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
            location_damage: crate::LOCATION_DAMAGE_IDENTITY,
            dual_mag: None,
            spread_before_fire_add: false,
            flat_spread: false,
            dual_wield_weapon: 0,
            spawned_grenade: 0,
            retrievable: false,
        }
    }

    pub fn test_fixture(_reason: &'static str, facts: Self) -> Self {
        facts
    }

    pub fn try_from_captured(input: CapturedCombatInput) -> Result<Self, MissingCombatFacts> {
        if input.fire_time_ms <= 0
            && input.raise_time_ms <= 0
            && input.damage <= 0
            && input.clip_size <= 0
        {
            return Ok(Self::none());
        }
        let fire_ty =
            FireType::from_i32(input.fire_type).map_err(|_| MissingCombatFacts::UnknownFireType)?;
        if fire_ty.is_burst() && input.burst_cooldown_ms <= 0 {
            return Err(MissingCombatFacts::BurstCooldown);
        }
        if input.segmented_reload && input.reload_ammo_add <= 0 {
            return Err(MissingCombatFacts::SegmentedReloadAmmoAdd);
        }
        if input.damage > 0 && input.max_damage_range <= 0.0 && input.min_damage_range <= 0.0 {
            return Err(MissingCombatFacts::BulletRange);
        }
        if !crate::location_damage_is_valid(&input.location_damage) {
            return Err(MissingCombatFacts::LocationDamage);
        }
        Ok(Self {
            aim_assist: AimAssistRanges::NONE,
            fire_time_ms: input.fire_time_ms,
            intro_fire_time_ms: input.intro_fire_time_ms,
            intro_fire_length: input.intro_fire_length,
            charge_shot: input.charge_shot,
            fire_delay_ms: input.fire_delay_ms,
            raise_time_ms: input.raise_time_ms,
            drop_time_ms: input.drop_time_ms,
            alternate_weapon: input.alternate_weapon,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
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
            unlimited_ammo: input.unlimited_ammo,
            inherits_perks: input.inherits_perks,
            sprint_raise_time_ms: input.sprint_raise_time_ms,
            sprint_drop_time_ms: input.sprint_drop_time_ms,
            damage: input.damage,
            min_damage: input.min_damage,
            max_damage_range: input.max_damage_range,
            min_damage_range: input.min_damage_range,
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
            flat_spread: input.flat_spread,
            dual_wield_weapon: 0,
            spawned_grenade: 0,
            retrievable: false,
        })
    }

    pub fn is_none(self) -> bool {
        self.fire_time_ms <= 0 && self.raise_time_ms <= 0 && self.damage <= 0 && self.clip_size <= 0
    }

    pub fn is_usable(self) -> bool {
        !self.is_none()
    }

    pub fn fire_type_enum(self) -> Result<FireType, WeaponDecodeError> {
        FireType::from_i32(self.fire_type)
    }

    pub fn ammo_per_shot(self) -> i32 {
        1
    }

    pub fn pellet_count(self) -> i32 {
        if self.weap_class == crate::WEAPCLASS_SPREAD {
            if self.shots_per_fire <= 0 {
                1
            } else {
                self.shots_per_fire
            }
        } else {
            1
        }
    }

    pub fn reload_duration_ms(self, clip_empty: bool) -> i32 {
        if clip_empty && self.reload_empty_time_ms > 0 {
            self.reload_empty_time_ms
        } else {
            self.reload_time_ms
        }
    }

    pub fn burst_cooldown(self) -> i32 {
        self.burst_cooldown_ms
    }

    pub fn shells_per_reload_add(self) -> i32 {
        self.reload_ammo_add
    }

    pub fn bullet_range(self) -> f32 {
        if self.weap_class == crate::WEAPCLASS_SPREAD {
            self.min_damage_range
        } else {
            crate::BULLET_MAX_RANGE
        }
    }

    pub fn location_scale(self, hitloc: u8) -> f32 {
        crate::location_damage_scale(&self.location_damage, hitloc)
    }

    pub fn spread_facts(self) -> crate::WeaponSpreadFacts {
        crate::WeaponSpreadFacts {
            stand_min: self.hip_spread_stand_min,
            ducked_min: self.hip_spread_ducked_min,
            prone_min: self.hip_spread_prone_min,
            stand_max: self.hip_spread_stand_max,
            ducked_max: self.hip_spread_ducked_max,
            prone_max: self.hip_spread_prone_max,
        }
    }

    pub fn aim_spread_decay_facts(self) -> crate::WeaponAimSpreadDecayFacts {
        crate::WeaponAimSpreadDecayFacts {
            decay_rate: self.hip_spread_decay_rate,
            fire_add: self.hip_spread_fire_add,
            turn_add: self.hip_spread_turn_add,
            move_add: self.hip_spread_move_add,
            ducked_decay: self.hip_spread_ducked_decay,
            prone_decay: self.hip_spread_prone_decay,
        }
    }

    pub fn ads_allow_facts(self) -> crate::AdsAllowWeaponFacts {
        crate::AdsAllowWeaponFacts {
            aim_down_sight: self.aim_down_sight,
            no_ads_when_mag_empty: self.no_ads_when_mag_empty,
            clip_index: self.clip_index,
        }
    }

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

pub fn spawn_clip_stock(facts: &WeaponCombatFacts, last_hand: i32) -> (i32, i32, i32) {
    let total = facts.start_ammo.max(0);
    let mag = facts.clip_size.max(0);
    let clip0 = if mag > 0 { total.min(mag) } else { 0 };
    let rest = (total - clip0).max(0);
    let clip1 = if last_hand >= 1 && mag > 0 {
        rest.min(mag)
    } else {
        0
    };
    let stock = (rest - clip1).max(0);
    (clip0, clip1, stock)
}

pub fn spawn_weapon_hand(weapon: u32, facts: &WeaponCombatFacts) -> WeaponHandState {
    let total = facts.start_ammo.max(0);
    let clip = if facts.clip_size > 0 {
        total.min(facts.clip_size)
    } else {
        0
    };
    let stock = (total - clip).max(0);
    let raise = if facts.raise_time_ms > 0 {
        facts.raise_time_ms
    } else {
        0
    };
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
            WeaponState::Raising as i32
        } else {
            WeaponState::Ready as i32
        },
        weapon_time: raise,
        weapon_delay: 0,
        weap_anim,
        hand_index: 0,
        clip,
        stock,
        shot_count: 0,
        burst_latch: false,
        rechamber_pending: false,
        delayed_rechamber: false,
        weapon_restrict_kick_time: 0,
        quick_reload: facts.dual_mag.is_some(),
    }
}
