use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::Real;
use frame::HasWorld;
use net::ClientSet;

use render_frontend::prepare::scene::camera::SimCamera;
use render_frontend::prepare::scene::cull::{DpvsFrameStats, apply_dpvs_cull};
use render_frontend::prepare::scene::world::WorldScene;

use super::ACCEPTANCE_SAMPLE_FRAMES as SAMPLES;
use super::ACCEPTANCE_WARM_FRAMES as WARM;
use super::ledger::{FAILURE_FILE, PICTURE_FILE, write_ledger};
use super::metrics::{AcceptanceSample, MetricValue};
use super::run::AcceptanceRun;

/// Frames the exit waits after asking for the picture, so the file lands first.
const PICTURE_GRACE_FRAMES: u32 = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    WaitWorld,
    Warm,
    Sample,
    /// The ledger is written and the picture requested on the first frame in
    /// this phase; the rest are the grace period.
    Capture {
        requested: bool,
        frames_waited: u32,
    },
    Done,
    Failed,
}

#[derive(Resource, Default)]
struct AcceptanceState {
    phase: Phase,
    warm_seen: u32,
    samples: Vec<AcceptanceSample>,
    fail_reason: Option<String>,
}

impl AcceptanceState {
    fn fail(&mut self, reason: String) {
        self.fail_reason = Some(reason);
        self.phase = Phase::Failed;
    }
}

/// Only an acceptance run has the resource this needs, so nothing is added
/// to an ordinary launch.
pub(crate) fn register_acceptance_systems(app: &mut App) {
    if !app.world().contains_resource::<AcceptanceRun>() {
        return;
    }
    app.init_resource::<AcceptanceState>().add_systems(
        Update,
        (
            prepare,
            sample.after(apply_dpvs_cull),
            capture_and_exit,
        )
            .chain()
            .in_set(ClientSet::Present),
    );
}

fn prepare(
    run: Res<AcceptanceRun>,
    mut state: ResMut<AcceptanceState>,
    has_world: Res<HasWorld>,
    scene: Res<WorldScene>,
    camera: Option<ResMut<SimCamera>>,
) {
    if state.phase != Phase::WaitWorld || !has_world.0 {
        return;
    }
    if scene.intermission_view.is_none() {
        let reason = format!(
            "acceptance refused: map `{}` has no authored mp_global_intermission",
            run.map
        );
        diag::error!(World, "{reason}");
        state.fail(reason);
        return;
    }
    if let Some(mut camera) = camera {
        camera.freeze_fly = true;
        camera.enabled = false;
    }
    let _ = std::fs::create_dir_all(&run.artifact_dir);
    diag::info!(
        World,
        "acceptance: world ready on {}; warm {WARM} then sample {SAMPLES}",
        run.map
    );
    state.phase = Phase::Warm;
}

fn sample(mut state: ResMut<AcceptanceState>, stats: Res<DpvsFrameStats>, real: Res<Time<Real>>) {
    match state.phase {
        Phase::Warm => {
            // Frames before anything is drawn do not count towards the warm-up.
            if stats.unculled_triangles == 0 && stats.triangles == 0 {
                return;
            }
            state.warm_seen += 1;
            if state.warm_seen >= WARM {
                state.phase = Phase::Sample;
                state.samples.clear();
                state.samples.reserve(SAMPLES as usize);
                diag::info!(World, "acceptance: sampling {SAMPLES} frames");
            }
        }
        Phase::Sample if (state.samples.len() as u32) < SAMPLES => {
            let sample = AcceptanceSample {
                i: state.samples.len() as u32,
                wall_frame_ms: real.delta_secs_f64() * 1000.0,
                visible_cells: stats.visible_cells,
                bsp_visible_surfaces: stats.bsp_visible_surfaces,
                bsp_admitted_surfaces: stats.bsp_admitted_surfaces,
                bsp_run_n: stats.bsp_run_n,
                bsp_input_gap_n: stats.bsp_input_gap_n,
                bsp_ranges_unavailable: stats.bsp_ranges_unavailable,
                surfaces: stats.surfaces,
                triangles: stats.triangles,
                rebinds: stats.rebinds,
                submitted_batches: stats.submitted_batches,
                rewritten_index_bytes: stats.rewritten_index_bytes,
                visibility_changes: stats.visibility_changes,
                gpu_frame_ms: MetricValue::Unavailable,
                draw_calls: MetricValue::Unavailable,
                process_allocations: diag::process_allocations().process_allocations,
            };
            state.samples.push(sample);
            if state.samples.len() as u32 >= SAMPLES {
                state.phase = Phase::Capture {
                    requested: false,
                    frames_waited: 0,
                };
                diag::info!(World, "acceptance: samples complete; capturing PNG");
            }
        }
        _ => {}
    }
}

fn capture_and_exit(
    mut commands: Commands,
    run: Res<AcceptanceRun>,
    mut state: ResMut<AcceptanceState>,
    mut exit: MessageWriter<AppExit>,
) {
    match state.phase {
        Phase::Failed => {
            if let Some(reason) = &state.fail_reason {
                let _ = std::fs::write(run.artifact_dir.join(FAILURE_FILE), format!("{reason}\n"));
            }
            exit.write(AppExit::from_code(2));
            state.phase = Phase::Done;
        }
        Phase::Capture {
            requested: false, ..
        } => {
            if let Err(error) = write_ledger(&run, &state.samples) {
                diag::error!(World, "acceptance: write ledger failed: {error}");
                state.fail(error);
                return;
            }
            let picture = run.artifact_dir.join(PICTURE_FILE);
            diag::info!(World, "acceptance: screenshot {}", picture.display());
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(picture));
            state.phase = Phase::Capture {
                requested: true,
                frames_waited: 0,
            };
        }
        Phase::Capture {
            requested: true,
            frames_waited,
        } => {
            let frames_waited = frames_waited + 1;
            if frames_waited >= PICTURE_GRACE_FRAMES {
                state.phase = Phase::Done;
                exit.write(AppExit::Success);
            } else {
                state.phase = Phase::Capture {
                    requested: true,
                    frames_waited,
                };
            }
        }
        _ => {}
    }
}
