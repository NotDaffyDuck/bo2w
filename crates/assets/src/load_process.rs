//! A map load as a thing of its own, separate from any screen drawn over it.
//!
//! Each request gets one process and its own [`LoadProgress`]. Surfaces read
//! it; one going away, or never being there, changes nothing about what the
//! load waits for or reports. A task still finishing the previous map writes
//! into a progress record nobody reads any more, not into the new map's rows.

use std::time::Instant;

use bevy::prelude::Resource;

use asset_transport::progress::{LoadProgress, StageHandle, StageId};

/// Where the load is in the hand-off to the session.
enum Wait {
    /// Still preparing locally, or not yet waiting on the session.
    Preparing,
    /// Prepared; the session has not let this client in. The open stage lets
    /// the progress table name what is holding the load.
    Admission(StageHandle),
    /// Over, whichever way it went.
    Over,
}

#[derive(Resource)]
pub struct MapLoadProcess {
    pub request_id: u64,
    pub load_key: frame::LocalLoadKey,
    pub zone: String,
    pub progress: LoadProgress,
    requested_at: Instant,
    wait: Wait,
}

impl MapLoadProcess {
    pub fn new(
        request_id: u64,
        load_key: frame::LocalLoadKey,
        zone: impl Into<String>,
        progress: LoadProgress,
    ) -> Self {
        Self {
            request_id,
            load_key,
            zone: zone.into(),
            requested_at: progress.requested_at(),
            progress,
            wait: Wait::Preparing,
        }
    }

    pub fn requested_at(&self) -> Instant {
        self.requested_at
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.wait, Wait::Over)
    }

    /// Local preparation is finished and the session has not admitted this
    /// client. The wait gets a stage row of its own rather than showing up as
    /// a gap between two others.
    pub fn await_admission(&mut self) {
        if matches!(self.wait, Wait::Preparing) {
            self.wait = Wait::Admission(self.progress.begin(StageId::Admission, None));
        }
    }

    /// The client is admitted and the world installed. Giving freed memory
    /// back to the system counts as part of the load.
    pub fn finish(&mut self) {
        match std::mem::replace(&mut self.wait, Wait::Over) {
            Wait::Over => return,
            Wait::Admission(stage) => stage.done(),
            Wait::Preparing => self.progress.record_skipped(StageId::Admission),
        }
        let installed_ms = self.requested_at.elapsed().as_secs_f32() * 1000.0;
        let trim_ms = diag::release_freed_heap().as_secs_f32() * 1000.0;
        let trimmed_ms = self.requested_at.elapsed().as_secs_f32() * 1000.0;
        let rss = asset_transport::process_resident_bytes()
            .map(|bytes| format!(" rss_mib={}", bytes >> 20))
            .unwrap_or_default();
        let heap = diag::process_live_heap_bytes()
            .map(|bytes| format!(" heap_mib={}", bytes >> 20))
            .unwrap_or_default();
        diag::info!(
            World,
            "load complete: {installed_ms:.0}ms from overlay to installed world ({trimmed_ms:.0}ms with trim) for `{}`{rss}{heap} trim={trim_ms:.1}ms",
            self.zone
        );
    }

    /// The session turned this client away. Whatever the load recorded stays
    /// readable; only the open wait is marked failed.
    pub fn fail(&mut self) {
        if let Wait::Admission(stage) = std::mem::replace(&mut self.wait, Wait::Over) {
            stage.fail();
        }
    }
}
