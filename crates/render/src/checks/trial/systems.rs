//! The acceptance run's frame loop: wait for the level, let rendering settle, record a fixed number of frames,
//! write the ledger and a screenshot, then quit with a status the harness reads.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::Real;
use frame::LevelPresent;
use net::ClientStep;

use render_frontend::prepare::scene::camera::GameplayCamera;
use render_frontend::prepare::scene::cull::{SightFrameStats, apply_sight_hide};
use render_frontend::prepare::scene::world::LevelStage;

use super::TRIAL_MEASURED_FRAME_COUNT as SAMPLES;
use super::TRIAL_WARMUP_FRAME_COUNT as WARM;
use super::ledger::{FAILURE_FILE, PICTURE_FILE, write_ledger};
use super::metrics::{AcceptanceReading, MeasuredReading};
use super::run::AcceptanceTrial;

/// Frames to keep running after the screenshot is requested, so the file is on disk before the app quits.
const PICTURE_GRACE_FRAMES: u32 = 30;

/// Exit status of a run that was refused or could not write its results.
const REFUSED_EXIT_CODE: u8 = 2;

/// Where the run stands. It only ever moves forward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    /// Waiting for the level to be up.
    #[default]
    Loading,
    /// Drawing frames that are not recorded.
    Settling,
    /// Recording one reading per frame.
    Recording,
    /// Recording is over: write the ledger and request the screenshot.
    Shooting,
    /// Screenshot requested; this many frames have passed since.
    Lingering(u32),
    /// Refused or failed; the reason is written out and the app told to quit.
    Refused,
    /// The exit has been sent.
    Finished,
}

#[derive(Resource, Default)]
struct AcceptanceProgress {
    phase: Stage,
    warm_frames_seen: u32,
    readings: Vec<AcceptanceReading>,
    refusal: Option<String>,
}

impl AcceptanceProgress {
    fn give_up(&mut self, why: String) {
        self.refusal = Some(why);
        self.phase = Stage::Refused;
    }
}

/// Adds the acceptance systems, but only when the run's settings resource exists, so an ordinary launch gets
/// none of them.
pub(crate) fn register_acceptance_systems(app: &mut App) {
    if !app.world().contains_resource::<AcceptanceTrial>() {
        return;
    }
    let systems = (prepare, sample.after(apply_sight_hide), capture_and_exit).chain().in_set(ClientStep::Present);
    app.init_resource::<AcceptanceProgress>().add_systems(Update, systems);
}

/// Once the level is up: refuse a map without a scoreboard camera spot; otherwise park the gameplay camera on
/// it and start settling.
fn prepare(
    run: Res<AcceptanceTrial>,
    mut progress: ResMut<AcceptanceProgress>,
    level_is_up: Res<LevelPresent>,
    scene: Res<LevelStage>,
    camera: Option<ResMut<GameplayCamera>>,
) {
    if progress.phase != Stage::Loading || !level_is_up.0 {
        return;
    }
    if scene.scoreboard_eye.is_none() {
        let reason = format!("acceptance refused: map `{}` has no authored mp_global_intermission", run.map);
        diag::error!(World, "acceptance run refused: {reason}");
        progress.give_up(reason);
        return;
    }
    if let Some(mut cam_ctl) = camera {
        cam_ctl.enabled = false;
        cam_ctl.hold_free_camera = true;
    }
    let _ = std::fs::create_dir_all(&run.output_folder);
    diag::info!(World, "acceptance: level up on {}; {WARM} settling frames, then {SAMPLES} recorded", run.map);
    progress.phase = Stage::Settling;
}

/// One ledger row from this frame's culling counters.
fn reading(index: u32, wall_ms: f64, stats: &SightFrameStats) -> AcceptanceReading {
    let &SightFrameStats {
        cells_seen: visible_cells,
        bsp_shown: bsp_visible_surfaces,
        bsp_accepted: bsp_admitted_surfaces,
        bsp_runs: bsp_run_n,
        bsp_input_gaps: bsp_input_gap_n,
        bsp_unranged: bsp_ranges_unavailable,
        surfaces,
        triangles,
        rebinds,
        batches_sent,
        index_bytes_rewritten: rewritten_index_bytes,
        vis_flips: visibility_changes,
        ..
    } = stats;
    AcceptanceReading {
        i: index,
        frame_wall_ms: wall_ms,
        cells_seen: visible_cells,
        bsp_surfaces_seen: bsp_visible_surfaces,
        bsp_surfaces_let_in: bsp_admitted_surfaces,
        bsp_runs: bsp_run_n,
        bsp_gaps: bsp_input_gap_n,
        bsp_no_ranges: bsp_ranges_unavailable,
        surfaces_drawn: surfaces,
        triangles_drawn: triangles,
        rebind_count: rebinds,
        batches_sent,
        index_bytes_rewritten: rewritten_index_bytes,
        vis_flips: visibility_changes,
        gpu_frame_ms: MeasuredReading::Missing,
        draw_calls: MeasuredReading::Missing,
        alloc_count: diag::process_allocations().allocation_count,
    }
}

/// Counts settling frames (only ones where something was drawn), then records `SAMPLES` readings.
fn sample(mut progress: ResMut<AcceptanceProgress>, stats: Res<SightFrameStats>, real: Res<Time<Real>>) {
    let progress = &mut *progress;
    match progress.phase {
        Stage::Settling => {
            let drew_something = stats.triangles != 0 || stats.unhidden_triangles != 0;
            if !drew_something {
                return;
            }
            progress.warm_frames_seen += 1;
            if progress.warm_frames_seen < WARM {
                return;
            }
            progress.readings = Vec::with_capacity(SAMPLES as usize);
            progress.phase = Stage::Recording;
            diag::info!(World, "acceptance: recording {SAMPLES} frames");
        }
        Stage::Recording => {
            let taken = progress.readings.len() as u32;
            if taken >= SAMPLES {
                return;
            }
            progress.readings.push(reading(taken, real.delta_secs_f64() * 1000.0, &stats));
            if taken + 1 >= SAMPLES {
                progress.phase = Stage::Shooting;
                diag::info!(World, "acceptance: recording done; taking the PNG");
            }
        }
        _ => {}
    }
}

/// Writes the results and quits: the refusal reason and status 2 for a refused run; otherwise the ledger, then a
/// screenshot, then success once the grace frames are over.
fn capture_and_exit(
    mut commands: Commands,
    run: Res<AcceptanceTrial>,
    mut progress: ResMut<AcceptanceProgress>,
    mut quit: MessageWriter<AppExit>,
) {
    match progress.phase {
        Stage::Refused => {
            if let Some(why) = progress.refusal.as_deref() {
                let _ = std::fs::write(run.output_folder.join(FAILURE_FILE), format!("{why}\n"));
            }
            progress.phase = Stage::Finished;
            quit.write(AppExit::from_code(REFUSED_EXIT_CODE));
        }
        Stage::Shooting => match write_ledger(&run, &progress.readings) {
            Err(problem) => {
                diag::error!(World, "acceptance: ledger not written: {problem}");
                progress.give_up(problem);
            }
            Ok(()) => {
                let picture = run.output_folder.join(PICTURE_FILE);
                diag::info!(World, "acceptance: saving the screenshot to {}", picture.display());
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(picture));
                progress.phase = Stage::Lingering(0);
            }
        },
        Stage::Lingering(waited) if waited + 1 >= PICTURE_GRACE_FRAMES => {
            progress.phase = Stage::Finished;
            quit.write(AppExit::Success);
        }
        Stage::Lingering(waited) => progress.phase = Stage::Lingering(waited + 1),
        _ => {}
    }
}
