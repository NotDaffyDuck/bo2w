//! A map load as a thing of its own, separate from any screen drawn over it.
//!
//! Each request gets one process and its own [`LoadCompletion`]. Surfaces read
//! it; one going away, or never being there, changes nothing about what the
//! load waits for or reports. A task still finishing the previous map writes
//! into a progress record nobody reads any more, not into the new map's rows.

use std::time::Instant;

use bevy::prelude::Resource;

use asset_transport::progress::{LoadCompletion, StageHandle, StageId};

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
    pub load_req_id: u64,
    pub decrypt_secret: frame::OwnLevelLoadTicket,
    pub zone: String,
    pub progress: LoadCompletion,
    requested_at: Instant,
    wait: Wait,
}

impl MapLoadProcess {
    pub fn new(
        load_req_id: u64,
        decrypt_secret: frame::OwnLevelLoadTicket,
        zone: impl Into<String>,
        progress: LoadCompletion,
    ) -> Self {
        Self {
            load_req_id,
            decrypt_secret,
            zone: zone.into(),
            requested_at: progress.requested_at(),
            progress,
            wait: Wait::Preparing,
        }
    }

    pub fn requested_at(&self) -> Instant {
        self.requested_at
    }

    pub fn is_done(&self) -> bool {
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
        let millis = |span: std::time::Duration| span.as_secs_f32() * 1000.0;
        let installed_ms = millis(self.requested_at.elapsed());
        let trim_ms = millis(diag::return_spare_heap());
        let trimmed_ms = millis(self.requested_at.elapsed());
        // Memory figures in whole MiB, left out where the platform has none.
        let mib = |label: &str, bytes: Option<u64>| bytes.map_or_else(String::new, |b| format!(" {label}={}", b >> 20));
        let rss = mib("rss_mib", asset_transport::working_set_size());
        let heap = mib("heap_mib", diag::heap_bytes_in_use());
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
