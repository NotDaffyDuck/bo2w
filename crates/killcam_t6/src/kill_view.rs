//! The client's killcam camera: which entity it follows, its per-type tuning and the blend
//! when the followed entity changes (CG_LerpKillCamView).

use game_math::degree_gap;

/// When the camera's entity changes, the view blends over 300 ms.
pub const BLEND_MS: i32 = 300;

/// The killcam entity types the client knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum KillcamEntityType {
    #[default]
    None = 0,
    Helicopter = 1,
    Destructible = 2,
    Scripted = 3,
    Explosive = 4,
    FastExplosive = 5,
    Rocket = 6,
    Drone = 7,
    Dog = 8,
    Artillery = 9,
    Vehicle = 10,
    Turret = 11,
}

impl KillcamEntityType {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::None,
            1 => Self::Helicopter,
            2 => Self::Destructible,
            3 => Self::Scripted,
            4 => Self::Explosive,
            5 => Self::FastExplosive,
            6 => Self::Rocket,
            7 => Self::Drone,
            8 => Self::Dog,
            9 => Self::Artillery,
            10 => Self::Vehicle,
            11 => Self::Turret,
            _ => return None,
        })
    }
}

/// The explosive camera's offsets and its stop (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplosiveCam {
    pub back: f32,
    pub up: f32,
    pub wall_out: f32,
    pub wall_side: f32,
    pub ground_up: f32,
    pub ground_back: f32,
    pub stop: f32,
    pub decel: f32,
}

/// A camera that rides above and behind its entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowCam {
    pub up: f32,
    pub back: f32,
}

/// The cg_*KillCam dvars' registered defaults. How each type places its camera from these
/// was not decoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamTuning {
    pub helicopter_fov: f32,
    pub scripted_fov: f32,
    pub destructible_fov: f32,
    pub dog_fov: f32,
    pub artillery_fov: f32,
    pub turret_fov: f32,
    pub microwave_turret_fov: f32,
    pub explosive: ExplosiveCam,
    pub rocket: FollowCam,
    pub drone: FollowCam,
}

impl Default for CamTuning {
    fn default() -> Self {
        Self {
            helicopter_fov: 15.0,
            scripted_fov: 50.0,
            destructible_fov: 50.0,
            dog_fov: 90.0,
            artillery_fov: 30.0,
            turret_fov: 65.0,
            microwave_turret_fov: 100.0,
            explosive: ExplosiveCam {
                back: 150.0,
                up: 10.0,
                wall_out: 10.0,
                wall_side: 30.0,
                ground_up: 25.0,
                ground_back: 50.0,
                stop: 200.0,
                decel: 100.0,
            },
            rocket: FollowCam { up: 100.0, back: 350.0 },
            drone: FollowCam { up: 70.0, back: 300.0 },
        }
    }
}

impl CamTuning {
    /// The fov a type sets; `None` keeps the view's own.
    #[must_use]
    pub fn fov(&self, kind: KillcamEntityType) -> Option<f32> {
        match kind {
            KillcamEntityType::Helicopter => Some(self.helicopter_fov),
            KillcamEntityType::Scripted => Some(self.scripted_fov),
            KillcamEntityType::Destructible => Some(self.destructible_fov),
            KillcamEntityType::Dog => Some(self.dog_fov),
            KillcamEntityType::Artillery => Some(self.artillery_fov),
            KillcamEntityType::Turret => Some(self.turret_fov),
            _ => None,
        }
    }
}

/// CG_LerpKillCamView's blend `elapsed_ms` after the camera's entity changed: the origin
/// straight from `from` to `to`, the angles the short way round. `trace` moves the blended
/// origin out of walls (BO2 runs a capsule trace from the start of the blend).
#[must_use]
pub fn blend_view(
    from: ([f32; 3], [f32; 3]),
    to: ([f32; 3], [f32; 3]),
    elapsed_ms: i32,
    trace: impl FnOnce([f32; 3], [f32; 3]) -> [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    if elapsed_ms >= BLEND_MS {
        return to;
    }
    let f = elapsed_ms.max(0) as f32 / BLEND_MS as f32;
    let origin: [f32; 3] = core::array::from_fn(|i| from.0[i] + (to.0[i] - from.0[i]) * f);
    let angles = core::array::from_fn(|i| from.1[i] + degree_gap(to.1[i], from.1[i]) * f);
    (trace(from.0, origin), angles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_round_trips() {
        for v in 0..12 {
            assert_eq!(KillcamEntityType::from_u8(v).map(|t| t as u8), Some(v));
        }
        assert_eq!(KillcamEntityType::from_u8(12), None);
    }

    #[test]
    fn the_blend_turns_the_short_way() {
        let from = ([0.0; 3], [0.0, 170.0, 0.0]);
        let to = ([30.0, 0.0, 0.0], [0.0, -170.0, 0.0]);
        let (o, a) = blend_view(from, to, 150, |_, p| p);
        assert!((o[0] - 15.0).abs() < 1e-4);
        assert!((a[1] - 180.0).abs() < 1e-3, "{a:?}");
        let done = blend_view(([0.0; 3], [0.0; 3]), ([1.0; 3], [2.0; 3]), BLEND_MS, |_, _| [9.0; 3]);
        assert_eq!(done, ([1.0; 3], [2.0; 3]));
    }

    #[test]
    fn helicopter_looks_narrow() {
        let t = CamTuning::default();
        assert_eq!(t.fov(KillcamEntityType::Helicopter), Some(15.0));
        assert_eq!(t.fov(KillcamEntityType::Rocket), None);
    }
}
