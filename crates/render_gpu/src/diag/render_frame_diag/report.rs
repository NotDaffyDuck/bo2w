//! The main-world side: turns the board's finished frames and the GPU batches into perf
//! counters, and writes the optional render log line.

use std::fmt::Write as _;
use std::time::Instant;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;

use super::gpu::{self, GpuBatch};
use super::stages::{FrameStamps, Phase, RenderStageBoard};

/// With `BO2MP_RENDER_LOG` set, one line is logged every this many main frames. The
/// write happens on the main thread and a slow one stalled whole frames for seconds, so
/// it is off unless asked for.
const LOG_EVERY: u32 = 64;

struct LogGate {
    enabled: bool,
    ticks: u32,
}

impl Default for LogGate {
    fn default() -> Self {
        Self {
            enabled: std::env::var_os("BO2MP_RENDER_LOG").is_some(),
            ticks: 0,
        }
    }
}

impl LogGate {
    fn due(&mut self) -> bool {
        let tick = self.ticks;
        self.ticks = tick.wrapping_add(1);
        self.enabled && tick % LOG_EVERY == 0
    }
}

/// What the sampler remembers between ticks.
#[derive(Default)]
pub struct ReportState {
    last_batch: Option<Instant>,
    last_frame: Option<FrameStamps>,
    log: LogGate,
}

fn emit_frame_counters(frame: &FrameStamps) {
    let at = frame.main_frame;
    let durations = [
        (perf::Counter::RenderGraphRenderMs, frame.graph_render_ms),
        (
            perf::Counter::RenderGraphSubmitIntervalMs,
            frame.graph_submit_ms,
        ),
        (perf::Counter::RenderGraphPresentMs, frame.graph_present_ms),
        (
            perf::Counter::RenderPresentedStateAgeMs,
            frame.state_age_ms,
        ),
        // The `PrepareViews` set as it ran: a set interval, not a body. Whatever the
        // executor scheduled inside the set is in it, so it cannot be added to anything
        // and is not exclusive CPU work.
        (
            perf::Counter::RenderPrepareViewsMs,
            frame.phase(Phase::PrepareViews),
        ),
    ];
    for (counter, ms) in durations {
        if let Some(ms) = ms {
            counter.emit_at(f64::from(ms), at);
        }
    }
    if let Some(queued) = frame.graph_submit_pending {
        perf::Counter::RenderGraphSubmitPendingN.emit_at(f64::from(queued), at);
    }
    if let Some(behind) = frame.frames_behind {
        perf::Counter::RenderPresentedFramesBehind.emit_at(f64::from(behind), at);
    }
}

fn emit_gpu_counters(batch: &GpuBatch) {
    if let Some(ms) = batch.total_ms {
        perf::Counter::RenderGpuFrameMs.emit(f64::from(ms));
    }
    for (counter, ms) in batch.passes.counters() {
        if let Some(ms) = ms {
            counter.emit(f64::from(ms));
        }
    }
}

fn fmt_ms(ms: Option<f32>) -> String {
    ms.map_or_else(|| "NULL".into(), |ms| format!("{ms:.3}"))
}

fn describe_stages(frame: Option<&FrameStamps>) -> String {
    let Some(frame) = frame else {
        return "no render frame finished yet".into();
    };
    let mut line = format!("extract_ms={}", fmt_ms(frame.extract_ms));
    for phase in Phase::ALL {
        let _ = write!(line, " {}_ms={}", phase.name(), fmt_ms(frame.phase(phase)));
    }
    line
}

/// Visible mesh entities and the indices they would draw.
fn visible_geometry(
    meshes: &Assets<Mesh>,
    visible: &Query<(&Mesh3d, &ViewVisibility)>,
) -> (u32, u64) {
    let mut entities = 0u32;
    let mut indices = 0u64;
    for (mesh3d, view_visibility) in visible {
        if !view_visibility.get() {
            continue;
        }
        entities = entities.saturating_add(1);
        let count = meshes
            .get(&mesh3d.0)
            .and_then(|mesh| mesh.indices())
            .map_or(0, |i| i.len() as u64);
        indices = indices.saturating_add(count);
    }
    (entities, indices)
}

/// Reports the render world's newest finished frame and the newest GPU batch, each
/// exactly once.
///
/// A render frame's counters belong to the main frame that was open when its extract
/// began, which is not the frame open when the numbers arrive here.
pub fn sample_render_frame_diag(
    board: Res<RenderStageBoard>,
    store: Option<Res<DiagnosticsStore>>,
    meshes: Res<Assets<Mesh>>,
    visible_meshes: Query<(&Mesh3d, &ViewVisibility)>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut state: Local<ReportState>,
) {
    if let Some(frame) = board.take_fresh() {
        emit_frame_counters(&frame);
        state.last_frame = Some(frame);
    }

    let batch = store.as_deref().map(gpu::latest_batch);
    if let Some(batch) = &batch
        && batch.delivered != state.last_batch
    {
        state.last_batch = batch.delivered;
        emit_gpu_counters(batch);
    }

    if !state.log.due() {
        return;
    }
    let (entities, indices) = visible_geometry(&meshes, &visible_meshes);
    let present_mode = windows
        .iter()
        .next()
        .map_or_else(|| "NULL".to_owned(), |w| format!("{:?}", w.present_mode));
    let gpu_frame = batch
        .as_ref()
        .and_then(|b| b.total_ms)
        .or_else(|| store.as_deref().and_then(gpu::world_pass_ms));
    let gpu_opaque = store.as_deref().and_then(gpu::opaque_pass_ms);
    diag::info!(
        World,
        "render frame diag: {} present_mode={} gpu_frame_ms={} gpu_opaque_ms={} drawn_entities={} drawn_index_count={}",
        describe_stages(state.last_frame.as_ref()),
        present_mode,
        fmt_ms(gpu_frame),
        fmt_ms(gpu_opaque),
        entities,
        indices
    );
}
