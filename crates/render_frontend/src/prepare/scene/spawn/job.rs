use std::time::Instant;

use bevy::prelude::*;
use frame::WorldGeneration;

use crate::assemble::drawsurf::RuntimeLightmapHandles;
use crate::prepare::scene::{world_gpu::WorldGpuWait, world_images::WorldImageUpload};

/// Where the world's spawn stands. The job only ever moves forward through
/// these, one per slice of work, until `Done`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorldSpawnPhase {
    /// Nothing installed yet.
    #[default]
    Unarmed,
    /// One frame given back so the loading overlay can draw.
    Yield,
    /// Material generation and the map's look get published.
    Materials,
    /// Material images are handed to `Assets<Image>` a slice at a time.
    Images,
    /// The draw plans are installed.
    Plan,
    /// The world tessellation is built and placed.
    WorldTess,
    /// Waiting for the GPU to take the images and pipelines.
    Gpu,
    Done,
}

/// The state of one world spawn. It is replaced whole when a match installs.
#[derive(Resource, Default)]
pub struct WorldSpawnJob {
    pub phase: WorldSpawnPhase,
    pub last_work_ms: f32,
    pub images: WorldImageUpload,
    pub gpu_wait: WorldGpuWait,
    pub spawn: WorldGeneration,
    pub(super) previous_slice: Option<Instant>,
}

impl WorldSpawnJob {
    /// Milliseconds since the previous slice began (zero for the first one),
    /// and marks `now` as the new slice start.
    pub(crate) fn slice_gap_ms(&mut self, now: Instant) -> f32 {
        let since = self
            .previous_slice
            .map_or(0.0, |earlier| (now - earlier).as_secs_f32() * 1000.0);
        self.previous_slice = Some(now);
        since
    }

    pub(crate) fn tess_image_handles(
        &self,
    ) -> (
        &[Option<Handle<Image>>],
        &[Option<RuntimeLightmapHandles>],
        &[Option<Handle<Image>>],
    ) {
        self.images.tess_handles()
    }
}
