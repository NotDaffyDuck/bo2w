use std::collections::VecDeque;
use std::mem::Discriminant;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::facts::Hold;
use super::request::ScreenshotOrder;

/// A request waiting for the picture to be fit to take.
#[derive(Clone, Debug)]
pub(super) struct Pending {
    pub request: ScreenshotOrder,
    pub frames_spent_waiting: u32,
    pub queued_at: Instant,
    last_reported: Option<Discriminant<Hold>>,
}

impl Pending {
    /// True the first time a given kind of hold is seen, so the log says it
    /// once rather than every frame.
    pub fn is_new_hold(&mut self, hold: Hold) -> bool {
        let kind = std::mem::discriminant(&hold);
        let changed = self.last_reported != Some(kind);
        self.last_reported = Some(kind);
        changed
    }
}

/// How one write ended, as the worker that did it saw it.
pub(super) struct Finished {
    pub path: PathBuf,
    pub error: Option<String>,
}

/// Completions come back through a shared list rather than a channel: it is
/// a handful a frame and the queue takes whatever is there.
pub(super) type FinishedList = Arc<Mutex<Vec<Finished>>>;

#[derive(Resource, Default)]
pub struct ScreenshotBacklog {
    pub(super) waiting_list: VecDeque<Pending>,
    pub(super) quiet_frame_count: u32,
    /// Readbacks handed to a worker and not yet reported done. Lowered when
    /// the worker says it finished, not when it was handed over: the file does
    /// not exist until then, and the exit is waiting on the file.
    pub(super) writes_running: usize,
    pub(super) file_write_done: bool,
    finish_once_emptied_flag: bool,
    pub(super) finished: FinishedList,
}

impl ScreenshotBacklog {
    pub fn enqueue(&mut self, order: ScreenshotOrder) {
        self.finish_once_emptied_flag |= order.quit_when_shot_taken;
        self.waiting_list.push_back(Pending {
            request: order,
            frames_spent_waiting: 0,
            queued_at: Instant::now(),
            last_reported: None,
        });
    }

    pub fn waiting(&self) -> usize {
        self.waiting_list.len()
    }

    /// Old name of `enqueue`, kept until the rename sweep.
    pub fn push(&mut self, request: ScreenshotOrder) {
        self.enqueue(request);
    }

    /// Old name of `waiting`, kept until the rename sweep.
    pub fn pending(&self) -> usize {
        self.waiting()
    }

    pub fn queued_file_paths(&self) -> Vec<PathBuf> {
        let requests = self.waiting_list.iter().map(|p| &p.request);
        requests.map(|request| request.path.clone()).collect()
    }

    /// Asks for an exit once everything queued is on disk. False when there
    /// is nothing left to wait for.
    pub fn finish_once_emptied(&mut self) -> bool {
        let owed = !self.waiting_list.is_empty() || self.writes_running > 0;
        self.finish_once_emptied_flag |= owed;
        owed
    }

    /// (requests still waiting, writes still running)
    pub fn owed_at_exit(&self) -> (usize, usize) {
        (self.waiting_list.len(), self.writes_running)
    }

    pub(super) fn drained(&self) -> bool {
        self.waiting_list.is_empty() && self.writes_running == 0
    }

    pub(super) fn should_exit(&self) -> bool {
        self.finish_once_emptied_flag && self.drained()
    }

    /// Books every write a worker finished since the last look.
    pub(super) fn collect_finished(&mut self) {
        let done = {
            let mut list = self.finished.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *list)
        };
        for write in done {
            self.writes_running = self.writes_running.saturating_sub(1);
            self.file_write_done = true;
            match write.error {
                Some(error) => diag::error!(
                    Launch,
                    "screenshot: {} was not written: {error}",
                    write.path.display()
                ),
                None => diag::info!(Launch, "screenshot: wrote {}", write.path.display()),
            }
        }
    }
}

/// Encodes and writes one readback. Runs on an I/O worker, never on the frame.
///
/// With HDR on, the alpha channel carries brightness, so it is dropped rather
/// than saved as opacity.
pub(super) fn write_image(path: &Path, image: Image) -> Result<(), String> {
    let image = image
        .try_into_dynamic()
        .map_err(|error| format!("readback is not a saveable format: {error}"))?;
    image
        .to_rgb8()
        .save(path)
        .map_err(|failure| failure.to_string())
}

/// What is left in the event after its pixels were moved out: one texel and no
/// allocation, since copying tens of megabytes to get them off the frame
/// would defeat the point of moving them.
pub(super) fn moved_out_readback() -> Image {
    let one_texel = Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    Image::new_uninit(
        one_texel,
        TextureDimension::D2,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::empty(),
    )
}
