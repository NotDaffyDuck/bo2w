//! The numbers and switches a weapon's definition gives the game rules, the
//! viewmodel and the HUD, grouped by what reads them.

use weapon_t6::placement::{WeaponIdleInputs, WeaponMovementOfsInputs};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponBodyFacts {
    /// The capture filled these facts from a real definition.
    pub body_resolved: bool,

    // Firing.
    pub fire_time_ms: i32,
    /// The quicker first shots of one trigger pull: this far apart, this many.
    pub intro_fire_time_ms: i32,
    pub intro_fire_length: i32,
    pub fire_delay_ms: i32,
    pub hold_fire_time_ms: i32,
    pub fire_type: i32,
    pub shots_per_fire: i32,
    pub impact_type: i32,
    /// Hip shots aim with the spread from before the shot widens it.
    pub spread_before_fire_add: bool,

    // Switching.
    pub raise_time_ms: i32,
    pub drop_time_ms: i32,
    pub alternate_raise_time_ms: i32,
    pub alternate_drop_time_ms: i32,
    pub quick_raise_time_ms: i32,
    pub quick_drop_time_ms: i32,
    /// Offhand-slot and vehicle weapons go away and come up with the quick times.
    pub quick_switch: bool,
    /// Pistols and melee weapons come up with the quick raise when switched to.
    pub quick_switch_target: bool,
    pub sprint_raise_time_ms: i32,
    pub sprint_loop_time_ms: i32,
    pub sprint_drop_time_ms: i32,
    /// Dive to prone: take-off, airborne, landing.
    pub dive_times_ms: [i32; 3],
    /// Crawl: in, forward, back, right, left, out to fire, out.
    pub crawl_times_ms: [i32; 7],

    // What kind of weapon.
    pub weap_type: i32,
    /// The definition's own type number, unmapped (1 grenade, 2 projectile,
    /// 6 mine): the grenade warning picks projectiles by it.
    pub t6_weap_type: i32,
    pub player_anim_type: i32,
    pub weap_class: i32,
    pub offhand_class: i32,
    pub inventory_type: i32,
    pub inherits_perks: bool,
    pub motion_tracker: bool,
    pub rifle_bullet: bool,
    pub select_requires_ammo: Option<bool>,
    pub offhand_hold_is_cancelable: Option<bool>,
    pub night_vision_wear_time: i32,

    // Ammo and reloads.
    pub ammo_index: i32,
    pub clip_index: i32,
    pub clip_size: i32,
    pub max_ammo: i32,
    pub start_ammo: i32,
    /// `start_ammo` and `max_ammo` count clips, not rounds.
    pub ammo_count_clip_relative: bool,
    pub clip_only: bool,
    pub low_ammo_warning_threshold: f32,
    /// This alternate mode feeds from its gun's magazine (both name the same
    /// ammo and clip, as Select Fire does); a launcher keeps its own.
    pub shares_alt_magazine: bool,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    pub reload_add_time_ms: i32,
    pub reload_empty_add_time_ms: i32,
    pub reload_start_time_ms: i32,
    pub reload_start_add_time_ms: i32,
    pub reload_end_time_ms: i32,
    pub reload_show_rocket_time_ms: i32,
    pub reload_ammo_add: i32,
    pub reload_start_add: i32,
    pub no_partial_reload: bool,
    pub segmented_reload: bool,
    /// Fast Mag's quicker reloads.
    pub dual_mag: Option<weapon_t6::dual_mag::DualMagTimes>,
    pub rechamber_time_ms: i32,
    pub rechamber_bolt_time_ms: i32,
    pub rechamber_bolt_delay_ms: i32,
    pub bolt_action: bool,

    // Hip spread and the hip crosshair.
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
    pub i_reticle_side_size: i32,
    pub i_reticle_min_ofs: i32,
    pub hip_reticle_side_pos: f32,

    // Aiming down the sights.
    pub aim_down_sight: bool,
    pub ads_fire_only: bool,
    pub ads_aim_pitch: f32,
    pub ads_spread: f32,
    pub ads_zoom_fov: f32,
    pub ads_zoom_in_frac: f32,
    pub ads_zoom_out_frac: f32,
    pub ads_crosshair_in_frac: f32,
    pub ads_crosshair_out_frac: f32,
    pub ads_in_rate: f32,
    pub ads_out_rate: f32,
    pub ads_dof: Option<[f32; 2]>,
    pub ads_bob_factor: f32,
    pub ads_view_bob_mult: f32,
    pub no_ads_when_mag_empty: bool,
    pub rechamber_while_ads: bool,
    /// A reload drops the sights.
    pub reload_lowers_ads: bool,
    /// The sights come back up with this much of the reload left.
    pub position_reload_trans_ms: i32,
    /// Sprint steadies the scope (sniper rifles).
    pub hold_breath_to_steady: bool,

    // Sights, scopes and their overlays.
    pub overlay_reticle: i32,
    pub overlay_interface: i32,
    pub ads_overlay_width: f32,
    pub ads_overlay_height: f32,
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
    pub melee_damage: i32,
    pub melee_time_ms: i32,
    pub melee_delay_ms: i32,
    pub melee_charge_time_ms: i32,
    pub melee_charge_delay_ms: i32,
    pub knife_model: u32,
    pub use_as_melee: bool,

    // Carrying.
    pub move_speed_scale: f32,
    pub ads_move_speed_scale: f32,
    pub sprint_duration_scale: f32,
    pub ducked_ofs: [f32; 3],
    pub prone_ofs: [f32; 3],
    pub movement: WeaponMovementOfsInputs,
    pub idle: WeaponIdleInputs,
    pub dual_wield_view_model_offset: f32,
    pub no_dual_wield: bool,
    /// The gun on a player's back: offset and pitch, yaw, roll in degrees
    /// from the tag it hangs on.
    pub stowed_offset: [f32; 3],
    pub stowed_rotation: [f32; 3],

    // Damage.
    pub damage: i32,
    pub min_damage: i32,
    pub min_player_damage: i32,
    pub max_damage_range: f32,
    pub min_damage_range: f32,
    /// Six damage points by range; None keeps near and far only.
    pub damage_steps: Option<weapon_t6::damage::DamageSteps>,
    pub location_damage_mult: Option<[f32; weapon_t6::hitloc::HITLOC_COUNT]>,
    pub penetrate_type: i32,
    /// An attachment already set `penetrate_type` (the first one with a type wins).
    pub penetrate_type_from_attachment: bool,
    pub penetrate_multiplier: f32,
    /// A suppressed gun puts no firing ping on the enemy's minimap.
    pub silenced: bool,

    // Grenades, rockets and other projectiles.
    pub fuse_time_ms: i32,
    pub cook_off_hold: bool,
    pub has_detonator: bool,
    pub detonate_delay_ms: i32,
    pub detonate_time_ms: i32,
    pub timed_detonation: bool,
    /// A fuse that runs out in the air waits for the ground.
    pub explode_on_ground: bool,
    pub proj_impact_explode: bool,
    /// The Sensor Grenade's lasers and its minimap marks.
    pub laser_sensor: bool,
    pub stick_to_players: bool,
    pub stickiness: i32,
    pub projectile_rotates: bool,
    /// 0 a grenade's tumble, 1 a blade's end-over-end spin, 2 a cylinder's.
    pub projectile_rotate_type: i32,
    pub projectile_speed: i32,
    pub projectile_speed_up: i32,
    pub projectile_speed_forward: i32,
    /// Extra speed along the thrower's view up.
    pub projectile_speed_relative_up: i32,
    /// How much of the thrower's own velocity the projectile keeps.
    pub projectile_parent_velocity: Option<f32>,
    /// A rolling grenade, and the radius it rests and rolls on.
    pub rolling_radius: Option<f32>,
    pub projectile_activate_dist: i32,
    pub projectile_explosion_type: i32,
    pub parallel_bounce: Option<[f32; 31]>,
    pub perpendicular_bounce: Option<[f32; 31]>,
    pub explosion_radius: i32,
    pub explosion_radius_min: i32,
    pub explosion_inner_damage: i32,
    pub explosion_outer_damage: i32,
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

    pub kick: WeaponKickFacts,
    pub sway: WeaponSwayFacts,
    pub bob: WeaponBobFacts,
}

impl WeaponBodyFacts {
    /// Rounds a fresh pickup carries.
    pub fn start_ammo_rounds(&self) -> i32 {
        self.in_rounds(self.start_ammo)
    }

    /// Most rounds the weapon can hold.
    pub fn max_ammo_rounds(&self) -> i32 {
        self.in_rounds(self.max_ammo)
    }

    fn in_rounds(&self, count: i32) -> i32 {
        match self.ammo_count_clip_relative && self.clip_size > 0 && count > 0 {
            true => count.saturating_mul(self.clip_size),
            false => count,
        }
    }
}

/// How shots push the gun and the view, hip and aimed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponKickFacts {
    pub f_hip_view_kick_center_speed: f32,
    pub f_ads_view_kick_center_speed: f32,
    pub gun_max_pitch: f32,
    pub gun_max_yaw: f32,

    pub hip_gun_kick_reduced_kick_bullets: i32,
    pub hip_gun_kick_reduced_kick_percent: f32,
    pub hip_gun_kick_pitch_min: f32,
    pub hip_gun_kick_pitch_max: f32,
    pub hip_gun_kick_yaw_min: f32,
    pub hip_gun_kick_yaw_max: f32,
    pub hip_gun_kick_accel: f32,
    pub hip_gun_kick_speed_max: f32,
    pub hip_gun_kick_speed_decay: f32,
    pub hip_gun_kick_static_decay: f32,
    pub hip_view_kick_pitch_min: f32,
    pub hip_view_kick_pitch_max: f32,
    pub hip_view_kick_yaw_min: f32,
    pub hip_view_kick_yaw_max: f32,
    /// The smallest view kick one shot gives.
    pub hip_view_kick_min_magnitude: f32,

    pub ads_gun_kick_reduced_kick_bullets: i32,
    pub ads_gun_kick_reduced_kick_percent: f32,
    pub ads_gun_kick_pitch_min: f32,
    pub ads_gun_kick_pitch_max: f32,
    pub ads_gun_kick_yaw_min: f32,
    pub ads_gun_kick_yaw_max: f32,
    pub ads_gun_kick_accel: f32,
    pub ads_gun_kick_speed_max: f32,
    pub ads_gun_kick_speed_decay: f32,
    pub ads_gun_kick_static_decay: f32,
    pub ads_view_kick_pitch_min: f32,
    pub ads_view_kick_pitch_max: f32,
    pub ads_view_kick_yaw_min: f32,
    pub ads_view_kick_yaw_max: f32,
    pub ads_view_kick_min_magnitude: f32,
}

/// The gun's sway as the view turns, hip and aimed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponSwayFacts {
    pub sway_max_angle: f32,
    pub sway_lerp_speed: f32,
    pub sway_pitch_scale: f32,
    pub sway_yaw_scale: f32,
    pub sway_horiz_scale: f32,
    pub sway_vert_scale: f32,
    pub sway_shell_shock_scale: f32,

    pub ads_sway_max_angle: f32,
    pub ads_sway_lerp_speed: f32,
    pub ads_sway_pitch_scale: f32,
    pub ads_sway_yaw_scale: f32,
    pub ads_sway_horiz_scale: f32,
    pub ads_sway_vert_scale: f32,
}

impl WeaponSwayFacts {
    pub fn hip_params(&self) -> weapon_t6::sway::WeaponSwayParams {
        weapon_t6::sway::WeaponSwayParams {
            max_angle: self.sway_max_angle,
            lerp_speed: self.sway_lerp_speed,
            pitch_scale: self.sway_pitch_scale,
            yaw_scale: self.sway_yaw_scale,
            horiz_scale: self.sway_horiz_scale,
            vert_scale: self.sway_vert_scale,
        }
    }

    pub fn ads_params(&self) -> weapon_t6::sway::WeaponSwayParams {
        weapon_t6::sway::WeaponSwayParams {
            max_angle: self.ads_sway_max_angle,
            lerp_speed: self.ads_sway_lerp_speed,
            pitch_scale: self.ads_sway_pitch_scale,
            yaw_scale: self.ads_sway_yaw_scale,
            horiz_scale: self.ads_sway_horiz_scale,
            vert_scale: self.ads_sway_vert_scale,
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
        Self {
            sprint_cycle_scale: 1.0,
            ducked_sprint_cycle_scale: 1.0,
            dtp_cycle_scale: 1.0,
            sprint_bob: [1.0, 1.0],
            dtp_bob: [1.0, 1.0],
        }
    }
}

/// Which create-a-class offhand slot a grenade fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacOffhandBucket {
    Lethal,
    Tactical,
}

/// The slot for an offhand class: frag, flash-free throwables and the
/// gear classes are lethal, smoke and flash are tactical.
pub fn cac_offhand_bucket(offhand_class: i32) -> Option<CacOffhandBucket> {
    match offhand_class {
        1 | 4 | 5 => Some(CacOffhandBucket::Lethal),
        2 | 3 => Some(CacOffhandBucket::Tactical),
        _ => None,
    }
}
