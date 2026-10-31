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
mod eye_place;
mod state;
mod tick;

mod pins;
mod tests;

pub use self::breath::{
    BREATH_GASP_LERP, BREATH_GASP_SCALE, BREATH_GASP_TIME_MS, BREATH_HOLD_LERP, BREATH_HOLD_TIME_MS, HoldBreath,
};
pub use self::camera::sync_camera_from_shown;
pub use self::death_cam::{DeathCamWatch, log_death_cam_jumps};
pub(crate) use self::feedback::iron_gun_turn;
pub use self::eye_place::GunOffset;
pub(crate) use self::eye_place::{
    sum3, placement_to_camera_local,
};
pub(crate) use self::state::bo2_camera;
pub use self::state::{QueuedHurtFlinch, CameraRecoilState};
pub use self::tick::{clear_recoil_at_respawn, advance_session_view_kick};
