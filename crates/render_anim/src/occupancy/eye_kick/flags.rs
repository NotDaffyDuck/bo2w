//! The two debug switches of the view-kick code, each read from the environment once.

use std::sync::OnceLock;

/// `BO2MP_SHOT_LOG` set (to anything): log every shot's kick and every landing's dip.
pub(super) fn shot_log_on() -> bool {
    static SWITCH: OnceLock<bool> = OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

/// `BO2MP_DEATHCAMLOG=1`: log what the camera is made from around a death.
pub(super) fn death_cam_log_on() -> bool {
    static SWITCH: OnceLock<bool> = OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var("BO2MP_DEATHCAMLOG").is_ok_and(|v| v == "1"))
}
