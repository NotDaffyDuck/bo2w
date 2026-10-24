pub const KILLCAM_DEFAULT_LERP_MS: i32 = 300;

pub const KILLCAM_TURRET_LERP_MS: i32 = 900;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum KillCamMode {
    Mode0 = 0,
    Mode1Heli = 1,
    Mode2Airstrike = 2,
    Mode3Missile = 3,
    Mode4MissileAlt = 4,
    Mode5Rocket = 5,
    Mode6Turret = 6,
    Mode7Javelin = 7,
    Mode8Remote = 8,
}

impl KillCamMode {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

pub fn killcam_lerp_deadline_ms(
    on_enter_frame: bool,
    focus_changed: bool,
    mode: KillCamMode,
    now_ms: i32,
) -> i32 {
    if on_enter_frame || !focus_changed {
        return 0;
    }
    let span = if mode == KillCamMode::Mode6Turret {
        KILLCAM_TURRET_LERP_MS
    } else {
        KILLCAM_DEFAULT_LERP_MS
    };
    now_ms.saturating_add(span)
}

#[inline]
pub fn third_person_in_killcam(in_killcam: bool, mode: KillCamMode) -> bool {
    in_killcam && mode != KillCamMode::Mode0
}
