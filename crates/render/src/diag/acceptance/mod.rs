//! The render acceptance run: warm a map up, sample 600 frames, photograph it,
//! write the numbers beside the picture and quit.

mod ledger;
mod metrics;
mod run;
mod systems;

use bevy::window::PresentMode;

pub use run::AcceptanceRun;
pub(crate) use systems::register_acceptance_systems;

pub const ACCEPTANCE_WIDTH: u32 = 1280;
pub const ACCEPTANCE_HEIGHT: u32 = 720;
pub const ACCEPTANCE_WARM_FRAMES: u32 = 120;
pub const ACCEPTANCE_SAMPLE_FRAMES: u32 = 600;
pub const ACCEPTANCE_PRESENT_MODE: PresentMode = PresentMode::AutoNoVsync;
pub const ACCEPTANCE_MAPS: &[&str] = &["mp_boneyard", "mp_favela", "mp_rust", "mp_highrise"];
pub const ACCEPTANCE_ENV: &str = "BO2MP_RENDER_ACCEPTANCE";
pub const ACCEPTANCE_FLAG: &str = "--render-acceptance";
