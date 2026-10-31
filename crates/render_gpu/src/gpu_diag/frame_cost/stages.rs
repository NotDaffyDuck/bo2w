//! Checkpoints taken inside Bevy's render world, one render frame at a time.
//!
//! The render world runs apart from the main world, so nothing it measures can be read
//! where it happens. Every checkpoint writes into a shared board instead; the board
//! publishes the finished frame when the render schedule closes, and the main world
//! picks it up from there (see `report`).
//!
//! A render frame is one chain of checkpoints in a fixed order: the `Render` schedule's
//! phases, then the render graph (record, submit, finish), then the present mark. Each
//! checkpoint reads the time since the one before it, so a single stopwatch serves all
//! of them.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::app::SubApp;
use bevy::ecs::schedule::MainThreadExecutor;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::pipelined_rendering::{RenderAppChannels, RenderExtractApp};
use bevy::render::renderer::{PendingCommandBuffers, RenderGraph, RenderGraphSystems};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::tasks::ComputeTaskPool;

/// The `Render` schedule phases that get a duration, in the order they run.
#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    ExtractCommands,
    PrepareAssets,
    PrepareMeshes,
    CreateViews,
    Specialize,
    PrepareViews,
    Queue,
    PhaseSort,
    Prepare,
}

impl Phase {
    pub const ALL: [Phase; 9] = [
        Phase::ExtractCommands,
        Phase::PrepareAssets,
        Phase::PrepareMeshes,
        Phase::CreateViews,
        Phase::Specialize,
        Phase::PrepareViews,
        Phase::Queue,
        Phase::PhaseSort,
        Phase::Prepare,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::ExtractCommands => "extract_commands",
            Phase::PrepareAssets => "prepare_assets",
            Phase::PrepareMeshes => "prepare_meshes",
            Phase::CreateViews => "create_views",
            Phase::Specialize => "specialize",
            Phase::PrepareViews => "prepare_views",
            Phase::Queue => "queue",
            Phase::PhaseSort => "phase_sort",
            Phase::Prepare => "prepare",
        }
    }

    fn set(self) -> RenderSystems {
        match self {
            Phase::ExtractCommands => RenderSystems::ExtractCommands,
            Phase::PrepareAssets => RenderSystems::PrepareAssets,
            Phase::PrepareMeshes => RenderSystems::PrepareMeshes,
            Phase::CreateViews => RenderSystems::CreateViews,
            Phase::Specialize => RenderSystems::Specialize,
            Phase::PrepareViews => RenderSystems::PrepareViews,
            Phase::Queue => RenderSystems::Queue,
            Phase::PhaseSort => RenderSystems::PhaseSort,
            Phase::Prepare => RenderSystems::Prepare,
        }
    }
}

/// Everything measured for one render frame.
#[derive(Clone, Default)]
pub(super) struct FrameStamps {
    /// Which render frame this is. The main world looks at the published frame every
    /// tick, whether or not a new one arrived; the serial keeps one frame's numbers from
    /// being reported twice.
    pub serial: u64,
    /// The main-world frame row that was open when this render frame's extract began,
    /// so the timings land on the frame whose wall the work ran inside rather than on
    /// whichever row is open when they arrive.
    pub main_frame: u64,
    /// When extract began: the moment the state being drawn was copied out of the main
    /// world, which stood still for the copy.
    pub extracted_at: Option<Instant>,
    pub extract_ms: Option<f32>,
    pub phase_ms: [Option<f32>; Phase::ALL.len()],
    /// Recording the render graph.
    pub render_graph_ms: Option<f32>,
    /// From the graph handing over its command buffers to the end of Bevy's submit.
    pub submit_graph_ms: Option<f32>,
    /// How many command buffers that submit was handed.
    pub graph_submit_pending: Option<u32>,
    /// From the last graph set to the end of the `Render` set, as close to the present
    /// as our systems get.
    pub present_graph_ms: Option<f32>,
    /// How old the drawn state was when the graph finished with it. It stops where the
    /// graph does: Bevy presents after the graph schedule returns, and the compositor
    /// and display are outside the process, so this is not input-to-photon.
    pub state_age_ms: Option<f32>,
    /// How many main frames were opened between extract and the end of the graph. A
    /// serialised renderer answers zero, a pipelined one answers one or more.
    pub frames_behind: Option<u32>,
}

impl FrameStamps {
    pub fn phase(&self, phase: Phase) -> Option<f32> {
        self.phase_ms[phase as usize]
    }
}

#[derive(Default)]
struct Board {
    frame: FrameStamps,
    /// When the previous checkpoint was crossed.
    last_checkpoint: Option<Instant>,
    /// The most recent frame that ran to the end.
    done: Option<FrameStamps>,
    /// Serial of the frame last handed to the main world.
    handed_out: u64,
}

impl Board {
    fn begin(&mut self) {
        self.frame = FrameStamps {
            serial: self.frame.serial + 1,
            main_frame: perf::frames::open_index(),
            extracted_at: Some(Instant::now()),
            ..FrameStamps::default()
        };
    }

    /// Crosses a checkpoint: the milliseconds since the previous one, or `None` for the
    /// very first.
    fn lap(&mut self) -> Option<f32> {
        let now = Instant::now();
        let before = self.last_checkpoint.replace(now)?;
        Some(now.duration_since(before).as_secs_f32() * 1000.0)
    }
}

/// The board, shared between the main world and the render world.
#[derive(Resource, Clone, Default)]
pub struct RenderStageBoard(Arc<Mutex<Board>>);

impl RenderStageBoard {
    fn with(&self, edit: impl FnOnce(&mut Board)) {
        if let Ok(mut board) = self.0.lock() {
            edit(&mut board);
        }
    }

    /// The newest finished frame, once; `None` while nothing new has finished.
    pub(super) fn take_fresh(&self) -> Option<FrameStamps> {
        let mut board = self.0.lock().ok()?;
        let serial = board.done.as_ref()?.serial;
        if serial == board.handed_out {
            return None;
        }
        board.handed_out = serial;
        board.done.clone()
    }
}

/// Crossed between phase `index - 1` and phase `index`. Index 0 is the start of the
/// frame and index `Phase::ALL.len()` is the end of the last phase.
fn phase_checkpoint(index: usize) -> impl FnMut(Res<RenderStageBoard>) {
    move |board| {
        if index == 0 {
            perf::Span::DrawThreadMs.begin();
        }
        if index == Phase::ALL.len() {
            perf::Span::DrawRenderMs.begin();
        }
        board.with(|b| {
            let ms = b.lap();
            if let Some(finished) = index.checked_sub(1) {
                b.frame.phase_ms[finished] = ms;
            }
        });
    }
}

fn render_set_closed() {
    perf::Span::DrawRenderMs.end();
}

fn present_mark_closed(board: Res<RenderStageBoard>) {
    board.with(|b| b.frame.present_graph_ms = b.lap());
}

fn frame_closed(board: Res<RenderStageBoard>) {
    perf::Span::DrawThreadMs.end();
    board.with(|b| b.done = Some(b.frame.clone()));
}

/// Start of the graph schedule: everything before it belongs to the phases.
fn graph_opened(board: Res<RenderStageBoard>) {
    board.with(|b| {
        b.lap();
    });
}

/// Where the graph's render systems have all flushed and nothing has been submitted yet.
///
/// The interval this opens covers `submit_pending_command_buffers` (which finishes every
/// pending encoder and then calls `Queue::submit`) and `handle_uncovered_swap_chains`.
/// Both are Bevy's exclusive systems and cannot be timed from outside; the count says
/// whether a long interval had many encoders to finish or one submit that blocked.
fn graph_recorded(board: Res<RenderStageBoard>, pending: Res<PendingCommandBuffers>) {
    let queued = pending.len() as u32;
    board.with(|b| {
        b.frame.render_graph_ms = b.lap();
        b.frame.graph_submit_pending = Some(queued);
    });
}

fn graph_submitted(board: Res<RenderStageBoard>) {
    board.with(|b| b.frame.submit_graph_ms = b.lap());
}

/// The last set of the graph: the latest point in the schedule that still belongs to
/// this image, since Bevy presents from `render_system` after the whole graph returns.
fn graph_finished(board: Res<RenderStageBoard>) {
    let now = Instant::now();
    let open = perf::frames::open_index();
    board.with(|b| {
        b.lap();
        let opened_for = b.frame.main_frame;
        b.frame.frames_behind = Some(u32::try_from(open.saturating_sub(opened_for)).unwrap_or(u32::MAX));
        if let Some(extracted) = b.frame.extracted_at {
            b.frame.state_age_ms = Some(now.duration_since(extracted).as_secs_f32() * 1000.0);
        }
    });
}

/// Wraps the render app's extract so each render frame starts a fresh set of stamps and
/// the extract itself is timed.
fn time_extract(sub_app: &mut SubApp, board: RenderStageBoard) {
    let mut default_extract = sub_app.take_extract();
    sub_app.set_extract(move |main_w, render_w| {
        let _body = perf::Span::RenderExtractBodyMs.enter();
        board.with(Board::begin);
        let t_start = Instant::now();
        if let Some(run) = default_extract.as_mut() {
            run(main_w, render_w);
        }
        let took = 1000.0 * t_start.elapsed().as_secs_f32();
        board.with(|b| b.frame.extract_ms = Some(took));
    });
}

fn schedule_render_checkpoints(render_sub: &mut SubApp) {
    for index in 0..=Phase::ALL.len() {
        let upcoming = Phase::ALL
            .get(index)
            .map_or(RenderSystems::Render, |phase| phase.set());
        let mut checkpoint = phase_checkpoint(index).before(upcoming);
        if let Some(finished) = index.checked_sub(1) {
            checkpoint = checkpoint.after(Phase::ALL[finished].set());
        }
        render_sub.add_systems(Render, checkpoint);
    }
    // The end of `Render`, the closest mark to the present, then the end of the frame.
    let closing = (
        render_set_closed.before(RenderSystems::Cleanup).after(RenderSystems::Render),
        present_mark_closed.before(RenderSystems::Cleanup).after(render_set_closed),
        frame_closed.after(RenderSystems::PostCleanup),
    );
    render_sub.add_systems(Render, closing);
}

fn schedule_graph_checkpoints(render_sub: &mut SubApp) {
    // One stamp between each pair of graph sets, the last inside `Finish`.
    let stamps = (
        graph_opened.before(RenderGraphSystems::Render).after(RenderGraphSystems::Begin),
        graph_recorded.before(RenderGraphSystems::Submit).after(RenderGraphSystems::Render),
        graph_submitted.before(RenderGraphSystems::Finish).after(RenderGraphSystems::Submit),
        graph_finished.in_set(RenderGraphSystems::Finish),
    );
    render_sub.add_systems(RenderGraph, stamps);
}

/// Times the hand-off between the main world and the pipelined render thread.
///
/// This mirrors Bevy 0.19's `renderer_extract`; keep its executor servicing and its
/// shutdown behaviour when upgrading Bevy.
fn time_pipelined_handoff(host: &mut App) {
    let Some(handoff) = host.get_sub_app_mut(RenderExtractApp) else {
        return;
    };
    handoff.set_extract(|main_world, _| {
        main_world.resource_scope(|world, executor: Mut<MainThreadExecutor>| {
            world.resource_scope(|world, mut channels: Mut<RenderAppChannels>| {
                let received = {
                    let _wait = perf::Span::RenderReceiveWorldMs.enter();
                    let pool = ComputeTaskPool::get();
                    let mut got = pool.scope_with_executor(true, Some(&*executor.0), |scope| {
                        scope.spawn(async { channels.recv().await });
                    });
                    got.pop().unwrap()
                };
                match received {
                    Some(mut render_sub) => {
                        render_sub.extract(world);
                        let _send = perf::Span::RenderDispatchWorldMs.enter();
                        channels.send_blocking(render_sub);
                    }
                    None => {
                        world.write_message(AppExit::error());
                    }
                }
            });
        });
    });
}

pub fn register_render_frame_diag(bevy_app: &mut App) {
    let host = bevy_app;
    host.add_plugins(RenderDiagnosticsPlugin);

    let board = RenderStageBoard::default();
    host.insert_resource(board.clone());

    time_pipelined_handoff(host);

    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        diag::warn!(World, "render frame diag: no RenderApp, so the extract, queue and prepare columns are left NULL");
        return;
    };
    render_sub.insert_resource(board.clone());
    time_extract(render_sub, board);
    schedule_render_checkpoints(render_sub);
    schedule_graph_checkpoints(render_sub);
}
