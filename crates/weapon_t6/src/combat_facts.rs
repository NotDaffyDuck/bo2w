//! What a weapon file says about how the gun fights, checked as it is taken in, and the
//! hand state a weapon starts in.

use crate::damage::catalog::WEAPCLASS_SPREAD;
use crate::fire_type::TriggerPattern;

/// What a perk-trimmed reload multiplies by when the weapon file gives none.
pub const QUICK_HANDS_FACTOR_STOCK: f32 = 0.5;

/// A weapon speeds its reload up with the fast reload perk only if it inherits perks. `perks` is the player's two
/// perk words.
pub fn perk_fastreload_eligible(perks: [u32; 2], carries_over_talents: bool) -> bool {
    carries_over_talents && crate::env::perk::held(perks, crate::env::perk::FASTRELOAD)
}

/// What a weapon file left out or got wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbsentBattleProfile {
    /// A burst weapon with no time between bursts.
    VolleyRest,
    /// A segmented reload that adds no rounds a step.
    PerRoundReloadGain,
    /// A bullet weapon that reaches nowhere.
    ShotReach,
    UnrecognisedTriggerPattern,
    /// A hit location table with a value that cannot scale damage.
    LocationDamage,
}

/// A weapon file's combat fields as read, before they are checked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecordedBattleFeed {
    /// Between shots.
    pub shot_gap_ms: i32,
    /// bo2zm: Black Ops II's faster first shots of a trigger pull
    /// (`iIntroFireTime`, `iIntroFireLength`): the HAMR, AN-94 and B23R.
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// bo2zm: Black Ops II's charge shot (fire type 8).
    pub charge_shot: bool,
    /// From the trigger to the shot.
    pub shot_lead_ms: i32,
    pub draw_ms: i32,
    pub holster_ms: i32,
    pub alternate_weapon: u32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub reload_duration_millis: i32,
    pub refill_dry_millis: i32,
    pub magazine_capacity: i32,
    pub spawn_rounds: i32,
    pub reserve_cap: i32,
    /// Key of the stock row in the ammo table (0: the weapon itself).
    pub ammo_type_slot: i32,
    /// Key of the clip row in the clip table (0: the weapon itself).
    pub motion_clip_slot: i32,
    /// A [`TriggerPattern`] number.
    pub trigger_mode: i32,
    pub gun_family_code: i32,
    pub gun_kind: i32,
    pub player_motion_kind: i32,
    pub carry_slot_kind: i32,
    /// bo2mp: switches to and from it are quick (BO2 `BG_IsQuickSwitch`).
    pub quick_switch: bool,
    /// bo2mp: a switch to it is quick (pistol, pistol spread, melee).
    pub quick_switch_target: bool,
    pub impact_kind_id: i32,
    pub rounds_each_trigger: i32,
    pub volley_rest_millis: i32,
    pub manual_cycle: bool,
    pub cycle_ms: i32,
    pub bolt_rack_duration_millis: i32,
    pub bolt_rack_wait_millis: i32,
    pub staged_reload: bool,
    pub refill_begin_millis: i32,
    pub refill_finish_millis: i32,
    pub reload_rounds_gained: i32,
    pub reload_refill_millis: i32,
    pub empty_reload_refill_millis: i32,
    pub reload_begin_refill_millis: i32,
    pub reload_begin_bonus: i32,
    pub forbid_topup_reload: bool,
    /// bo2zm: fires with an empty clip and never uses ammo.
    pub unlimited_ammo: bool,
    pub carries_over_talents: bool,
    pub run_lift_millis: i32,
    pub run_lower_millis: i32,
    pub damage: i32,
    pub hurt_floor: i32,
    pub full_damage_reach: f32,
    pub minimum_damage_reach: f32,
    pub damage_steps: Option<crate::damage::DamageSteps>,
    pub hip_cone_up_lo: f32,
    pub hip_cone_crouch_lo: f32,
    pub hip_cone_lie_lo: f32,
    pub hip_cone_up_hi: f32,
    pub hip_cone_crouch_hi: f32,
    pub hip_cone_lie_hi: f32,
    pub hip_cone_settle: f32,
    pub hip_cone_per_shot: f32,
    pub hip_cone_per_turn: f32,
    pub hip_cone_per_step: f32,
    pub hip_cone_crouch_settle: f32,
    pub hip_cone_lie_settle: f32,
    pub aimed_cone: f32,
    pub scoped_aiming: bool,
    pub block_aim_if_clip_dry: bool,
    pub scope_raise_speed: f32,
    pub scope_lower_speed: f32,
    pub rack_bolt_when_scoped: bool,
    /// BO2: a reload lowers the sights (!bReloadWhileAds).
    pub reload_lowers_ads: bool,
    /// BO2 iPositionReloadTransTime: the sights come back up when this
    /// much of the reload is left.
    pub position_reload_trans_ms: i32,
    pub fire_only_when_scoped: bool,
    /// bo2mp: Hybrid Optic gun: its alternate weapon is the other sight.
    pub optic_switch: bool,
    pub bash_hurt: i32,
    pub hud_crosshair_layer: i32,
    pub bash_ms: i32,
    pub strike_wait_millis: i32,
    pub bash_windup_ms: i32,
    pub lunge_windup_wait_millis: i32,
    pub lunge_has_clip: bool,
    pub blade_mesh: u32,
    pub fast_draw_ms: i32,
    pub fast_holster_ms: i32,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub aim_soft_kick_shots: i32,
    pub hip_soft_kick_shots: i32,
    pub location_damage: [f32; crate::hitloc::HITLOC_COUNT],
    pub dual_mag: Option<crate::dual_mag::DualMagTimes>,
    /// bo2zm fix list 2: aim each shot with the spread before its own
    /// fire add (Black Ops II); false = Modern Warfare 2's order.
    pub spread_before_fire_add: bool,
    /// bo2zm: pellets in a flat line across the aim (Black Ops II Zombies'
    /// accuracy-perk weapons, the Blundergat).
    pub flat_spread: bool,
}

/// How far each kind of aim assist reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AimAssistRanges {
    pub auto_aim: f32,
    pub hip: f32,
    pub ads: f32,
}

impl AimAssistRanges {
    pub const OFF: Self = Self {
        auto_aim: 0.0,
        hip: 0.0,
        ads: 0.0,
    };
}

/// A weapon's combat fields, checked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunBattleProfile {
    pub aim_assist: AimAssistRanges,
    /// bo2zm: Black Ops II footstep rhythm scales while sprinting, sprinting
    /// crouched and diving (1.0: unchanged).
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,
    /// Between shots.
    pub shot_gap_ms: i32,
    /// bo2zm: the first `intro_fire_length` shots of a trigger pull come
    /// `intro_fire_time_ms` apart (Black Ops II's HAMR, AN-94, B23R).
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// bo2zm: Black Ops II's charge shot (fire type 8).
    pub charge_shot: bool,
    /// From the trigger to the shot.
    pub shot_lead_ms: i32,
    pub draw_ms: i32,
    pub holster_ms: i32,
    pub alternate_weapon: u32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub reload_duration_millis: i32,
    pub refill_dry_millis: i32,
    pub magazine_capacity: i32,
    pub spawn_rounds: i32,
    pub reserve_cap: i32,
    /// Key of the stock row in the ammo table (0: the weapon itself).
    pub ammo_type_slot: i32,
    /// Key of the clip row in the clip table (0: the weapon itself).
    pub motion_clip_slot: i32,
    /// A [`TriggerPattern`] number.
    pub trigger_mode: i32,
    pub gun_family_code: i32,
    pub gun_kind: i32,
    pub player_motion_kind: i32,
    pub carry_slot_kind: i32,
    /// bo2mp: switches to and from it are quick (BO2 `BG_IsQuickSwitch`).
    pub quick_switch: bool,
    /// bo2mp: a switch to it is quick (pistol, pistol spread, melee).
    pub quick_switch_target: bool,
    pub impact_kind_id: i32,
    pub rounds_each_trigger: i32,
    pub volley_rest_millis: i32,
    pub manual_cycle: bool,
    pub cycle_ms: i32,
    pub bolt_rack_duration_millis: i32,
    pub bolt_rack_wait_millis: i32,
    pub staged_reload: bool,
    pub refill_begin_millis: i32,
    pub refill_finish_millis: i32,
    pub reload_rounds_gained: i32,
    pub reload_refill_millis: i32,
    pub empty_reload_refill_millis: i32,
    pub reload_begin_refill_millis: i32,
    pub reload_begin_bonus: i32,
    pub forbid_topup_reload: bool,
    /// bo2zm: fires with an empty clip and never uses ammo.
    pub unlimited_ammo: bool,
    pub carries_over_talents: bool,
    pub run_lift_millis: i32,
    pub run_lower_millis: i32,
    pub damage: i32,
    pub hurt_floor: i32,
    pub full_damage_reach: f32,
    pub minimum_damage_reach: f32,
    pub damage_steps: Option<crate::damage::DamageSteps>,
    pub hip_cone_up_lo: f32,
    pub hip_cone_crouch_lo: f32,
    pub hip_cone_lie_lo: f32,
    pub hip_cone_up_hi: f32,
    pub hip_cone_crouch_hi: f32,
    pub hip_cone_lie_hi: f32,
    pub hip_cone_settle: f32,
    pub hip_cone_per_shot: f32,
    pub hip_cone_per_turn: f32,
    pub hip_cone_per_step: f32,
    pub hip_cone_crouch_settle: f32,
    pub hip_cone_lie_settle: f32,
    pub aimed_cone: f32,
    pub scoped_aiming: bool,
    pub block_aim_if_clip_dry: bool,
    pub scope_raise_speed: f32,
    pub scope_lower_speed: f32,
    pub rack_bolt_when_scoped: bool,
    /// BO2: a reload lowers the sights (!bReloadWhileAds).
    pub reload_lowers_ads: bool,
    /// BO2 iPositionReloadTransTime: the sights come back up when this
    /// much of the reload is left.
    pub position_reload_trans_ms: i32,
    pub fire_only_when_scoped: bool,
    pub optic_switch: bool,
    pub bash_hurt: i32,
    pub hud_crosshair_layer: i32,
    pub bash_ms: i32,
    pub strike_wait_millis: i32,
    pub bash_windup_ms: i32,
    pub lunge_windup_wait_millis: i32,
    pub lunge_has_clip: bool,
    pub blade_mesh: u32,
    pub fast_draw_ms: i32,
    pub fast_holster_ms: i32,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub aim_soft_kick_shots: i32,
    pub hip_soft_kick_shots: i32,
    pub location_damage: [f32; crate::hitloc::HITLOC_COUNT],
    pub dual_mag: Option<crate::dual_mag::DualMagTimes>,
    /// bo2zm fix list 2: see `RecordedBattleFeed::spread_before_fire_add`.
    pub spread_before_fire_add: bool,
    /// bo2zm: see `RecordedBattleFeed::flat_spread`.
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

impl Default for GunBattleProfile {
    fn default() -> Self {
        GunBattleProfile::none()
    }
}

/// The zero, unset and off values every field of [`GunBattleProfile::none`] starts from.
const NIL_INT: i32 = 0;
const NIL_UINT: u32 = 0;
const NIL_FLOAT: f32 = 0.0;
const FLAG_OFF: bool = false;

impl GunBattleProfile {
    /// A weapon that does nothing.
    pub const fn none() -> Self {
        Self {
            aim_assist: AimAssistRanges::OFF,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
            shot_gap_ms: NIL_INT,
            intro_fire_time_ms: NIL_INT,
            intro_fire_length: NIL_INT,
            charge_shot: FLAG_OFF,
            shot_lead_ms: NIL_INT,
            draw_ms: NIL_INT,
            holster_ms: NIL_INT,
            alternate_weapon: NIL_UINT,
            alternate_raise_time_ms: NIL_INT,
            alternate_drop_time_ms: NIL_INT,
            reload_duration_millis: NIL_INT,
            refill_dry_millis: NIL_INT,
            magazine_capacity: NIL_INT,
            spawn_rounds: NIL_INT,
            reserve_cap: NIL_INT,
            ammo_type_slot: NIL_INT,
            motion_clip_slot: NIL_INT,
            trigger_mode: NIL_INT,
            gun_family_code: NIL_INT,
            gun_kind: NIL_INT,
            player_motion_kind: NIL_INT,
            carry_slot_kind: NIL_INT,
            quick_switch: FLAG_OFF,
            quick_switch_target: FLAG_OFF,
            impact_kind_id: NIL_INT,
            rounds_each_trigger: NIL_INT,
            volley_rest_millis: NIL_INT,
            manual_cycle: FLAG_OFF,
            cycle_ms: NIL_INT,
            bolt_rack_duration_millis: NIL_INT,
            bolt_rack_wait_millis: NIL_INT,
            staged_reload: FLAG_OFF,
            refill_begin_millis: NIL_INT,
            refill_finish_millis: NIL_INT,
            reload_rounds_gained: NIL_INT,
            reload_refill_millis: NIL_INT,
            empty_reload_refill_millis: NIL_INT,
            reload_begin_refill_millis: NIL_INT,
            reload_begin_bonus: NIL_INT,
            forbid_topup_reload: FLAG_OFF,
            unlimited_ammo: FLAG_OFF,
            carries_over_talents: FLAG_OFF,
            run_lift_millis: NIL_INT,
            run_lower_millis: NIL_INT,
            damage: NIL_INT,
            hurt_floor: NIL_INT,
            full_damage_reach: NIL_FLOAT,
            minimum_damage_reach: NIL_FLOAT,
            damage_steps: None,
            hip_cone_up_lo: NIL_FLOAT,
            hip_cone_crouch_lo: NIL_FLOAT,
            hip_cone_lie_lo: NIL_FLOAT,
            hip_cone_up_hi: NIL_FLOAT,
            hip_cone_crouch_hi: NIL_FLOAT,
            hip_cone_lie_hi: NIL_FLOAT,
            hip_cone_settle: NIL_FLOAT,
            hip_cone_per_shot: NIL_FLOAT,
            hip_cone_per_turn: NIL_FLOAT,
            hip_cone_per_step: NIL_FLOAT,
            hip_cone_crouch_settle: NIL_FLOAT,
            hip_cone_lie_settle: NIL_FLOAT,
            aimed_cone: NIL_FLOAT,
            scoped_aiming: FLAG_OFF,
            block_aim_if_clip_dry: FLAG_OFF,
            scope_raise_speed: NIL_FLOAT,
            scope_lower_speed: NIL_FLOAT,
            rack_bolt_when_scoped: true,
            reload_lowers_ads: FLAG_OFF,
            position_reload_trans_ms: NIL_INT,
            fire_only_when_scoped: FLAG_OFF,
            optic_switch: FLAG_OFF,
            bash_hurt: NIL_INT,
            hud_crosshair_layer: NIL_INT,
            bash_ms: NIL_INT,
            strike_wait_millis: NIL_INT,
            bash_windup_ms: NIL_INT,
            lunge_windup_wait_millis: NIL_INT,
            lunge_has_clip: FLAG_OFF,
            blade_mesh: NIL_UINT,
            fast_draw_ms: NIL_INT,
            fast_holster_ms: NIL_INT,
            select_requires_ammo: Some(false),
            offhand_hold_is_cancelable: Some(false),
            aim_soft_kick_shots: NIL_INT,
            hip_soft_kick_shots: NIL_INT,
            location_damage: crate::damage::LOCATION_DAMAGE_IDENTITY,
            dual_mag: None,
            spread_before_fire_add: FLAG_OFF,
            flat_spread: FLAG_OFF,
            dual_wield_weapon: NIL_UINT,
            spawned_grenade: NIL_UINT,
            retrievable: FLAG_OFF,
        }
    }

    /// The captured fields checked and taken in. A weapon with no fire time, raise time,
    /// damage or clip is [`Self::none`].
    pub fn try_from_captured(cap: RecordedBattleFeed) -> Result<Self, AbsentBattleProfile> {
        let nothing = cap.shot_gap_ms <= 0
            && cap.draw_ms <= 0
            && cap.damage <= 0
            && cap.magazine_capacity <= 0;
        if nothing {
            return Ok(GunBattleProfile::none());
        }
        let Ok(fire_type) = TriggerPattern::from_i32(cap.trigger_mode) else {
            return Err(AbsentBattleProfile::UnrecognisedTriggerPattern);
        };
        if fire_type.fires_volleys() && cap.volley_rest_millis <= 0 {
            return Err(AbsentBattleProfile::VolleyRest);
        }
        if cap.staged_reload && cap.reload_rounds_gained <= 0 {
            return Err(AbsentBattleProfile::PerRoundReloadGain);
        }
        if cap.damage > 0 && cap.full_damage_reach <= 0.0 && cap.minimum_damage_reach <= 0.0 {
            return Err(AbsentBattleProfile::ShotReach);
        }
        if !crate::damage::location_damage_is_valid(&cap.location_damage) {
            return Err(AbsentBattleProfile::LocationDamage);
        }
        Ok(Self {
            aim_assist: AimAssistRanges::OFF,
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
            shot_gap_ms: cap.shot_gap_ms,
            intro_fire_time_ms: cap.intro_fire_time_ms,
            intro_fire_length: cap.intro_fire_length,
            charge_shot: cap.charge_shot,
            shot_lead_ms: cap.shot_lead_ms,
            draw_ms: cap.draw_ms,
            holster_ms: cap.holster_ms,
            alternate_weapon: cap.alternate_weapon,
            alternate_raise_time_ms: cap.alternate_raise_time_ms,
            alternate_drop_time_ms: cap.alternate_drop_time_ms,
            reload_duration_millis: cap.reload_duration_millis,
            refill_dry_millis: cap.refill_dry_millis,
            magazine_capacity: cap.magazine_capacity,
            spawn_rounds: cap.spawn_rounds,
            reserve_cap: cap.reserve_cap,
            ammo_type_slot: cap.ammo_type_slot,
            motion_clip_slot: cap.motion_clip_slot,
            trigger_mode: cap.trigger_mode,
            gun_family_code: cap.gun_family_code,
            gun_kind: cap.gun_kind,
            player_motion_kind: cap.player_motion_kind,
            carry_slot_kind: cap.carry_slot_kind,
            quick_switch: cap.quick_switch,
            quick_switch_target: cap.quick_switch_target,
            impact_kind_id: cap.impact_kind_id,
            rounds_each_trigger: cap.rounds_each_trigger,
            volley_rest_millis: cap.volley_rest_millis,
            manual_cycle: cap.manual_cycle,
            cycle_ms: cap.cycle_ms,
            bolt_rack_duration_millis: cap.bolt_rack_duration_millis,
            bolt_rack_wait_millis: cap.bolt_rack_wait_millis,
            staged_reload: cap.staged_reload,
            refill_begin_millis: cap.refill_begin_millis,
            refill_finish_millis: cap.refill_finish_millis,
            reload_rounds_gained: cap.reload_rounds_gained,
            reload_refill_millis: cap.reload_refill_millis,
            empty_reload_refill_millis: cap.empty_reload_refill_millis,
            reload_begin_refill_millis: cap.reload_begin_refill_millis,
            reload_begin_bonus: cap.reload_begin_bonus,
            forbid_topup_reload: cap.forbid_topup_reload,
            unlimited_ammo: cap.unlimited_ammo,
            carries_over_talents: cap.carries_over_talents,
            run_lift_millis: cap.run_lift_millis,
            run_lower_millis: cap.run_lower_millis,
            damage: cap.damage,
            hurt_floor: cap.hurt_floor,
            full_damage_reach: cap.full_damage_reach,
            minimum_damage_reach: cap.minimum_damage_reach,
            damage_steps: cap.damage_steps,
            hip_cone_up_lo: cap.hip_cone_up_lo,
            hip_cone_crouch_lo: cap.hip_cone_crouch_lo,
            hip_cone_lie_lo: cap.hip_cone_lie_lo,
            hip_cone_up_hi: cap.hip_cone_up_hi,
            hip_cone_crouch_hi: cap.hip_cone_crouch_hi,
            hip_cone_lie_hi: cap.hip_cone_lie_hi,
            hip_cone_settle: cap.hip_cone_settle,
            hip_cone_per_shot: cap.hip_cone_per_shot,
            hip_cone_per_turn: cap.hip_cone_per_turn,
            hip_cone_per_step: cap.hip_cone_per_step,
            hip_cone_crouch_settle: cap.hip_cone_crouch_settle,
            hip_cone_lie_settle: cap.hip_cone_lie_settle,
            aimed_cone: cap.aimed_cone,
            scoped_aiming: cap.scoped_aiming,
            block_aim_if_clip_dry: cap.block_aim_if_clip_dry,
            scope_raise_speed: cap.scope_raise_speed,
            scope_lower_speed: cap.scope_lower_speed,
            rack_bolt_when_scoped: cap.rack_bolt_when_scoped,
            reload_lowers_ads: cap.reload_lowers_ads,
            position_reload_trans_ms: cap.position_reload_trans_ms,
            fire_only_when_scoped: cap.fire_only_when_scoped,
            optic_switch: cap.optic_switch,
            bash_hurt: cap.bash_hurt,
            hud_crosshair_layer: cap.hud_crosshair_layer,
            bash_ms: cap.bash_ms,
            strike_wait_millis: cap.strike_wait_millis,
            bash_windup_ms: cap.bash_windup_ms,
            lunge_windup_wait_millis: cap.lunge_windup_wait_millis,
            lunge_has_clip: cap.lunge_has_clip,
            blade_mesh: cap.blade_mesh,
            fast_draw_ms: cap.fast_draw_ms,
            fast_holster_ms: cap.fast_holster_ms,
            select_requires_ammo: cap.select_requires_ammo,
            offhand_hold_is_cancelable: cap.offhand_hold_is_cancelable,
            aim_soft_kick_shots: cap.aim_soft_kick_shots,
            hip_soft_kick_shots: cap.hip_soft_kick_shots,
            location_damage: cap.location_damage,
            dual_mag: cap.dual_mag,
            spread_before_fire_add: cap.spread_before_fire_add,
            flat_spread: cap.flat_spread,
            dual_wield_weapon: 0,
            spawned_grenade: 0,
            retrievable: false,
        })
    }

    pub fn is_none(self) -> bool {
        self.shot_gap_ms <= 0 && self.draw_ms <= 0 && self.damage <= 0 && self.magazine_capacity <= 0
    }

    pub fn ready_for_use(self) -> bool {
        !self.is_none()
    }

    /// Bullets a shot sends: a spread weapon's pellets, else 1.
    pub fn shot_pellet_total(self) -> i32 {
        if self.gun_kind == WEAPCLASS_SPREAD {
            self.rounds_each_trigger.max(1)
        } else {
            1
        }
    }

    /// How far a bullet reaches: a spread weapon's own range, else the world's.
    pub fn shot_reach(self) -> f32 {
        if self.gun_kind == WEAPCLASS_SPREAD {
            self.minimum_damage_reach
        } else {
            crate::damage::BULLET_MAX_RANGE
        }
    }

    /// Damage scale for a hit on `hitloc`.
    pub fn location_scale(self, hitloc: u8) -> f32 {
        crate::hitloc::location_scale(&self.location_damage, hitloc)
    }

    pub fn ads_allow_facts(self) -> crate::ads_allow::ScopePermitGunInfo {
        crate::ads_allow::ScopePermitGunInfo {
            scoped_aiming: self.scoped_aiming,
            block_aim_if_clip_dry: self.block_aim_if_clip_dry,
            motion_clip_slot: self.motion_clip_slot,
            optic_switch: self.optic_switch,
        }
    }

    /// What moves the sights: (rate in, rate out, rechamber while aimed, aimed fire only).
    pub fn scope_blend_bundle(self) -> (f32, f32, bool, bool) {
        (
            self.scope_raise_speed,
            self.scope_lower_speed,
            self.rack_bolt_when_scoped,
            self.fire_only_when_scoped,
        )
    }

    /// BO2's reload rule for the sights: (a reload lowers them, how much of
    /// the reload is left when they come back up, segmented reload).
    pub fn ads_reload_rule(self) -> (bool, i32, bool) {
        (
            self.reload_lowers_ads,
            self.position_reload_trans_ms,
            self.staged_reload,
        )
    }
}

/// A hand's weapon, its state and its rounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GunGripState {
    pub weapon: u32,
    pub gun_phase: i32,
    pub gun_timer_ms: i32,
    pub weapon_ready_wait: i32,
    pub gun_clip: i32,
    pub grip_slot: u8,
    pub mag_rounds: i32,
    pub spare_rounds: i32,
    pub rounds_fired: u8,
    pub burst_hold: bool,
    pub cycle_owed: bool,
    pub late_bolt_cycle: bool,
    pub gun_lockout_kick_stamp: i32,
    pub quick_reload: bool,
}

/// The rounds a weapon spawns with, as (right clip, left clip, stock). The left clip is
/// only filled once the left hand has been used.
pub fn spawn_clip_stock(stats: &GunBattleProfile, hand_idx: i32) -> (i32, i32, i32) {
    let pool = stats.spawn_rounds.max(0);
    let clip_cap = stats.magazine_capacity.max(0);
    let right = if clip_cap > 0 { pool.min(clip_cap) } else { 0 };
    let spare = pool - right;
    let left = if hand_idx >= 1 && clip_cap > 0 {
        spare.min(clip_cap)
    } else {
        0
    };
    (right, left, spare - left)
}

/// The right hand right after `weapon` is handed over: magazine filled from the start
/// ammo, the rest in stock, and the gun coming up when it has a raise time (ready at once
/// otherwise).
pub fn spawn_weapon_hand(gun: u32, stats: &GunBattleProfile) -> GunGripState {
    let (clip, _, stock) = spawn_clip_stock(stats, 0);
    let mut hand = GunGripState {
        weapon: gun,
        mag_rounds: clip,
        spare_rounds: stock,
        quick_reload: stats.dual_mag.is_some(),
        gun_phase: crate::state::READY,
        ..GunGripState::default()
    };
    let lift_ms = stats.draw_ms;
    if lift_ms > 0 {
        hand.gun_phase = crate::state::RAISING;
        hand.gun_timer_ms = lift_ms;
        crate::gun_anim::begin_gun_clip(&mut hand.gun_clip, crate::gun_anim::gun_clip_cue::LIFT);
    }
    hand
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> RecordedBattleFeed {
        RecordedBattleFeed {
            shot_gap_ms: 90,
            draw_ms: 400,
            damage: 30,
            magazine_capacity: 30,
            spawn_rounds: 90,
            full_damage_reach: 1000.0,
            minimum_damage_reach: 2000.0,
            location_damage: crate::damage::LOCATION_DAMAGE_IDENTITY,
            ..RecordedBattleFeed::default()
        }
    }

    #[test]
    fn an_empty_weapon_file_is_nothing() {
        let facts = GunBattleProfile::try_from_captured(RecordedBattleFeed::default()).unwrap();
        assert!(facts.is_none() && !facts.ready_for_use());
        assert_eq!(facts, GunBattleProfile::none());
    }

    #[test]
    fn a_rifle_is_taken_in_and_a_bad_file_names_what_is_missing() {
        let facts = GunBattleProfile::try_from_captured(rifle()).unwrap();
        assert!(facts.ready_for_use());
        assert_eq!((facts.shot_gap_ms, facts.magazine_capacity), (90, 30));
        assert_eq!(facts.shot_pellet_total(), 1);
        assert_eq!(facts.shot_reach(), crate::damage::BULLET_MAX_RANGE);

        let burst = RecordedBattleFeed {
            trigger_mode: 2,
            ..rifle()
        };
        assert_eq!(
            GunBattleProfile::try_from_captured(burst),
            Err(AbsentBattleProfile::VolleyRest)
        );
        let odd = RecordedBattleFeed {
            trigger_mode: 9,
            ..rifle()
        };
        assert_eq!(
            GunBattleProfile::try_from_captured(odd),
            Err(AbsentBattleProfile::UnrecognisedTriggerPattern)
        );
        let shells = RecordedBattleFeed {
            staged_reload: true,
            ..rifle()
        };
        assert_eq!(
            GunBattleProfile::try_from_captured(shells),
            Err(AbsentBattleProfile::PerRoundReloadGain)
        );
        let short = RecordedBattleFeed {
            full_damage_reach: 0.0,
            minimum_damage_reach: 0.0,
            ..rifle()
        };
        assert_eq!(
            GunBattleProfile::try_from_captured(short),
            Err(AbsentBattleProfile::ShotReach)
        );
        let mut bad = rifle();
        bad.location_damage[0] = f32::NAN;
        assert_eq!(
            GunBattleProfile::try_from_captured(bad),
            Err(AbsentBattleProfile::LocationDamage)
        );
    }

    #[test]
    fn a_shotgun_sends_its_pellets_and_reaches_its_own_range() {
        let spread = RecordedBattleFeed {
            gun_kind: WEAPCLASS_SPREAD,
            rounds_each_trigger: 8,
            ..rifle()
        };
        let facts = GunBattleProfile::try_from_captured(spread).unwrap();
        assert_eq!(facts.shot_pellet_total(), 8);
        assert_eq!(facts.shot_reach(), 2000.0);
        let none = GunBattleProfile {
            gun_kind: WEAPCLASS_SPREAD,
            ..facts
        };
        assert_eq!(
            GunBattleProfile {
                rounds_each_trigger: 0,
                ..none
            }
            .shot_pellet_total(),
            1
        );
    }

    #[test]
    fn spawn_ammo_fills_the_clip_then_the_left_clip_then_stock() {
        let facts = GunBattleProfile {
            spawn_rounds: 100,
            magazine_capacity: 30,
            ..GunBattleProfile::none()
        };
        assert_eq!(spawn_clip_stock(&facts, 0), (30, 0, 70));
        assert_eq!(spawn_clip_stock(&facts, 1), (30, 30, 40));
        let small = GunBattleProfile {
            spawn_rounds: 40,
            ..facts
        };
        assert_eq!(spawn_clip_stock(&small, 1), (30, 10, 0));
        assert_eq!(spawn_clip_stock(&GunBattleProfile::none(), 1), (0, 0, 0));
    }

    #[test]
    fn a_spawned_hand_raises_the_weapon_if_it_has_a_raise_time() {
        let facts = GunBattleProfile::try_from_captured(rifle()).unwrap();
        let hand = spawn_weapon_hand(7, &facts);
        assert_eq!(
            (hand.weapon, hand.gun_phase, hand.gun_timer_ms),
            (7, crate::state::RAISING, 400)
        );
        assert_eq!((hand.mag_rounds, hand.spare_rounds), (30, 60));
        assert_ne!(hand.gun_clip, 0);

        let ready = spawn_weapon_hand(
            7,
            &GunBattleProfile {
                draw_ms: Default::default(),
                ..facts
            },
        );
        assert_eq!(
            (ready.gun_phase, ready.gun_timer_ms, ready.gun_clip),
            (crate::state::READY, 0, 0)
        );
    }

    #[test]
    fn fast_reload_needs_the_perk_and_a_weapon_that_inherits_perks() {
        // BO2's perk 20 is bit 20 of the first word.
        let fast = [1 << 20, 0];
        assert!(perk_fastreload_eligible(fast, true));
        assert!(!perk_fastreload_eligible(fast, false));
        assert!(!perk_fastreload_eligible([0, 0], true));
        // Another perk's bit is not it, in either word.
        assert!(!perk_fastreload_eligible([1 << 4, !0 << 1], true));
    }
}
