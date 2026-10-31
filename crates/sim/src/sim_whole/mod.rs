//! `WholeSim`: the whole simulation as one value the outside world holds.
//!
//! Inside, the match lives in a bevy `World` with one *state entity* carrying the [`MatchSimulation`] and a
//! row per player, shell in the air, script mover and gun on the floor beside it. `WholeSim` derefs to that
//! state for plain reads and writes, and offers the operations that must reach into the rows or
//! the script runtime. The operations are grouped by what they touch:
//!
//! * [`scripting`] — GSC programs, the Black Ops II script install, bots, killcam seats
//! * [`entities`] — script movers and players
//! * [`checkpoint`] — snapshots, adopting them, shutdown, collision-history queries

mod checkpoint;
mod entities;
mod scripting;
#[cfg(test)]
mod tests;

use core::fmt;
use core::ops::{Deref, DerefMut};

use bevy_ecs::prelude::{Entity, World};
use bevy_ecs::schedule::Schedule;

use crate::step_view::StepWorldView;
use crate::step_view::RowDirectory;
use crate::step_view::{gather_loot, gather_avatars};
use crate::step_view::{gather_shells, gather_movers};
use crate::step_view::{seat_state_holder, seed_rows};
use crate::tick_feed::StepInputs;
use crate::tick_publish::TickSnapshot;
use crate::sim_parts::{MatchSimulation, StepNo};

/// The whole simulation: one bevy world, the entity holding the match state, and the per-tick schedule.
pub struct WholeSim {
    bevy: World,
    tick_plan: Schedule,
    state_row: Entity,
}

/// Older spelling SimWorld still calls; the rename sweep moves it over.
pub use self::WholeSim as SimWorld;

/// A bevy world holding only `state` on its state entity, ready for rows and script resources.
fn world_around(state: MatchSimulation) -> (World, Entity) {
    let mut bevy = World::new();
    let row = bevy.spawn(state).id();
    bevy.entity_mut(row).insert(RowDirectory::default());
    seat_state_holder(&mut bevy, row);
    (bevy, row)
}

impl Default for WholeSim {
    fn default() -> Self {
        let (mut bevy, state_row) = world_around(MatchSimulation::default());
        bevy.insert_resource(crate::gsc_host::Runtime::default());
        bevy.insert_resource(crate::gsc_host::Mechanics::default());
        bevy.insert_resource(crate::gsc_host::NativeRegistry::default());
        Self { bevy, state_row, tick_plan: crate::frame_step::build_schedule() }
    }
}

impl Clone for WholeSim {
    /// A full copy: the state, every entity row and the script runtime. The two worlds share
    /// nothing afterwards.
    fn clone(&self) -> Self {
        let (mut bevy, state_row) = world_around(MatchSimulation::clone(self));
        let players = gather_avatars(&self.bevy);
        let shells = gather_shells(&self.bevy);
        let movers = gather_movers(&self.bevy);
        let items = gather_loot(&self.bevy);
        seed_rows(&mut bevy, players, shells, movers, items);
        crate::gsc_host::copy_state(&self.bevy, &mut bevy);
        Self { bevy, state_row, tick_plan: crate::frame_step::build_schedule() }
    }
}

impl fmt::Debug for WholeSim {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("SimWorld").field("state", &**self).finish()
    }
}

const STATE_MISSING: &str = "simulation state entity is missing its SimState component";

impl Deref for WholeSim {
    type Target = MatchSimulation;

    fn deref(&self) -> &MatchSimulation {
        self.bevy.get::<MatchSimulation>(self.state_row).expect(STATE_MISSING)
    }
}

impl DerefMut for WholeSim {
    fn deref_mut(&mut self) -> &mut MatchSimulation {
        self.bevy.get_mut::<MatchSimulation>(self.state_row).expect(STATE_MISSING).into_inner()
    }
}

impl WholeSim {
    pub fn new() -> Self {
        Self::default()
    }

    /// The working view that systems and helpers in this crate use to reach the rows.
    pub(crate) fn frame(&mut self) -> StepWorldView<'_> {
        StepWorldView::new(&mut self.bevy, self.state_row)
    }

    fn run(
        &mut self,
        tick: StepNo,
        input: &StepInputs,
        msec: i32,
        why: AdvanceCause,
    ) -> Result<TickSnapshot, crate::gsc_host::Fault> {
        crate::frame_step::run_frame(&mut self.bevy, &mut self.tick_plan, tick, input, msec, why)
    }
}

/// Why a tick is being run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdvanceCause {
    /// The server's own frame: the one that moves the shared world forward.
    #[default]
    ServerStepFrame,
    /// A client running a tick it has not run before, ahead of the server.
    GuessAhead,
    /// A client re-running a tick it already ran, after the server corrected it.
    Replay,
}

impl AdvanceCause {
    /// Only the server's own frame changes the authoritative world; client-side ticks are guesses.
    pub const fn moves_server_world_forward(self) -> bool {
        !matches!(self, Self::GuessAhead | Self::Replay)
    }
}

/// Runs one tick and returns what it produced. A script fault is a bug in the loaded scripts and
/// stops the process.
pub fn step(
    sim: &mut WholeSim,
    tick: StepNo,
    input: &StepInputs,
    msec: i32,
    why: AdvanceCause,
) -> TickSnapshot {
    try_step(sim, tick, input, msec, why)
        .unwrap_or_else(|fault| panic!("GSC execution failed: {fault}"))
}

/// As [`step`], handing a script fault back instead of stopping.
pub fn try_step(
    sim: &mut WholeSim,
    tick: StepNo,
    input: &StepInputs,
    msec: i32,
    why: AdvanceCause,
) -> Result<TickSnapshot, crate::gsc_host::Fault> {
    sim.run(tick, input, msec, why)
}
