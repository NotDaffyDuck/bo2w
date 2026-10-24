use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use assets::LoadingScreen;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::IoTaskPool;
use frame::{AppScreen, HasWorld};
use render_frontend::prepare::scene::cull::DpvsFrameStats;

use super::drain::{self, WRITES_IN_FLIGHT};
use super::facts::FrameFacts;
use super::queue::{CaptureQueue, Finished, moved_out_readback, write_image};

/// A request that waits this long is given up on.
const WAIT_LIMIT: Duration = Duration::from_secs(180);

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_frame(
    mut commands: Commands,
    mut queue: ResMut<CaptureQueue>,
    screen: Option<Res<AppScreen>>,
    loading: Option<Res<LoadingScreen>>,
    has_world: Option<Res<HasWorld>>,
    stats: Option<Res<DpvsFrameStats>>,
    mut exit: MessageWriter<AppExit>,
) {
    let facts = FrameFacts {
        screen: screen.map(|s| *s).unwrap_or_default(),
        loading_overlay: loading.is_some(),
        has_world: has_world.is_some_and(|w| w.0),
        submitted_batches: stats.as_ref().map_or(0, |s| s.submitted_batches),
        g0_world: stats.as_ref().map_or(0, |s| s.g0_world_surfs.len() as u32),
    };

    // bo2mp: a world counts as drawn once the BO2 pass has world surfaces.
    let pipelines_ready = !facts.has_world || facts.g0_world > 0;
    queue.settled_frames = if facts.content_up() && pipelines_ready {
        queue.settled_frames.saturating_add(1)
    } else {
        0
    };

    queue.collect_finished();
    take_ready(&mut commands, &mut queue, facts);
    // A capture is four things (the request waiting, the readback, the encode,
    // the file write) and only the first two are on this thread. The flag
    // stays up while a worker is still writing, so the frames a capture was in
    // flight over are marked and a frame-performance pair can leave them out.
    let busy = !queue.pending.is_empty() || queue.writing > 0;
    perf::frames::set_state(perf::frames::flag::SCREENSHOT, busy);

    if queue.write_completed {
        queue.write_completed = false;
    } else if queue.should_exit() {
        exit.write(AppExit::Success);
    }
}

fn take_ready(commands: &mut Commands, queue: &mut CaptureQueue, facts: FrameFacts) {
    let settled = queue.settled_frames;
    let Some(front) = queue.pending.front_mut() else {
        return;
    };
    front.waited_frames = front.waited_frames.saturating_add(1);
    let waited = front.waited_frames;

    if let Some(hold) = facts.hold(settled) {
        if front.queued_at.elapsed() >= WAIT_LIMIT {
            if let Some(failed) = queue.pending.pop_front() {
                diag::error!(
                    Launch,
                    "screenshot: gave up on {} after {} frames — {}",
                    failed.request.path.display(),
                    waited,
                    hold.label()
                );
            }
        } else if front.is_new_hold(hold) {
            diag::info!(
                Launch,
                "screenshot: holding {} — {}",
                front.request.path.display(),
                hold.label()
            );
        }
        return;
    }

    let Some(ready) = queue.pending.pop_front() else {
        return;
    };
    let path = ready.request.path;
    diag::info!(
        Launch,
        "screenshot: writing {} (screen {:?}, world batches {}, g0_world {}, settled {} frames, \
         queued {} frames ago)",
        path.display(),
        facts.screen,
        facts.submitted_batches,
        facts.g0_world,
        settled,
        waited
    );
    render_anim::anim::fpv::vm_log_note(&format!("SHOT {}", path.display()));
    queue.writing += 1;
    drain::arm_at_exit();
    let finished = queue.finished.clone();
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |mut captured: On<ScreenshotCaptured>| {
            // The readback landed. This frame pays only for moving the pixels
            // out of the event; encoding and writing belong to an I/O worker,
            // which reports back through `finished`.
            perf::frames::mark(perf::frames::flag::SCREENSHOT);
            let image = std::mem::replace(&mut captured.event_mut().image, moved_out_readback());
            let path = path.clone();
            let finished = finished.clone();
            WRITES_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
            IoTaskPool::get()
                .spawn(async move {
                    let error = write_image(&path, image).err();
                    let mut list = finished.lock().unwrap_or_else(|p| p.into_inner());
                    list.push(Finished { path, error });
                    drop(list);
                    WRITES_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
                })
                .detach();
        });
}

/// Finishes what is still in flight before the process leaves, and says what
/// could not be finished. The same wait is armed on `atexit` for exits that
/// leave without an `AppExit` anyone can read; this one is where it is said.
pub(crate) fn report_unwritten_captures(
    mut queue: ResMut<CaptureQueue>,
    mut exit: MessageReader<AppExit>,
) {
    if exit.read().next().is_none() {
        return;
    }
    let budget = drain::budget();
    let deadline = Instant::now() + budget;
    while queue.writing > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
        queue.collect_finished();
    }
    if queue.writing > 0 {
        diag::error!(
            Launch,
            "screenshot: {} write(s) had not finished after {}ms — the files they were encoding are incomplete",
            queue.writing,
            budget.as_millis(),
        );
    }
    for path in queue.pending_paths() {
        diag::error!(
            Launch,
            "screenshot: {} was never written — process exiting with the request queued",
            path.display()
        );
    }
}
