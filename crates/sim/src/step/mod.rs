//! One simulation frame. The schedule below runs the whole frame as a chain of systems that
//! each read the frame's request from the world, and the last one leaves the snapshot in it.
//!
//! - `actions`: what clients ask for between commands (joining, classes, debug moves).
//! - `classes`, `give`, `weapon_config`: loadouts, weapons handed out and configuration changes.
//! - `players`: every player's commands run through the movement and the weapon.
//! - `clip`, `feel`: what the movement collides with and how a BO2 player handles.
//! - `phases`: the entity phases that follow the movement.
//! - `hazard`: falling and drowning.
//! - `close`: bookkeeping at the end of the frame and publishing the snapshot.

mod actions;
mod classes;
mod clip;
mod close;
mod feel;
mod give;
mod hazard;
mod phases;
mod players;
mod weapon_config;

#[cfg(test)]
mod tests;

use std::time::Instant;

use bevy_ecs::prelude::{Resource, World};
use bevy_ecs::schedule::{IntoScheduleConfigs, Schedule};

use crate::frame::FrameWorld;
use crate::gentity::KernelPhase;
use crate::identities::MatchPhase;
use crate::input::TickInput;
use crate::match_state::ClientLifecycle;
use crate::snapshot::Snapshot;
use crate::world::{ClientId, Tick};

pub use clip::{VehicleClip, bo2_map_collision};
pub(crate) use clip::{OtherVehicle, PlayerModelTouches};
pub use feel::set_omni_movement;
pub(crate) use feel::omni_movement_setting;
pub(crate) use give::{arm_held_weapon, seed_ps_ammo_tables};
pub use phases::phase_materialize_entity_dobjs;

pub(crate) use classes::log_forced_spawn;

/// What the frame in progress was asked to do, and where it leaves its result.
#[derive(Resource)]
pub(crate) struct StepRequest {
    pub(crate) tick: Tick,
    pub(crate) input: TickInput,
    msec: i32,
    pub(crate) reason: crate::StepReason,
    output: Option<Snapshot>,
}

/// A part of the frame that takes longer than this (in milliseconds) names itself in the log,
/// profiling or not, so that a freeze says where it happened.
const STALL_MS: f64 = 250.0;

/// Times one part of the frame and logs it when it ran long.
pub(crate) struct StallGuard {
    began: Instant,
    part: &'static str,
    cpu_at_start: Option<f64>,
}

impl StallGuard {
    pub(crate) fn new(part: &'static str) -> Self {
        Self {
            began: Instant::now(),
            part,
            cpu_at_start: diag::thread_cpu_ms(),
        }
    }
}

impl Drop for StallGuard {
    fn drop(&mut self) {
        let ms = self.began.elapsed().as_secs_f64() * 1000.0;
        if ms > STALL_MS {
            diag::warn!(
                Sim,
                "bo2mp stall: {} took {ms:.0} ms{}",
                self.part,
                diag::busy_note(self.cpu_at_start)
            );
        }
    }
}

/// The systems of one frame, in the order they run. Everything after the script runtime's
/// bookkeeping only runs while the script runtime is healthy.
pub(crate) fn schedule() -> Schedule {
    let after_scripts = (
        apply_script_signals_system,
        apply_actions_system,
        crate::script::sync_players,
        crate::script::sync_presence,
        players::run_players_system,
        phases::record_collision_state_system,
        phases::run_entity_types_system,
        phases::dispatch_touches_system,
        crate::t6::collect_item_pickups,
        crate::script::sync_engine_events,
        close::finalize_system,
        close::publish_snapshot_system,
    )
        .chain()
        .run_if(crate::script::healthy);

    let mut frame = Schedule::default();
    frame.add_systems(
        (
            advance_time_system,
            expire_transient_events_system,
            crate::script::apply_disconnects,
            crate::script::advance_mechanics,
            crate::script::advance_scheduler,
            crate::t6::advance,
            after_scripts,
        )
            .chain(),
    );
    frame
}

/// Runs one frame and returns the snapshot it published, or the fault that stopped the scripts.
pub(crate) fn run_schedule(
    ecs: &mut World,
    schedule: &mut Schedule,
    tick: Tick,
    input: &TickInput,
    msec: i32,
    reason: crate::StepReason,
) -> Result<Snapshot, crate::script::Fault> {
    crate::script::preflight(ecs, tick, reason)?;
    assert!(
        !ecs.contains_resource::<StepRequest>(),
        "simulation schedule already has a pending frame"
    );
    ecs.insert_resource(StepRequest {
        tick,
        input: input.clone(),
        msec: msec.clamp(1, 200),
        reason,
        output: None,
    });
    schedule.run(ecs);
    let request = ecs
        .remove_resource::<StepRequest>()
        .expect("simulation request missing");
    if let Some(fault) = ecs.resource::<crate::script::Runtime>().fault.as_ref() {
        return Err(fault.clone());
    }
    Ok(request
        .output
        .expect("simulation schedule did not publish a snapshot"))
}

fn frame_world(world: &mut World) -> FrameWorld<'_> {
    FrameWorld::from_world(world)
}

/// Whether the client is in the match and alive.
fn is_alive(world: &FrameWorld, id: ClientId) -> bool {
    world
        .client_meta(id)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive)
}

fn advance_time_system(world: &mut World) {
    let tick = {
        let mut request = world.resource_mut::<StepRequest>();
        request.input.canonicalize();
        request.tick
    };
    let mut frame = frame_world(world);
    frame.mark_running();
    frame.begin_entity_frame(tick);
}

fn expire_transient_events_system(world: &mut World) {
    let tick = world.resource::<StepRequest>().tick;
    let mut frame = frame_world(world);
    frame.enter_kernel_phase(KernelPhase::ExpireTransientEvents);
    frame.expire_dying_missiles(tick);
    frame.clear_tick_events(tick);
}

/// Acts on what the scripts signalled last frame: ending the level, restarting it, or a
/// notification the match start waits for.
fn apply_script_signals_system(world: &mut World) {
    let signals = crate::script::take_signals(world);
    if signals.is_empty() {
        return;
    }
    let mut frame = frame_world(world);
    for signal in signals {
        let name: &str = &signal;
        if name == crate::script::EXIT_LEVEL {
            let finished = matches!(frame.phase(), MatchPhase::Intermission | MatchPhase::PostGame);
            if !finished {
                frame.set_phase(MatchPhase::Intermission);
            }
        } else if name == crate::script::MAP_RESTART {
            let tick = frame.ecs().resource::<StepRequest>().tick;
            crate::script::restart_level(frame.ecs(), tick);
        } else {
            crate::script::host::match_start::apply_level_notify(&mut frame, name);
        }
    }
}

/// Applies the requests clients made this frame, after making sure each one has the row it needs.
fn apply_actions_system(world: &mut World) {
    let (tick, actions) = {
        let request = world.resource::<StepRequest>();
        (request.tick, request.input.actions.clone())
    };
    let mut frame = frame_world(world);
    for (id, action) in &actions {
        if actions::needs_player_row(action) {
            let _ = frame.ensure_player(*id);
        } else {
            let _ = frame.client_meta_mut(*id);
        }
    }
    frame.enter_kernel_phase(KernelPhase::ApplyActions);
    actions::apply_actions(&mut frame, tick, &actions);
}
