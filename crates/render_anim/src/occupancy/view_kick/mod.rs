//! The local player's view: the gun's kick, the camera's bob, the hit flinch and the
//! landing dip, and the system that places the world camera from them.
//!
//! - `state` holds everything carried from one frame to the next.
//! - `tick` steps it once per frame, before the camera is placed.
//! - `first_person` and `camera` place the camera.
//! - `feedback`, `params`, `breath` and `placement` are the smaller pieces they use.

mod breath;
mod camera;
mod death_cam;
mod feedback;
mod first_person;
mod flags;
mod params;
mod placement;
mod state;
mod tick;

#[cfg(test)]
mod pins;
#[cfg(test)]
mod tests;

pub use self::breath::{
    BREATH_GASP_LERP, BREATH_GASP_SCALE, BREATH_GASP_TIME_MS, BREATH_HOLD_LERP, BREATH_HOLD_TIME_MS, HoldBreath,
};
pub use self::camera::sync_camera_from_presented;
pub use self::death_cam::{DeathCamWatch, log_death_cam_jumps};
pub(crate) use self::feedback::iron_gun_turn;
pub use self::placement::GunOffset;
pub(crate) use self::placement::{
    apply_cg_gun_offset_view, apply_viewweapon_land_view, iw_view_placement_to_bevy_camera_local,
};
pub(crate) use self::state::bo2_camera;
pub use self::state::{PendingViewHurt, SessionViewKick};
pub use self::tick::{reset_view_kick_on_life_started, tick_session_view_kick};
