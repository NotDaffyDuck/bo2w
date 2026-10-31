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
mod solids;
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

use crate::step_view::StepWorldView;
use crate::ent_slots::EntityPassStage;
use crate::match_ids::GameRoundStage;
use crate::tick_feed::StepInputs;
use crate::player_ledger::PlayerSessionStage;
use crate::tick_publish::TickSnapshot;
use crate::sim_parts::{SeatNo, StepNo};

pub use solids::{VehicleClip, bo2_map_collision};
pub(crate) use solids::{OtherVehicle, PlayerModelTouches};
pub use feel::set_omni_movement;
pub(crate) use feel::{BODY_HALF_WIDTH, STANDING_FOOTPRINT_HIGH, STANDING_FOOTPRINT_LOW};
pub(crate) use feel::omni_movement_setting;
pub(crate) use give::{arm_held_weapon, seed_ps_ammo_tables};
pub use phases::stage_build_entity_models;

pub(crate) use classes::log_forced_spawn;

/// What the frame in progress was asked to do, and where it leaves its result.
#[derive(Resource)]
pub(crate) struct FrameTicket {
    pub(crate) tick: StepNo,
    pub(crate) input: StepInputs,
    msec: i32,
    pub(crate) reason: crate::AdvanceCause,
    published: Option<TickSnapshot>,
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
pub(crate) fn build_schedule() -> Schedule {
    let after_scripts = (
        apply_script_signals_system,
        run_client_actions_system,
        crate::gsc_host::sync_players,
        crate::gsc_host::sync_presence,
        players::run_players_system,
        phases::record_collision_state_system,
        phases::run_entity_types_system,
        phases::dispatch_touches_system,
        crate::zombie_rt::collect_item_pickups,
        crate::gsc_host::sync_engine_events,
        close::finalize_system,
        close::publish_snapshot_system,
    )
        .chain()
        .run_if(crate::gsc_host::healthy);

    let mut frame = Schedule::default();
    frame.add_systems(
        (
            begin_frame_system,
            expire_events_system,
            crate::gsc_host::apply_disconnects,
            crate::gsc_host::advance_mechanics,
            crate::gsc_host::advance_scheduler,
            crate::zombie_rt::advance,
            after_scripts,
        )
            .chain(),
    );
    frame
}

/// Runs one frame and returns the snapshot it published, or the fault that stopped the scripts.
pub(crate) fn run_frame(
    app: &mut World,
    frame_schedule: &mut Schedule,
    tick: StepNo,
    input: &StepInputs,
    msec: i32,
    why: crate::AdvanceCause,
) -> Result<TickSnapshot, crate::gsc_host::Fault> {
    crate::gsc_host::preflight(app, tick, why)?;
    if app.contains_resource::<FrameTicket>() {
        panic!("simulation schedule already has a pending frame");
    }
    let msec = msec.min(200).max(1);
    app.insert_resource(FrameTicket {
        tick,
        input: StepInputs::clone(input),
        msec,
        reason: why,
        published: None,
    });
    frame_schedule.run(app);
    let finished = app
        .remove_resource::<FrameTicket>()
        .expect("simulation request missing");
    if let Some(fault) = app.resource::<crate::gsc_host::Runtime>().fault.as_ref() {
        return Err(fault.clone());
    }
    let snapshot = finished
        .published
        .expect("the frame plan ended without handing back a tick picture");
    Ok(snapshot)
}

fn step_view_of(ecs: &mut World) -> StepWorldView<'_> {
    StepWorldView::from_world(ecs)
}

/// Whether the client is in the match and alive.
fn is_alive(world: &StepWorldView, seat: SeatNo) -> bool {
    world
        .client_meta(seat)
        .is_some_and(|meta| matches!(meta.lifecycle, PlayerSessionStage::Alive))
}

fn begin_frame_system(world: &mut World) {
    let tick = {
        let mut pending = world.resource_mut::<FrameTicket>();
        pending.input.put_in_order();
        pending.tick
    };
    let mut fw = step_view_of(world);
    fw.mark_running();
    fw.begin_entity_frame(tick);
}

/// The tick of the frame in progress.
fn frame_tick(world: &World) -> StepNo {
    let pending = world.resource::<FrameTicket>();
    pending.tick
}

fn expire_events_system(world: &mut World) {
    let tick = frame_tick(world);
    let mut fw = step_view_of(world);
    fw.begin_pass_stage(EntityPassStage::DropOldEvents);
    fw.expire_dying_missiles(tick);
    fw.discard_events_for_tick(tick);
}

/// Acts on what the scripts signalled last frame: ending the level, restarting it, or a
/// notification the match start waits for.
fn apply_script_signals_system(world: &mut World) {
    let signals = crate::gsc_host::take_signals(world);
    if signals.is_empty() {
        return;
    }
    let mut fw = step_view_of(world);
    for signal in signals {
        let name: &str = &signal;
        if name == crate::gsc_host::EXIT_LEVEL {
            let already_over = matches!(fw.phase(), GameRoundStage::Intermission | GameRoundStage::PostGame);
            if !already_over {
                fw.set_phase(GameRoundStage::Intermission);
            }
        } else if name != crate::gsc_host::MAP_RESTART {
            crate::gsc_host::apply_level_notify(&mut fw, name);
        }
    }
}

/// Applies the requests clients made this frame, after making sure each one has the row it needs.
fn run_client_actions_system(world: &mut World) {
    let (tick, moves) = {
        let pending = world.resource::<FrameTicket>();
        (pending.tick, pending.input.actions.clone())
    };
    let mut fw = step_view_of(world);
    for (who, action) in &moves {
        if actions::needs_player_row(action) {
            let _ = fw.seat_slot_for(*who);
        } else {
            let _ = fw.client_meta_mut(*who);
        }
    }
    fw.begin_pass_stage(EntityPassStage::TakeInput);
    actions::apply_actions(&mut fw, tick, &moves);
}
