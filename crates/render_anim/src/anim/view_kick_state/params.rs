//! The weapon-file numbers that decide how a shot kicks.
//!
//! Every weapon carries two complete sets, one for firing from the hip and one
//! for aiming down sights. The code that fires or steps a kick blends between
//! the two by how far into the sights the weapon is (0 = hip, 1 = fully aimed).

use weapon_t6::kick::{GunKickRange, GunKickSpring, ViewKickRange};

/// Everything a weapon file says about its kick.
///
/// Angles are in degrees. A "range" is a pair of limits the game picks a random
/// value between on each shot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KickParams {
    /// How quickly the view kick is pulled back to centre while aimed.
    pub f_ads_view_kick_center_speed: f32,
    /// How quickly the view kick is pulled back to centre from the hip.
    pub f_hip_view_kick_center_speed: f32,
    /// The furthest the gun may swing up or down before it is held there.
    pub gun_max_pitch: f32,
    /// The furthest the gun may swing sideways before it is held there.
    pub gun_max_yaw: f32,

    /// Percent of the gun kick left while the "reduced kick" window is open, aimed.
    pub ads_gun_kick_reduced_kick_percent: f32,
    /// Aimed gun kick: lowest and highest pitch picked per shot.
    pub ads_gun_kick_pitch_min: f32,
    pub ads_gun_kick_pitch_max: f32,
    /// Aimed gun kick: lowest and highest yaw picked per shot.
    pub ads_gun_kick_yaw_min: f32,
    pub ads_gun_kick_yaw_max: f32,
    /// Aimed gun spring: the pull back toward centre.
    pub ads_gun_kick_accel: f32,
    /// Aimed gun spring: the fastest the gun may turn.
    pub ads_gun_kick_speed_max: f32,
    /// Aimed gun spring: the share of its speed the gun loses each second.
    pub ads_gun_kick_speed_decay: f32,
    /// Aimed gun spring: speed lost each second whatever the speed is.
    pub ads_gun_kick_static_decay: f32,
    /// Aimed view kick: lowest and highest pitch picked per shot.
    pub ads_view_kick_pitch_min: f32,
    pub ads_view_kick_pitch_max: f32,
    /// Aimed view kick: lowest and highest yaw picked per shot.
    pub ads_view_kick_yaw_min: f32,
    pub ads_view_kick_yaw_max: f32,

    /// Percent of the gun kick left while the "reduced kick" window is open, from the hip.
    pub hip_gun_kick_reduced_kick_percent: f32,
    /// Hip gun kick: lowest and highest pitch picked per shot.
    pub hip_gun_kick_pitch_min: f32,
    pub hip_gun_kick_pitch_max: f32,
    /// Hip gun kick: lowest and highest yaw picked per shot.
    pub hip_gun_kick_yaw_min: f32,
    pub hip_gun_kick_yaw_max: f32,
    /// Hip gun spring: the pull back toward centre.
    pub hip_gun_kick_accel: f32,
    /// Hip gun spring: the fastest the gun may turn.
    pub hip_gun_kick_speed_max: f32,
    /// Hip gun spring: the share of its speed the gun loses each second.
    pub hip_gun_kick_speed_decay: f32,
    /// Hip gun spring: speed lost each second whatever the speed is.
    pub hip_gun_kick_static_decay: f32,
    /// Hip view kick: lowest and highest pitch picked per shot.
    pub hip_view_kick_pitch_min: f32,
    pub hip_view_kick_pitch_max: f32,
    /// Hip view kick: lowest and highest yaw picked per shot.
    pub hip_view_kick_yaw_min: f32,
    pub hip_view_kick_yaw_max: f32,

    /// bo2zm: the smallest view kick a Black Ops II shot from the hip may have;
    /// a weaker roll is stretched up to this. 0 turns the floor off.
    pub hip_view_kick_min_magnitude: f32,
    /// The same floor while fully aimed.
    pub ads_view_kick_min_magnitude: f32,
}

impl KickParams {
    /// The hip-fire view kick limits, in the shape the recoil maths takes.
    pub(super) fn hip_view_range(&self) -> ViewKickRange {
        ViewKickRange {
            pitch_min: self.hip_view_kick_pitch_min,
            pitch_max: self.hip_view_kick_pitch_max,
            yaw_min: self.hip_view_kick_yaw_min,
            yaw_max: self.hip_view_kick_yaw_max,
        }
    }

    /// The aimed view kick limits.
    pub(super) fn ads_view_range(&self) -> ViewKickRange {
        ViewKickRange {
            pitch_min: self.ads_view_kick_pitch_min,
            pitch_max: self.ads_view_kick_pitch_max,
            yaw_min: self.ads_view_kick_yaw_min,
            yaw_max: self.ads_view_kick_yaw_max,
        }
    }

    /// The hip-fire gun kick limits.
    pub(super) fn hip_gun_range(&self) -> GunKickRange {
        GunKickRange {
            pitch_min: self.hip_gun_kick_pitch_min,
            pitch_max: self.hip_gun_kick_pitch_max,
            yaw_min: self.hip_gun_kick_yaw_min,
            yaw_max: self.hip_gun_kick_yaw_max,
        }
    }

    /// The aimed gun kick limits.
    pub(super) fn ads_gun_range(&self) -> GunKickRange {
        GunKickRange {
            pitch_min: self.ads_gun_kick_pitch_min,
            pitch_max: self.ads_gun_kick_pitch_max,
            yaw_min: self.ads_gun_kick_yaw_min,
            yaw_max: self.ads_gun_kick_yaw_max,
        }
    }

    /// The spring that pulls the gun back when fired from the hip.
    pub(super) fn hip_spring(&self) -> GunKickSpring {
        GunKickSpring {
            accel: self.hip_gun_kick_accel,
            speed_max: self.hip_gun_kick_speed_max,
            speed_decay: self.hip_gun_kick_speed_decay,
            static_decay: self.hip_gun_kick_static_decay,
        }
    }

    /// The spring that pulls the gun back when aimed.
    pub(super) fn ads_spring(&self) -> GunKickSpring {
        GunKickSpring {
            accel: self.ads_gun_kick_accel,
            speed_max: self.ads_gun_kick_speed_max,
            speed_decay: self.ads_gun_kick_speed_decay,
            static_decay: self.ads_gun_kick_static_decay,
        }
    }
}
