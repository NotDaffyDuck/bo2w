//! The per-entity rows that sit beside `SimState` in the ECS, and the index that finds them again.
//!
//! Players are found by client id. Projectiles, script movers and dropped items are found by gentity number; the
//! entity kernel (not this index) decides which numbers are live, so a row can briefly outlive its number.

use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::{Component, Entity};
use playerstate_t6::game::PlayerState;

use crate::ProjectileState;
use crate::gentity::{EntityRunKind, ScriptMoverGentity};
use crate::item::DroppedItem;
use crate::world::ClientId;

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct PlayerRow {
    pub(super) client: ClientId,
    pub(super) state: PlayerState,
}

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct ProjectileRow(ProjectileState);

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct ScriptMoverRow(ScriptMoverGentity);

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct DroppedItemRow(DroppedItem);

/// A row that carries one gentity-numbered payload. Lets the spawn, lookup, replace and remove code be written once.
pub(super) trait NumberedRow: Component<Mutability = Mutable> + Sized {
    type Payload: Copy;

    /// The kind the entity kernel must hold the number as when a row is spawned.
    const SPAWN_KIND: EntityRunKind;
    /// What a panic message calls this row.
    const LABEL: &'static str;

    fn wrap(payload: Self::Payload) -> Self;
    fn payload(&self) -> &Self::Payload;
    fn payload_mut(&mut self) -> &mut Self::Payload;
    fn number_of(payload: &Self::Payload) -> i32;

    /// Whether a kernel slot of this kind counts as one of these rows when listing numbers.
    fn listed(kind: EntityRunKind) -> bool {
        kind == Self::SPAWN_KIND
    }
}

impl NumberedRow for ProjectileRow {
    type Payload = ProjectileState;
    const SPAWN_KIND: EntityRunKind = EntityRunKind::Missile;
    const LABEL: &'static str = "in-flight projectile";

    fn wrap(payload: ProjectileState) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &ProjectileState {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut ProjectileState {
        &mut self.0
    }
    fn number_of(payload: &ProjectileState) -> i32 {
        payload.entnum
    }
}

impl NumberedRow for ScriptMoverRow {
    type Payload = ScriptMoverGentity;
    const SPAWN_KIND: EntityRunKind = EntityRunKind::ScriptMover;
    const LABEL: &'static str = "script mover";

    fn wrap(payload: ScriptMoverGentity) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &ScriptMoverGentity {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut ScriptMoverGentity {
        &mut self.0
    }
    fn number_of(payload: &ScriptMoverGentity) -> i32 {
        payload.state.number
    }

    /// Primary-light entities are listed with the movers, though only a `ScriptMover` slot can be given a row.
    fn listed(kind: EntityRunKind) -> bool {
        matches!(kind, EntityRunKind::ScriptMover | EntityRunKind::PrimaryLight)
    }
}

impl NumberedRow for DroppedItemRow {
    type Payload = DroppedItem;
    const SPAWN_KIND: EntityRunKind = EntityRunKind::Item;
    const LABEL: &'static str = "dropped item";

    fn wrap(payload: DroppedItem) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &DroppedItem {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut DroppedItem {
        &mut self.0
    }
    fn number_of(payload: &DroppedItem) -> i32 {
        payload.state.number
    }
}

/// Which row entity answers to which gentity number or client. Lives on the same entity as `SimState`.
#[derive(Component, Debug, Default)]
pub(crate) struct PayloadIndex {
    /// Indexed by gentity number.
    numbered: Vec<Option<Entity>>,
    /// Ascending by client id, one entry per client.
    clients: Vec<(ClientId, Entity)>,
}

impl PayloadIndex {
    pub(super) fn numbered(&self, number: i32) -> Option<Entity> {
        self.numbered.get(usize::try_from(number).ok()?).copied().flatten()
    }

    pub(super) fn bind_numbered(&mut self, number: i32, row: Entity) {
        let at = usize::try_from(number).expect("a row is only bound to a number the kernel holds");
        if self.numbered.len() <= at {
            self.numbered.resize(at + 1, None);
        }
        self.numbered[at] = Some(row);
    }

    pub(super) fn release_numbered(&mut self, number: i32) {
        let slot = usize::try_from(number).ok().and_then(|at| self.numbered.get_mut(at));
        if let Some(slot) = slot {
            *slot = None;
        }
    }

    /// Every bound `(number, row)` pair, lowest number first.
    pub(super) fn numbered_rows(&self) -> impl Iterator<Item = (i32, Entity)> + '_ {
        self.numbered
            .iter()
            .enumerate()
            .filter_map(|(at, row)| Some((i32::try_from(at).ok()?, (*row)?)))
    }

    fn client_slot(&self, id: ClientId) -> Result<usize, usize> {
        self.clients.binary_search_by_key(&id.0, |(client, _)| client.0)
    }

    pub(super) fn client_row(&self, id: ClientId) -> Option<Entity> {
        self.client_slot(id).ok().map(|at| self.clients[at].1)
    }

    pub(super) fn bind_client(&mut self, id: ClientId, row: Entity) {
        match self.client_slot(id) {
            Ok(at) => self.clients[at].1 = row,
            Err(at) => self.clients.insert(at, (id, row)),
        }
    }

    pub(super) fn release_client(&mut self, id: ClientId) {
        if let Ok(at) = self.client_slot(id) {
            self.clients.remove(at);
        }
    }

    pub(super) fn clients(&self) -> &[(ClientId, Entity)] {
        &self.clients
    }

    /// All bound rows, numbered ones first.
    pub(super) fn every_row(&self) -> Vec<Entity> {
        let numbered = self.numbered_rows().map(|(_, row)| row);
        let clients = self.clients.iter().map(|&(_, row)| row);
        numbered.chain(clients).collect()
    }
}
