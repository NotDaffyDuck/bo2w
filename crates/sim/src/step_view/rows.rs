//! The per-entity rows that sit beside `MatchSimulation` in the ECS, and the index that finds them again.
//!
//! Players are found by client id. Projectiles, script movers and dropped items are found by gentity number; the
//! entity kernel (not this index) decides which numbers are live, so a row can briefly outlive its number.

use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::{Component, Entity};
use playerstate_t6::game::AvatarState;

use crate::FlyingProjectile;
use crate::ent_slots::{EntityBehaviorClass, MoverSlot};
use crate::floor_guns::LootOnGround;
use crate::sim_parts::SeatNo;

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct PlayerRow {
    pub(super) client: SeatNo,
    pub(super) state: AvatarState,
}

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct ProjectileRow(FlyingProjectile);

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct ScriptMoverRow(MoverSlot);

#[derive(Component, Clone, Copy, Debug)]
pub(super) struct DroppedItemRow(LootOnGround);

/// A row that carries one gentity-numbered payload. Lets the spawn, lookup, replace and remove code be written once.
pub(super) trait NumberedRow: Component<Mutability = Mutable> + Sized {
    type Payload: Copy;

    /// The kind the entity kernel must hold the number as when a row is spawned.
    const SPAWN_KIND: EntityBehaviorClass;
    /// What a panic message calls this row.
    const LABEL: &'static str;

    fn wrap(payload: Self::Payload) -> Self;
    fn payload(&self) -> &Self::Payload;
    fn payload_mut(&mut self) -> &mut Self::Payload;
    fn number_of(payload: &Self::Payload) -> i32;

    /// Whether a kernel slot of this kind counts as one of these rows when listing numbers.
    fn listed(kind: EntityBehaviorClass) -> bool {
        kind == Self::SPAWN_KIND
    }
}

impl NumberedRow for ProjectileRow {
    type Payload = FlyingProjectile;
    const SPAWN_KIND: EntityBehaviorClass = EntityBehaviorClass::Missile;
    const LABEL: &'static str = "in-flight projectile";

    fn wrap(payload: FlyingProjectile) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &FlyingProjectile {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut FlyingProjectile {
        &mut self.0
    }
    fn number_of(payload: &FlyingProjectile) -> i32 {
        payload.ent_no
    }
}

impl NumberedRow for ScriptMoverRow {
    type Payload = MoverSlot;
    const SPAWN_KIND: EntityBehaviorClass = EntityBehaviorClass::ScriptedMover;
    const LABEL: &'static str = "script mover";

    fn wrap(payload: MoverSlot) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &MoverSlot {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut MoverSlot {
        &mut self.0
    }
    fn number_of(payload: &MoverSlot) -> i32 {
        payload.state.slot_index
    }

    /// Primary-light entities are listed with the movers, though only a `ScriptMover` slot can be given a row.
    fn listed(kind: EntityBehaviorClass) -> bool {
        matches!(kind, EntityBehaviorClass::ScriptedMover | EntityBehaviorClass::MainLightSource)
    }
}

impl NumberedRow for DroppedItemRow {
    type Payload = LootOnGround;
    const SPAWN_KIND: EntityBehaviorClass = EntityBehaviorClass::Item;
    const LABEL: &'static str = "dropped item";

    fn wrap(payload: LootOnGround) -> Self {
        Self(payload)
    }
    fn payload(&self) -> &LootOnGround {
        &self.0
    }
    fn payload_mut(&mut self) -> &mut LootOnGround {
        &mut self.0
    }
    fn number_of(payload: &LootOnGround) -> i32 {
        payload.state.slot_index
    }
}

/// Which row entity answers to which gentity number or client. Lives on the same entity as `MatchSimulation`.
#[derive(Component, Debug, Default)]
pub(crate) struct RowDirectory {
    /// Indexed by gentity number.
    numbered: Vec<Option<Entity>>,
    /// Ascending by client id, one entry per client.
    clients: Vec<(SeatNo, Entity)>,
}

impl RowDirectory {
    pub(super) fn numbered(&self, slot_no: i32) -> Option<Entity> {
        self.numbered.get(usize::try_from(slot_no).ok()?).copied().flatten()
    }

    pub(super) fn bind_numbered(&mut self, slot_no: i32, row: Entity) {
        let at = usize::try_from(slot_no).expect("a row is only bound to a number the kernel holds");
        if self.numbered.len() <= at {
            self.numbered.resize(at + 1, None);
        }
        self.numbered[at] = Some(row);
    }

    pub(super) fn release_numbered(&mut self, slot_no: i32) {
        let cell = usize::try_from(slot_no).ok().and_then(|at| self.numbered.get_mut(at));
        if let Some(cell) = cell {
            *cell = None;
        }
    }

    /// Every bound `(row, number)` pair, lowest number first.
    pub(super) fn numbered_rows(&self) -> impl Iterator<Item = (Entity, i32)> + '_ {
        self.numbered
            .iter()
            .enumerate()
            .filter_map(|(at, row)| Some(((*row)?, i32::try_from(at).ok()?)))
    }

    fn client_slot(&self, id: SeatNo) -> Result<usize, usize> {
        self.clients.binary_search_by_key(&id.0, |(who, _)| who.0)
    }

    pub(super) fn client_row(&self, id: SeatNo) -> Option<Entity> {
        self.client_slot(id).ok().map(|at| self.clients[at].1)
    }

    pub(super) fn bind_client(&mut self, id: SeatNo, row: Entity) {
        match self.client_slot(id) {
            Ok(at) => self.clients[at].1 = row,
            Err(at) => self.clients.insert(at, (id, row)),
        }
    }

    pub(super) fn release_client(&mut self, id: SeatNo) {
        if let Ok(at) = self.client_slot(id) {
            self.clients.remove(at);
        }
    }

    pub(super) fn clients(&self) -> &[(SeatNo, Entity)] {
        &self.clients
    }

    /// All bound rows, numbered ones first.
    pub(super) fn every_row(&self) -> Vec<Entity> {
        let numbered = self.numbered_rows().map(|(row, _)| row);
        let clients = self.clients.iter().map(|&(_, row)| row);
        numbered.chain(clients).collect()
    }
}
