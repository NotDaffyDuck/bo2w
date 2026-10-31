//! Per-frame measurements of the client's schedule: allocations in each phase, and how the time
//! in Present and Ui divides among the systems that run there.

mod allocs;
mod stage_split;

pub use allocs::PlayerStageTally;
pub use stage_split::{OVERLAY_STAGE_TOTAL, FrameUpdateTally};

use bevy::prelude::*;

/// Install both censuses on the client's `Update` schedule.
pub fn enroll_update_stage_tally(app: &mut App) {
    allocs::add_to_app(app);
    stage_split::add_to_app(app);
}
