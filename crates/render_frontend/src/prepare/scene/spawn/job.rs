use std::time::Instant;

use bevy::prelude::*;
use frame::LevelGeneration;

use crate::assemble::draw_items::LiveLightbakeHandles;
use crate::prepare::scene::level_gpu::LevelGpuWait;
use crate::prepare::scene::level_pictures::LevelImageUpload;

/// Where the world's spawn stands. The job only ever moves forward through
/// these, one per slice of work, until `Done`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LevelSpawnPhase {
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
pub struct LevelSpawnJob {
    pub stage_now: LevelSpawnPhase,
    pub prior_work_ms: f32,
    pub images: LevelImageUpload,
    pub gpu_wait: LevelGpuWait,
    pub spawn: LevelGeneration,
    pub(super) previous_slice: Option<Instant>,
}

impl LevelSpawnJob {
    /// Milliseconds since the previous slice began (zero for the first one),
    /// and marks `started` as the new slice start.
    pub(crate) fn slice_gap_millis(&mut self, started: Instant) -> f32 {
        let since = self
            .previous_slice
            .map_or(Default::default(), |earlier| (started - earlier).as_secs_f32() * 1000.0);
        self.previous_slice = Some(started);
        since
    }

    pub(crate) fn stitch_image_handles(
        &self,
    ) -> (
        &[Option<Handle<Image>>],
        &[Option<LiveLightbakeHandles>],
        &[Option<Handle<Image>>],
    ) {
        self.images.stitch_handles()
    }
}
