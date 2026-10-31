//! The numbers and switches a weapon's definition gives the game rules, the
//! viewmodel and the HUD, grouped by what reads them.

use weapon_t6::placement::{GunRestDriftFeed, GunMotionShiftFeed};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponChassisSummary {
    /// The capture filled these facts from a real definition.
    pub payload_linked: bool,

    // Firing.
    pub shot_gap_ms: i32,
    /// The quicker first shots of one trigger pull: this far apart, this many.
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    /// bo2zm: Black Ops II's charge shot (fire type 8): Origins' upgraded
    /// staffs, held to charge and let go to fire.
    pub charge_shot: bool,
    pub shot_lead_ms: i32,
    pub trigger_hold_millis: i32,
    pub trigger_mode: i32,
    pub rounds_each_trigger: i32,
    pub impact_kind_id: i32,
    /// Hip shots aim with the spread from before the shot widens it.
    pub spread_before_fire_add: bool,
    /// bo2zm: a Zombies weapon whose variant carries the accuracy perk
    /// (PERK_ACCURACY, the Blundergat) lays its pellets out in a flat line
    /// across the aim (Bullet_Endpos), not a random cone.
    pub flat_spread: bool,

    // Switching.
    pub draw_ms: i32,
    pub holster_ms: i32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub fast_draw_ms: i32,
    pub fast_holster_ms: i32,
    /// Offhand-slot and vehicle weapons go away and come up with the quick times.
    pub quick_switch: bool,
    /// Pistols and melee weapons come up with the quick raise when switched to.
    pub quick_switch_target: bool,
    pub run_lift_millis: i32,
    pub run_cycle_millis: i32,
    pub run_lower_millis: i32,
    /// Dive to prone: take-off, airborne, landing.
    pub dive_times_ms: [i32; 3],
    /// Crawl: in, forward, back, right, left, out to fire, out.
    pub crawl_times_ms: [i32; 7],

    // What kind of weapon.
    pub gun_family_code: i32,
    /// The definition's own type number, unmapped (1 grenade, 2 projectile,
    /// 6 mine): the grenade warning picks projectiles by it.
    pub t6_weap_type: i32,
    /// The definition's own class and fire type numbers, unmapped: the scripts' `weaponclass`
    /// and `weaponissemiauto` answer by BO2's numbering, which the remaps above fold away.
    pub t6_weap_class: i32,
    pub t6_fire_type: i32,
    /// The definition's own `weapInventoryType_t`, unmapped (4 melee, 5 dual-wield left hand):
    /// `carry_slot_kind` folds anything past alt mode onto item.
    pub t6_inventory_type: i32,
    pub player_motion_kind: i32,
    pub gun_kind: i32,
    pub secondary_hand_category: i32,
    /// bo2zm: Black Ops II's `offhandSlot` (1 lethal, 2 tactical, 4 specific
    /// use): the slot its engine gives the offhand to, apart from its class
    /// (the Hell's Retriever is class 1 but slot 2, the tactical).
    pub offhand_slot: i32,
    /// bo2zm: `bRetrievable` (equipment and knives the player picks back up;
    /// the scripts' `getretrievableweapons`).
    pub retrievable: bool,
    pub carry_slot_kind: i32,
    pub carries_over_talents: bool,
    pub motion_tracker: bool,
    pub long_gun_round: bool,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub goggles_equip_duration: i32,

    // Ammo and reloads.
    pub ammo_type_slot: i32,
    pub motion_clip_slot: i32,
    pub magazine_capacity: i32,
    pub reserve_cap: i32,
    pub spawn_rounds: i32,
    /// `start_ammo` and `max_ammo` count clips, not rounds.
    pub rounds_in_magazine_relative: bool,
    pub motion_clip_exclusive: bool,
    pub scarce_rounds_alert_level: f32,
    /// This alternate mode feeds from its gun's magazine (both name the same
    /// ammo and clip, as Select Fire does); a launcher keeps its own.
    pub shares_alt_magazine: bool,
    pub reload_duration_millis: i32,
    pub refill_dry_millis: i32,
    pub reload_refill_millis: i32,
    pub empty_reload_refill_millis: i32,
    pub refill_begin_millis: i32,
    pub reload_begin_refill_millis: i32,
    pub refill_finish_millis: i32,
    pub rocket_reappear_millis: i32,
    pub reload_rounds_gained: i32,
    pub reload_begin_bonus: i32,
    pub forbid_topup_reload: bool,
    /// Fires with an empty clip and never uses ammo (Mob of the Dead's
    /// Afterlife lightning hands; an overheating gun heats up instead).
    pub unlimited_ammo: bool,
    pub staged_reload: bool,
    /// Fast Mag's quicker reloads.
    pub dual_mag: Option<weapon_t6::dual_mag::DualMagTimes>,
    pub cycle_ms: i32,
    pub bolt_rack_duration_millis: i32,
    pub bolt_rack_wait_millis: i32,
    pub manual_cycle: bool,

    // Hip spread and the hip crosshair.
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
    pub crosshair_arm_length: i32,
    pub crosshair_inner_gap: i32,
    pub hipfire_reticle_flank_offset: f32,

    // Aiming down the sights.
    pub scoped_aiming: bool,
    pub fire_only_when_scoped: bool,
    pub aimed_pitch: f32,
    pub aimed_cone: f32,
    pub aim_magnified_view: f32,
    pub aim_in_share: f32,
    pub aim_out_share: f32,
    pub scope_reticle_fade_in: f32,
    pub scope_reticle_fade_out: f32,
    pub scope_raise_speed: f32,
    pub scope_lower_speed: f32,
    pub aim_blur_depth: Option<[f32; 2]>,
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub block_aim_if_clip_dry: bool,
    pub rack_bolt_when_scoped: bool,
    /// A reload drops the sights.
    pub reload_lowers_ads: bool,
    /// The sights come back up with this much of the reload left.
    pub position_reload_trans_ms: i32,
    /// Sprint steadies the scope (sniper rifles).
    pub hold_breath_to_steady: bool,

    // Sights, scopes and their overlays.
    pub hud_crosshair_layer: i32,
    pub overlay_interface: i32,
    pub scope_pic_w: f32,
    pub scope_pic_h: f32,
    /// The scope overlay's alpha divides by the larger of 1 and this.
    pub overlay_alpha_scale: f32,
    /// A sight's dot or ring drawn at infinity: its picture spans 1 / this
    /// in tangent of the view angle; 0 means no such reticle.
    pub sight_reticle_scale: f32,
    /// Variable Zoom: the scope's field of view and the closer one Sprint
    /// zooms to; zeros for a fixed zoom.
    pub variable_zoom_fovs: [f32; 2],
    /// Hybrid Optic: Sprint while aimed swaps to the other optic, which is
    /// the alternate weapon.
    pub optic_switch: bool,
    /// Full aim shows the thermal view (Dual Band).
    pub infrared: bool,
    /// Target Finder: its screen and enemy markers show at full aim.
    pub rangefinder: bool,

    // Melee.
    pub bash_hurt: i32,
    pub bash_ms: i32,
    pub strike_wait_millis: i32,
    pub bash_windup_ms: i32,
    pub lunge_windup_wait_millis: i32,
    pub blade_mesh: u32,
    pub use_as_melee: bool,

    // Carrying.
    pub walk_factor: f32,
    pub aimed_walk_factor: f32,
    pub run_time_multiplier: f32,
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    pub movement: GunMotionShiftFeed,
    pub idle: GunRestDriftFeed,
    pub twin_grip_viewmodel_shift: f32,
    pub forbid_twin_grip: bool,
    /// The gun on a player's back: offset and pitch, yaw, roll in degrees
    /// from the tag it hangs on.
    pub stowed_offset: [f32; 3],
    pub stowed_rotation: [f32; 3],

    // Damage.
    pub damage: i32,
    pub hurt_floor: i32,
    pub smallest_hurt_amount: i32,
    pub full_damage_reach: f32,
    pub minimum_damage_reach: f32,
    /// Six damage points by range; None keeps near and far only.
    pub damage_steps: Option<weapon_t6::damage::DamageSteps>,
    pub location_damage_mult: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub wall_pierce_class: i32,
    /// An attachment already set `wall_pierce_class` (the first one with a type wins).
    pub penetrate_type_from_attachment: bool,
    pub wall_pierce_scale: f32,
    /// A suppressed gun puts no firing ping on the enemy's minimap.
    pub silenced: bool,

    // Grenades, rockets and other projectiles.
    pub detonation_delay_millis: i32,
    pub grenade_cookoff_wait: bool,
    pub has_detonator: bool,
    pub detonate_delay_ms: i32,
    pub detonate_time_ms: i32,
    pub timed_detonation: bool,
    /// A fuse that runs out in the air waits for the ground.
    pub explode_on_ground: bool,
    pub shell_detonate_on_hit: bool,
    /// bo2zm: `bBulletImpactExplode`: a bullet weapon whose bullet explodes
    /// where it hits (the explosion radius and damages).
    pub bullet_impact_explode: bool,
    /// bo2zm: `bForceBounce`: a rocket (projectile) that bounces off what it
    /// hits; any other rocket that does not explode on impact hits and is
    /// gone (`G_FireRocket` sets the bounce flag only from this;
    /// `G_InitGrenadeEntity` always sets it for grenades).
    pub force_bounce: bool,
    /// The Sensor Grenade's lasers and its minimap marks.
    pub laser_sensor: bool,
    pub adheres_to_bodies: bool,
    pub stickiness: i32,
    pub projectile_rotates: bool,
    /// 0 a grenade's tumble, 1 a blade's end-over-end spin, 2 a cylinder's.
    pub projectile_rotate_type: i32,
    pub missile_velocity: i32,
    pub missile_upward_velocity: i32,
    pub projectile_speed_forward: i32,
    /// Extra speed along the thrower's view up.
    pub projectile_speed_relative_up: i32,
    /// How much of the thrower's own velocity the projectile keeps.
    pub projectile_parent_velocity: Option<f32>,
    /// A rolling grenade, and the radius it rests and rolls on.
    pub rolling_radius: Option<f32>,
    pub shell_arming_range: i32,
    pub shell_blast_kind: i32,
    pub along_rebound: Option<[f32; 31]>,
    pub across_rebound: Option<[f32; 31]>,
    pub blast_outer_reach: i32,
    pub blast_reach_floor: i32,
    pub blast_core_damage: i32,
    pub blast_rim_damage: i32,
    pub damage_cone_angle: f32,
    /// How near the grenade warning shows; 0 uses the warning's own range or
    /// the blast's, whichever is bigger.
    pub indicator_radius: i32,
    pub missile_guidance: i32,
    /// How hard a lock-on rocket steers toward its target.
    pub max_steering_accel: f32,
    pub ignition_delay_ms: i32,
    pub require_lock_to_fire: bool,

    // Aim assist.
    pub auto_aim_range: f32,
    pub aim_assist_range: f32,
    pub aim_assist_range_ads: f32,

    pub kick: WeaponRecoilSummary,
    pub sway: WeaponDriftSummary,
    pub bob: WeaponBobFacts,
}

impl WeaponChassisSummary {
    /// Rounds a fresh pickup carries.
    pub fn opening_rounds(&self) -> i32 {
        self.in_rounds(self.spawn_rounds)
    }

    /// Most rounds the weapon can hold.
    pub fn rounds_ceiling(&self) -> i32 {
        self.in_rounds(self.reserve_cap)
    }

    /// `count` is in clips when the weapon counts ammo relative to its clip.
    fn in_rounds(&self, count: i32) -> i32 {
        let per_clip = if self.rounds_in_magazine_relative { self.magazine_capacity.max(0) } else { 0 };
        if per_clip > 0 && count > 0 { count.saturating_mul(per_clip) } else { count }
    }
}

/// How shots push the gun and the view, hip and aimed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponRecoilSummary {
    pub hipfire_view_kick_recenter_speed: f32,
    pub scoped_view_kick_recenter_speed: f32,
    pub weapon_pitch_ceiling: f32,
    pub weapon_yaw_ceiling: f32,

    pub hip_soft_kick_rounds: i32,
    pub hipfire_weapon_kick_damped_percent: f32,
    pub hipfire_weapon_kick_pitch_bottom: f32,
    pub hipfire_weapon_kick_pitch_top: f32,
    pub hipfire_weapon_kick_yaw_bottom: f32,
    pub hipfire_weapon_kick_yaw_top: f32,
    pub hipfire_weapon_kick_accel: f32,
    pub hipfire_weapon_kick_velocity_cap: f32,
    pub hipfire_weapon_kick_velocity_falloff: f32,
    pub hipfire_weapon_kick_resting_falloff: f32,
    pub hipfire_camera_kick_pitch_bottom: f32,
    pub hipfire_camera_kick_pitch_top: f32,
    pub hipfire_camera_kick_yaw_bottom: f32,
    pub hipfire_camera_kick_yaw_top: f32,
    /// The smallest view kick one shot gives.
    pub hip_view_kick_min_magnitude: f32,

    pub aimed_soft_kick_rounds: i32,
    pub scoped_weapon_kick_damped_percent: f32,
    pub scoped_weapon_kick_pitch_bottom: f32,
    pub scoped_weapon_kick_pitch_top: f32,
    pub scoped_weapon_kick_yaw_bottom: f32,
    pub scoped_weapon_kick_yaw_top: f32,
    pub scoped_weapon_kick_accel: f32,
    pub scoped_weapon_kick_velocity_cap: f32,
    pub scoped_weapon_kick_velocity_falloff: f32,
    pub scoped_weapon_kick_resting_falloff: f32,
    pub scoped_camera_kick_pitch_bottom: f32,
    pub scoped_camera_kick_pitch_top: f32,
    pub scoped_camera_kick_yaw_bottom: f32,
    pub scoped_camera_kick_yaw_top: f32,
    pub ads_view_kick_min_magnitude: f32,
}

/// The gun's sway as the view turns, hip and aimed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponDriftSummary {
    pub drift_cap: f32,
    pub drift_ease: f32,
    pub drift_pitch_k: f32,
    pub drift_yaw_k: f32,
    pub drift_side_k: f32,
    pub drift_up_k: f32,
    pub drift_stun_k: f32,

    pub aim_drift_angle_cap: f32,
    pub aim_drift_blend_rate: f32,
    pub aim_drift_tilt_gain: f32,
    pub aim_drift_turn_gain: f32,
    pub aim_drift_sideways_gain: f32,
    pub aim_drift_vertical_gain: f32,
}

impl WeaponDriftSummary {
    pub fn unaimed_settings(&self) -> weapon_t6::sway::GunDriftTuning {
        Self::params_from([
            self.drift_cap,
            self.drift_ease,
            self.drift_pitch_k,
            self.drift_yaw_k,
            self.drift_side_k,
            self.drift_up_k,
        ])
    }

    pub fn aim_down_settings(&self) -> weapon_t6::sway::GunDriftTuning {
        Self::params_from([
            self.aim_drift_angle_cap,
            self.aim_drift_blend_rate,
            self.aim_drift_tilt_gain,
            self.aim_drift_turn_gain,
            self.aim_drift_sideways_gain,
            self.aim_drift_vertical_gain,
        ])
    }

    /// The six sway numbers, in the order the runtime lists them.
    fn params_from(values: [f32; 6]) -> weapon_t6::sway::GunDriftTuning {
        let [widest_cone, ease_rate, tilt_multiplier, turn_factor, sideways_factor, upward_factor] = values;
        weapon_t6::sway::GunDriftTuning {
            widest_cone,
            ease_rate,
            tilt_multiplier,
            turn_factor,
            sideways_factor,
            upward_factor,
        }
    }
}

/// Sprint and dive bob: the cycle scales speed up the footstep rhythm, each
/// pair scales the gun's side-to-side and up-down bob. 1.0 leaves it as is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponBobFacts {
    pub sprint_cycle_scale: f32,
    pub ducked_sprint_cycle_scale: f32,
    pub dtp_cycle_scale: f32,
    pub sprint_bob: [f32; 2],
    pub dtp_bob: [f32; 2],
}

impl Default for WeaponBobFacts {
    fn default() -> Self {
        const NEUTRAL: f32 = 1.0;
        Self {
            sprint_cycle_scale: NEUTRAL,
            ducked_sprint_cycle_scale: NEUTRAL,
            dtp_cycle_scale: NEUTRAL,
            sprint_bob: [NEUTRAL; 2],
            dtp_bob: [NEUTRAL; 2],
        }
    }
}

/// Which create-a-class offhand slot a grenade fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadoutThrowableGroup {
    Lethal,
    Tactical,
}

/// The slot for an offhand class: frag, flash-free throwables and the
/// gear classes are lethal, smoke and flash are tactical.
pub fn cac_offhand_bucket(secondary_hand_category: i32) -> Option<LoadoutThrowableGroup> {
    const BUCKETS: [(LoadoutThrowableGroup, &[i32]); 2] = [
        (LoadoutThrowableGroup::Lethal, &[1, 4, 5]),
        (LoadoutThrowableGroup::Tactical, &[2, 3]),
    ];
    BUCKETS
        .iter()
        .find(|(_, classes)| classes.contains(&secondary_hand_category))
        .map(|&(bucket, _)| bucket)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counting(clip_relative: bool, reel: i32, start: i32, max: i32) -> WeaponChassisSummary {
        WeaponChassisSummary {
            rounds_in_magazine_relative: clip_relative,
            magazine_capacity: reel,
            spawn_rounds: start,
            reserve_cap: max,
            ..Default::default()
        }
    }

    #[test]
    fn clip_relative_ammo_is_multiplied_out() {
        let facts = counting(true, 30, 3, 5);
        assert_eq!((facts.opening_rounds(), facts.rounds_ceiling()), (90, 150));
    }

    #[test]
    fn plain_ammo_counts_stay_as_they_are() {
        for facts in [counting(false, 30, 3, 5), counting(true, 0, 3, 5), counting(true, -4, 3, 5)] {
            assert_eq!((facts.opening_rounds(), facts.rounds_ceiling()), (3, 5));
        }
        assert_eq!(counting(true, 30, 0, -2).rounds_ceiling(), -2);
        assert_eq!(counting(true, i32::MAX, 0, 2).rounds_ceiling(), i32::MAX);
    }

    #[test]
    fn offhand_classes_land_in_their_bucket() {
        use LoadoutThrowableGroup::{Lethal, Tactical};
        let got: Vec<_> = (0..7).map(cac_offhand_bucket).collect();
        assert_eq!(got, [None, Some(Lethal), Some(Tactical), Some(Tactical), Some(Lethal), Some(Lethal), None]);
        assert_eq!(cac_offhand_bucket(-1), None);
    }

    #[test]
    fn sway_params_pick_the_hip_or_aimed_set() {
        let sway = WeaponDriftSummary {
            drift_cap: 1.0,
            drift_up_k: 6.0,
            aim_drift_angle_cap: 11.0,
            aim_drift_vertical_gain: 16.0,
            ..WeaponDriftSummary::default()
        };
        let (hip, ads) = (sway.unaimed_settings(), sway.aim_down_settings());
        assert_eq!([hip.widest_cone, hip.upward_factor], [1.0, 6.0]);
        assert_eq!([ads.widest_cone, ads.upward_factor], [11.0, 16.0]);
    }

    #[test]
    fn bob_defaults_leave_the_gun_alone() {
        let bob = WeaponBobFacts::default();
        assert_eq!(bob.sprint_bob, [1.0, 1.0]);
        assert_eq!([bob.sprint_cycle_scale, bob.ducked_sprint_cycle_scale, bob.dtp_cycle_scale], [1.0; 3]);
    }
}
