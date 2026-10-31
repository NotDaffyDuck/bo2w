//! Taking the queued screenshots: wait until the frame shows what was asked for, start one capture at a time,
//! and hand the encode and file write to an I/O worker. On exit, wait briefly for writes still running and
//! report what never made it to disk.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use assets::LoadBackdrop;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::IoTaskPool;
use frame::{ShellScreen, LevelPresent};
use render_frontend::prepare::scene::cull::SightFrameStats;

use super::drain::{self, PENDING_SAVES};
use super::facts::FrameFacts;
use super::queue::{Finished, ScreenshotBacklog, moved_out_readback, write_image};

/// A request still held back after this long is dropped.
const WAIT_LIMIT: Duration = Duration::from_secs(180);

#[allow(clippy::too_many_arguments)]
pub(crate) fn grab_screenshot_frame(
    mut commands: Commands,
    mut queue: ResMut<ScreenshotBacklog>,
    app_screen: Option<Res<ShellScreen>>,
    loading: Option<Res<LoadBackdrop>>,
    level_is_up: Option<Res<LevelPresent>>,
    stats: Option<Res<SightFrameStats>>,
    mut quit: MessageWriter<AppExit>,
) {
    let (batches_sent, first_group_world_batches) = match stats.as_deref() {
        Some(counts) => (counts.batches_sent, counts.g0_level_facets.len() as u32),
        None => (0, 0),
    };
    let facts = FrameFacts {
        on_screen: app_screen.as_deref().copied().unwrap_or_default(),
        loading_screen_shown: loading.is_some(),
        level_is_up: level_is_up.is_some_and(|present| present.0),
        batches_sent,
        first_group_world_batches,
    };

    // A loaded level only counts as drawn once its world surfaces are being submitted.
    let world_drawn = !facts.level_is_up || facts.first_group_world_batches > 0;
    let quiet = facts.content_up() && world_drawn;
    queue.quiet_frame_count = if quiet { queue.quiet_frame_count.saturating_add(1) } else { 0 };

    queue.collect_finished();
    serve_front_request(&mut commands, &mut queue, facts);
    // A capture has four steps (waiting, readback, encode, write) and only the first two run here. The flag stays
    // up while a worker is still writing, so every frame a capture overlapped is marked and frame-time comparisons
    // can leave those frames out.
    let capturing = queue.writes_running > 0 || !queue.waiting_list.is_empty();
    perf::frames::set_state(perf::frames::flag::SCREENSHOT, capturing);

    // The frame that sees a write finish never exits; the exit check waits one more frame.
    if std::mem::take(&mut queue.file_write_done) {
        return;
    }
    if queue.should_exit() {
        quit.write(AppExit::Success);
    }
}

/// Ages the oldest request by a frame, then either holds it (dropping it once it has waited too long) or starts
/// its capture.
fn serve_front_request(commands: &mut Commands, queue: &mut ScreenshotBacklog, facts: FrameFacts) {
    let settled = queue.quiet_frame_count;
    let (hold, waited, expired, file) = match queue.waiting_list.front_mut() {
        None => return,
        Some(oldest) => {
            oldest.frames_spent_waiting = oldest.frames_spent_waiting.saturating_add(1);
            let hold = facts.hold(settled);
            let newly_held = hold.is_some_and(|reason| oldest.is_new_hold(reason));
            let expired = oldest.queued_at.elapsed() >= WAIT_LIMIT;
            (hold.map(|reason| (reason, newly_held)), oldest.frames_spent_waiting, expired, oldest.request.path.clone())
        }
    };
    match hold {
        Some((reason, _)) if expired => {
            queue.waiting_list.pop_front();
            diag::error!(Launch, "screenshot: dropped {} after waiting {waited} frames ({})", file.display(), reason.label());
        }
        Some((reason, newly_held)) => {
            if newly_held {
                diag::info!(Launch, "screenshot: waiting on {}: {}", file.display(), reason.label());
            }
        }
        None => {
            queue.waiting_list.pop_front();
            diag::info!(
                Launch,
                "screenshot: saving {} (screen {:?}, batches sent {}, first-group world batches {}, steady {settled} frames, in queue {waited} frames)",
                file.display(),
                facts.on_screen,
                facts.batches_sent,
                facts.first_group_world_batches,
            );
            start_capture(commands, queue, file);
        }
    }
}

/// Requests a screenshot of the window for `file`. When the pixels arrive, an I/O worker encodes and writes them
/// and reports the result through the backlog's finished list.
fn start_capture(commands: &mut Commands, queue: &mut ScreenshotBacklog, file: PathBuf) {
    render_anim::motion::fpv::vm_log_note(&format!("SHOT {}", file.display()));
    queue.writes_running += 1;
    drain::arm_at_exit();
    let finished = queue.finished.clone();
    let on_pixels = move |mut captured: On<ScreenshotCaptured>| {
        // Only moving the pixels out of the event costs this frame anything.
        perf::frames::mark(perf::frames::flag::SCREENSHOT);
        let image = std::mem::replace(&mut captured.event_mut().image, moved_out_readback());
        // The 72-byte black PNGs: say what the readback was.
        if image.width() <= 1 || image.height() <= 1 {
            warn!(
                "screenshot: readback for {} is {}x{} (window surface minimised or zero-size?)",
                file.display(),
                image.width(),
                image.height()
            );
        }
        let (file, finished) = (file.clone(), finished.clone());
        PENDING_SAVES.fetch_add(1, Ordering::Relaxed);
        let write = async move {
            let error = write_image(&file, image).err();
            finished.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(Finished { path: file, error });
            PENDING_SAVES.fetch_sub(1, Ordering::Relaxed);
        };
        IoTaskPool::get().spawn(write).detach();
    };
    commands.spawn(Screenshot::primary_window()).observe(on_pixels);
}

/// On the way out: wait up to the drain budget for writes still running, then report any that did not finish and
/// any request never started. The same wait is also armed at process exit, for exits that send no `AppExit`;
/// this is where the report is made.
pub(crate) fn complain_uncaptured_outputs(mut queue: ResMut<ScreenshotBacklog>, mut quitting: MessageReader<AppExit>) {
    let leaving = quitting.read().last().is_some();
    if !leaving {
        return;
    }
    let budget = drain::budget();
    let deadline = Instant::now() + budget;
    while queue.writes_running > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
        queue.collect_finished();
    }
    let unfinished = queue.writes_running;
    if unfinished > 0 {
        let waited_ms = budget.as_millis();
        diag::error!(
            Launch,
            "screenshot: {unfinished} write(s) still running after {waited_ms}ms; the files they were encoding are incomplete"
        );
    }
    for never_written in queue.queued_file_paths() {
        diag::error!(Launch, "screenshot: {} never got written; the process quit while it was still queued", never_written.display());
    }
}
