//! Bringing a freshly installed world on screen: the job that walks the spawn
//! phases, the systems that advance it, what it publishes for the draw side,
//! and how it lets go again when the match is torn down.

mod drive;
mod job;
mod look;
mod present;
mod teardown;

pub(crate) use drive::{spawn_world, spawn_world_finish};
pub use job::{WorldSpawnJob, WorldSpawnPhase};
pub(crate) use present::register_world_gpu_ready;
pub(crate) use teardown::{
    arm_world_spawn_on_install, despawn_fly_cameras_on_teardown,
    despawn_world_entities_on_teardown, reset_world_spawn_on_teardown,
    shutdown_world_on_teardown,
};
