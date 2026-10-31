//! `StepWorldView`: the view of the simulation one system works through. It borrows the ECS `World`, derefs to the
//! `MatchSimulation` kept on the state entity, and adds the entity rows (players, shells, script movers, floor guns)
//! that sit beside it.

use core::ops::{Deref, DerefMut};

use bevy_ecs::prelude::{Entity, World};
use playerstate_t6::game::AvatarState;

use crate::tick_publish::TickSnapshot;
use crate::sim_parts::{SeatNo, MatchSimulation, StepNo};

mod adopt;
mod movers;
mod objects;
mod players;
mod rows;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use rows::RowDirectory;
pub(crate) use store::{
    gather_loot, gather_avatars, gather_shells, gather_movers, seat_state_holder,
    avatar_of, player_row_count, script_mover_by_id, script_mover_by_number, script_mover_row_count,
    seed_rows, state_holder, each_avatar_row, visit_projectiles, visit_script_movers,
};

pub(crate) struct StepWorldView<'w> {
    ecs: &'w mut World,
    state_ent: Entity,
}

const MISSING_STATE: &str = "state entity has lost its SimState component";

impl<'a> StepWorldView<'a> {
    pub(crate) fn from_world(ecs: &'a mut World) -> Self {
        let entity = state_holder(ecs);
        Self::new(ecs, entity)
    }

    pub(crate) fn new(world: &'a mut World, state: Entity) -> Self {
        Self { ecs: world, state_ent: state }
    }
}

impl<'a> Deref for StepWorldView<'a> {
    type Target = MatchSimulation;

    fn deref(&self) -> &MatchSimulation {
        self.ecs.get::<MatchSimulation>(self.state_ent).expect(MISSING_STATE)
    }
}

impl<'a> DerefMut for StepWorldView<'a> {
    fn deref_mut(&mut self) -> &mut MatchSimulation {
        let component = self.ecs.get_mut::<MatchSimulation>(self.state_ent);
        component.expect(MISSING_STATE).into_inner()
    }
}

impl<'a> StepWorldView<'a> {
    pub(crate) fn ecs(&mut self) -> &mut World {
        self.ecs
    }

    /// The entity world for reading only.
    pub(crate) fn ecs_read(&self) -> &World {
        self.ecs
    }

    pub fn snapshot(&self, at: StepNo) -> TickSnapshot {
        let players = gather_avatars(self.ecs);
        let shells = gather_shells(self.ecs);
        let script_movers = gather_movers(self.ecs);
        let floor_loot = gather_loot(self.ecs);
        self.snapshot_plus_live_rows(at, players, shells, script_movers, floor_loot)
    }

    pub fn take_over_from(&mut self, incoming: &TickSnapshot) -> crate::TakeoverReport {
        let (summary, projectile_rows, mover_rows, item_rows) = self.adopt_snapshot_state(incoming);
        adopt::reconcile(self.ecs, &incoming.players, projectile_rows, mover_rows, item_rows);
        summary
    }

    /// Adopts only the local client's own player row; the dynamic rows still follow the snapshot.
    pub fn adopt_prediction_snapshot(&mut self, snap: &TickSnapshot, local: SeatNo) -> crate::TakeoverReport {
        let (summary, projectile_rows, mover_rows, item_rows) = self.adopt_prediction_snapshot_state(snap, local);
        let own_at = snap.players.iter().position(|(seat, _)| *seat == local);
        let own: &[(SeatNo, AvatarState)] = own_at.map_or(&[], |at| &snap.players[at..=at]);
        adopt::reconcile(self.ecs, own, projectile_rows, mover_rows, item_rows);
        summary
    }

    pub fn end_game_session(&mut self) {
        let sim: &mut MatchSimulation = self;
        *sim = MatchSimulation::new();
        store::clear_rows(self.ecs);
    }
}
