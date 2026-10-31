//! Timing of the render world, reported as perf counters.
//!
//! - `stages`: checkpoints inside the render world, collected on a shared board
//! - `gpu`: GPU pass timings from Bevy's diagnostics store
//! - `report`: the main-world sampler that turns both into counters, and the optional
//!   `BO2MP_RENDER_LOG` line

mod gpu;
mod report;
mod stages;

pub use report::sample_frame_diag;
pub use stages::register_render_frame_diag;
