//! Bringing a freshly installed world on screen: the job that walks the spawn
//! phases, the systems that advance it, what it publishes for the draw side,
//! and how it lets go again when the match is torn down.

mod drive;
mod job;
mod look;
mod shown_frame;
mod level_teardown;

pub(crate) use drive::{spawn_level_world, spawn_level_finish};
pub use job::{LevelSpawnJob, LevelSpawnPhase};
pub(crate) use shown_frame::register_level_gpu_ready;
pub(crate) use level_teardown::{
    arm_level_spawn_on_install, despawn_free_cameras_on_teardown,
    despawn_level_entities_on_teardown, reset_level_spawn_on_teardown,
    shutdown_level_on_teardown,
};
