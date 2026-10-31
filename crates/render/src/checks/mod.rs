//! Run-time diagnostics: screenshot capture, the acceptance run, frame
//! timing spans and the GPU allocator report.

pub mod trial;
pub mod shots;
pub mod frame_timing;
pub mod gpu_report;
pub use render_gpu::gpu_diag::frame_cost as frame_report;

/// Old names of `trial` and `shots`, kept until the rename sweep.
pub use trial as acceptance;
pub use shots as capture;
