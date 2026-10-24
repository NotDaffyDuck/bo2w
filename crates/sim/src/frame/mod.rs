//! `FrameWorld`: the view of the simulation one system works through. It borrows the ECS `World`, derefs to the
//! `SimState` kept on the state entity, and adds the entity rows (players, projectiles, script movers, dropped items)
//! that sit beside it.

use core::ops::{Deref, DerefMut};

use bevy_ecs::prelude::{Entity, World};
use playerstate_t6::game::PlayerState;

use crate::snapshot::Snapshot;
use crate::world::{ClientId, SimState, Tick};

mod adopt;
mod movers;
mod objects;
mod players;
mod rows;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use rows::PayloadIndex;
pub(crate) use store::{
    collect_dropped_items, collect_players, collect_projectiles, collect_script_movers, install_state_entity,
    player_ref, player_row_count, script_mover_by_id, script_mover_by_number, script_mover_row_count,
    spawn_payload_rows, state_entity, visit_players, visit_projectiles, visit_script_movers,
};

pub(crate) struct FrameWorld<'w> {
    ecs: &'w mut World,
    state_entity: Entity,
}

impl<'w> FrameWorld<'w> {
    pub(crate) fn from_world(ecs: &'w mut World) -> Self {
        let state_entity = state_entity(ecs);
        Self { ecs, state_entity }
    }

    pub(crate) fn new(ecs: &'w mut World, state_entity: Entity) -> Self {
        Self { ecs, state_entity }
    }
}

impl Deref for FrameWorld<'_> {
    type Target = SimState;

    fn deref(&self) -> &SimState {
        self.ecs.get::<SimState>(self.state_entity).expect("simulation state entity is missing its SimState component")
    }
}

impl DerefMut for FrameWorld<'_> {
    fn deref_mut(&mut self) -> &mut SimState {
        self.ecs
            .get_mut::<SimState>(self.state_entity)
            .expect("simulation state entity is missing its SimState component")
            .into_inner()
    }
}

impl FrameWorld<'_> {
    pub(crate) fn ecs(&mut self) -> &mut World {
        self.ecs
    }

    /// The entity world for reading only.
    pub(crate) fn ecs_read(&self) -> &World {
        self.ecs
    }

    pub fn snapshot(&self, tick: Tick) -> Snapshot {
        self.snapshot_with_dynamic_rows(
            tick,
            collect_players(self.ecs),
            collect_projectiles(self.ecs),
            collect_script_movers(self.ecs),
            collect_dropped_items(self.ecs),
        )
    }

    pub fn adopt_snapshot(&mut self, snapshot: &Snapshot) -> crate::AdoptReport {
        let (report, projectiles, movers, items) = self.adopt_snapshot_state(snapshot);
        adopt::reconcile(self.ecs, &snapshot.players, projectiles, movers, items);
        report
    }

    /// Adopts only the local client's own player row; the dynamic rows still follow the snapshot.
    pub fn adopt_prediction_snapshot(&mut self, snapshot: &Snapshot, local: ClientId) -> crate::AdoptReport {
        let (report, projectiles, movers, items) = self.adopt_prediction_snapshot_state(snapshot, local);
        let own: &[(ClientId, PlayerState)] = snapshot
            .players
            .iter()
            .find(|(id, _)| *id == local)
            .map(std::slice::from_ref)
            .unwrap_or_default();
        adopt::reconcile(self.ecs, own, projectiles, movers, items);
        report
    }

    pub fn shutdown_game(&mut self) {
        *self.deref_mut() = SimState::new();
        store::clear_rows(self.ecs);
    }
}
