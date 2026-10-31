//! The render acceptance run: warm a map up, sample 600 frames, photograph it,
//! write the numbers beside the picture and quit.

mod ledger;
mod metrics;
mod run;
mod systems;

use bevy::window::PresentMode;

pub use run::AcceptanceTrial;
pub(crate) use systems::register_acceptance_systems;

pub const TRIAL_RESOLUTION_COLUMNS: u32 = 1280;
pub const TRIAL_RESOLUTION_ROWS: u32 = 720;
pub const TRIAL_WARMUP_FRAME_COUNT: u32 = 120;
pub const TRIAL_MEASURED_FRAME_COUNT: u32 = 600;
pub const SMOKE_CHECK_SWAP_POLICY: PresentMode = PresentMode::AutoNoVsync;
pub const TRIAL_MAPS: &[&str] = &["mp_boneyard", "mp_favela", "mp_rust", "mp_highrise"];
pub const RENDER_CHECK_ENV: &str = "BO2MP_RENDER_ACCEPTANCE";
pub const SMOKE_CHECK_CLI_SWITCH: &str = "--render-acceptance";
