//! `SimWorld`: the whole simulation as one value the outside world holds.
//!
//! Inside, the match lives in a bevy `World` with one *state entity* carrying the [`SimState`] and a
//! row per player, projectile, script mover and dropped item beside it. `SimWorld` derefs to that
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

use crate::frame::{
    FrameWorld, PayloadIndex, collect_dropped_items, collect_players, collect_projectiles,
    collect_script_movers, install_state_entity, spawn_payload_rows,
};
use crate::input::TickInput;
use crate::snapshot::Snapshot;
use crate::world::{SimState, Tick};

pub struct SimWorld {
    ecs: World,
    state_entity: Entity,
    schedule: Schedule,
}

/// A bevy world holding only `state` on its state entity, ready for rows and script resources.
fn world_around(state: SimState) -> (World, Entity) {
    let mut ecs = World::new();
    let state_entity = ecs.spawn(state).id();
    ecs.entity_mut(state_entity).insert(PayloadIndex::default());
    install_state_entity(&mut ecs, state_entity);
    (ecs, state_entity)
}

impl Default for SimWorld {
    fn default() -> Self {
        let (mut ecs, state_entity) = world_around(SimState::default());
        ecs.insert_resource(crate::script::Runtime::default());
        ecs.insert_resource(crate::script::Mechanics::default());
        ecs.insert_resource(crate::script::NativeRegistry::default());
        Self { ecs, state_entity, schedule: crate::step::schedule() }
    }
}

impl Clone for SimWorld {
    /// A full copy: the state, every entity row and the script runtime. The two worlds share
    /// nothing afterwards.
    fn clone(&self) -> Self {
        let (mut ecs, state_entity) = world_around(SimState::clone(self));
        spawn_payload_rows(
            &mut ecs,
            collect_players(&self.ecs),
            collect_projectiles(&self.ecs),
            collect_script_movers(&self.ecs),
            collect_dropped_items(&self.ecs),
        );
        crate::script::copy_state(&self.ecs, &mut ecs);
        Self { ecs, state_entity, schedule: crate::step::schedule() }
    }
}

impl fmt::Debug for SimWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimWorld").field("state", &**self).finish()
    }
}

const STATE_MISSING: &str = "simulation state entity is missing its SimState component";

impl Deref for SimWorld {
    type Target = SimState;

    fn deref(&self) -> &SimState {
        self.ecs.get::<SimState>(self.state_entity).expect(STATE_MISSING)
    }
}

impl DerefMut for SimWorld {
    fn deref_mut(&mut self) -> &mut SimState {
        self.ecs.get_mut::<SimState>(self.state_entity).expect(STATE_MISSING).into_inner()
    }
}

impl SimWorld {
    pub fn new() -> Self {
        Self::default()
    }

    /// The working view that systems and helpers in this crate use to reach the rows.
    pub(crate) fn frame(&mut self) -> FrameWorld<'_> {
        FrameWorld::new(&mut self.ecs, self.state_entity)
    }

    fn run(
        &mut self,
        tick: Tick,
        input: &TickInput,
        msec: i32,
        reason: StepReason,
    ) -> Result<Snapshot, crate::script::Fault> {
        crate::step::run_schedule(&mut self.ecs, &mut self.schedule, tick, input, msec, reason)
    }
}

/// Why a tick is being run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StepReason {
    /// The server's own frame: the one that moves the shared world forward.
    #[default]
    AuthorityFrame,
    /// A client running a tick it has not run before, ahead of the server.
    PredictNew,
    /// A client re-running a tick it already ran, after the server corrected it.
    Replay,
}

impl StepReason {
    pub const fn advances_authority_world(self) -> bool {
        matches!(self, Self::AuthorityFrame)
    }
}

/// Runs one tick and returns what it produced. A script fault is a bug in the loaded scripts and
/// stops the process.
pub fn step(
    world: &mut SimWorld,
    tick: Tick,
    input: &TickInput,
    msec: i32,
    reason: StepReason,
) -> Snapshot {
    try_step(world, tick, input, msec, reason)
        .unwrap_or_else(|fault| panic!("GSC execution failed: {fault}"))
}

/// As [`step`], handing a script fault back instead of stopping.
pub fn try_step(
    world: &mut SimWorld,
    tick: Tick,
    input: &TickInput,
    msec: i32,
    reason: StepReason,
) -> Result<Snapshot, crate::script::Fault> {
    world.run(tick, input, msec, reason)
}
