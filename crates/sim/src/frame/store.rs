//! Reading and writing the rows straight on the ECS `World`, for callers that hold no `FrameWorld`.

use bevy_ecs::prelude::{Entity, Resource, World};
use playerstate_t6::game::PlayerState;

use super::rows::{DroppedItemRow, NumberedRow, PayloadIndex, PlayerRow, ProjectileRow, ScriptMoverRow};
use crate::ProjectileState;
use crate::gentity::{EntityKernel, ScriptMoverGentity};
use crate::identities::ScriptModelId;
use crate::item::DroppedItem;
use crate::world::{ClientId, SimState};

/// Where the one `SimState` entity lives.
#[derive(Resource, Clone, Copy)]
struct SimStateEntity(Entity);

pub(crate) fn install_state_entity(world: &mut World, entity: Entity) {
    world.insert_resource(SimStateEntity(entity));
}

pub(crate) fn state_entity(world: &World) -> Entity {
    world.resource::<SimStateEntity>().0
}

fn kernel(world: &World) -> &EntityKernel {
    world
        .get::<SimState>(state_entity(world))
        .expect("simulation ECS must contain exactly one SimState component")
        .entity_kernel()
}

pub(super) fn index(world: &World) -> &PayloadIndex {
    world
        .get::<PayloadIndex>(state_entity(world))
        .expect("simulation ECS must contain a PayloadIndex beside SimState")
}

pub(super) fn index_mut(world: &mut World) -> &mut PayloadIndex {
    let at = state_entity(world);
    world
        .get_mut::<PayloadIndex>(at)
        .expect("simulation ECS must contain a PayloadIndex beside SimState")
        .into_inner()
}

/// Despawns a row that must exist.
pub(super) fn despawn_row(world: &mut World, row: Entity, what: &str) {
    assert!(world.despawn(row), "{what} row vanished before it was despawned");
}

// ---- numbered rows (projectiles, script movers, dropped items) ----

pub(super) fn find<R: NumberedRow>(world: &World, number: i32) -> Option<&R::Payload> {
    let row = index(world).numbered(number)?;
    world.get::<R>(row).map(R::payload)
}

pub(super) fn find_mut<R: NumberedRow>(world: &mut World, number: i32) -> Option<&mut R::Payload> {
    let row = index(world).numbered(number)?;
    Some(world.get_mut::<R>(row)?.into_inner().payload_mut())
}

/// The first row, lowest number first, whose payload `wanted` accepts.
pub(super) fn find_where<R: NumberedRow>(
    world: &World,
    wanted: impl Fn(&R::Payload) -> bool,
) -> Option<&R::Payload> {
    listed_numbers::<R>(world)
        .into_iter()
        .filter_map(|number| find::<R>(world, number))
        .find(|&payload| wanted(payload))
}

/// Every number the kernel holds that counts as this kind of row, lowest first. A listed number may have no row.
pub(super) fn listed_numbers<R: NumberedRow>(world: &World) -> Vec<i32> {
    kernel(world).occupied_numbers(R::listed)
}

/// The listed numbers that really do have a row.
pub(super) fn present_numbers<R: NumberedRow>(world: &World) -> Vec<i32> {
    let mut numbers = listed_numbers::<R>(world);
    numbers.retain(|&number| find::<R>(world, number).is_some());
    numbers
}

pub(super) fn visit_rows<R: NumberedRow>(world: &World, mut visit: impl FnMut(&R::Payload)) {
    for number in listed_numbers::<R>(world) {
        if let Some(payload) = find::<R>(world, number) {
            visit(payload);
        }
    }
}

pub(super) fn copy_rows<R: NumberedRow>(world: &World) -> Vec<R::Payload> {
    let mut rows = Vec::new();
    visit_rows::<R>(world, |payload| rows.push(*payload));
    rows
}

/// Gives `payload` a new row. The kernel must already hold its number as the right kind.
pub(super) fn spawn_row<R: NumberedRow>(world: &mut World, payload: R::Payload) {
    let number = R::number_of(&payload);
    assert!(
        kernel(world).occupied_kind(number) == Some(R::SPAWN_KIND),
        "{} has no {:?} think occupancy",
        R::LABEL,
        R::SPAWN_KIND,
    );
    let row = world.spawn(R::wrap(payload)).id();
    index_mut(world).bind_numbered(number, row);
}

/// Removes the row at `number` and hands back what it held; `None` when that number has no row of this kind.
pub(super) fn remove_row<R: NumberedRow>(world: &mut World, number: i32) -> Option<R::Payload> {
    let row = index(world).numbered(number)?;
    let payload = *world.get::<R>(row)?.payload();
    index_mut(world).release_numbered(number);
    despawn_row(world, row, R::LABEL);
    Some(payload)
}

/// Overwrites the row at the payload's number, or makes one. Whatever other kind of row held that number is replaced.
pub(super) fn upsert_row<R: NumberedRow>(world: &mut World, payload: R::Payload) {
    let number = R::number_of(&payload);
    if let Some(row) = index(world).numbered(number) {
        if let Some(mut held) = world.get_mut::<R>(row) {
            *held.payload_mut() = payload;
            return;
        }
        index_mut(world).release_numbered(number);
        despawn_row(world, row, R::LABEL);
    }
    spawn_row::<R>(world, payload);
}

// ---- players ----

pub(super) fn spawn_player(world: &mut World, client: ClientId, state: PlayerState) {
    let row = world.spawn(PlayerRow { client, state }).id();
    index_mut(world).bind_client(client, row);
}

pub(crate) fn player_ref(world: &World, id: ClientId) -> Option<&PlayerState> {
    let row = index(world).client_row(id)?;
    world.get::<PlayerRow>(row).map(|row| &row.state)
}

pub(crate) fn collect_players(world: &World) -> Vec<(ClientId, PlayerState)> {
    let mut rows = Vec::new();
    visit_players(world, |id, state| rows.push((id, *state)));
    rows
}

pub(crate) fn visit_players(world: &World, mut visit: impl FnMut(ClientId, &PlayerState)) {
    for &(id, _) in index(world).clients() {
        if let Some(state) = player_ref(world, id) {
            visit(id, state);
        }
    }
}

pub(crate) fn player_row_count(world: &World) -> usize {
    index(world).clients().len()
}

// ---- the other kinds, under the names the rest of the crate uses ----

pub(crate) fn collect_projectiles(world: &World) -> Vec<ProjectileState> {
    copy_rows::<ProjectileRow>(world)
}

pub(crate) fn visit_projectiles(world: &World, visit: impl FnMut(&ProjectileState)) {
    visit_rows::<ProjectileRow>(world, visit);
}

pub(crate) fn collect_script_movers(world: &World) -> Vec<ScriptMoverGentity> {
    copy_rows::<ScriptMoverRow>(world)
}

pub(crate) fn visit_script_movers(world: &World, visit: impl FnMut(&ScriptMoverGentity)) {
    visit_rows::<ScriptMoverRow>(world, visit);
}

/// How many script-mover slots the kernel holds. This counts slots, not rows.
pub(crate) fn script_mover_row_count(world: &World) -> usize {
    listed_numbers::<ScriptMoverRow>(world).len()
}

pub(crate) fn script_mover_by_number(world: &World, number: i32) -> Option<ScriptMoverGentity> {
    find::<ScriptMoverRow>(world, number).copied()
}

pub(crate) fn script_mover_by_id(world: &World, id: ScriptModelId) -> Option<ScriptMoverGentity> {
    find_where::<ScriptMoverRow>(world, |mover| mover.id == id).copied()
}

pub(crate) fn collect_dropped_items(world: &World) -> Vec<DroppedItem> {
    copy_rows::<DroppedItemRow>(world)
}

// ---- whole-store operations ----

/// Builds every row from the given payloads (a fresh world being filled from a saved one).
pub(crate) fn spawn_payload_rows(
    world: &mut World,
    players: Vec<(ClientId, PlayerState)>,
    projectiles: Vec<ProjectileState>,
    movers: Vec<ScriptMoverGentity>,
    items: Vec<DroppedItem>,
) {
    for (client, state) in players {
        spawn_player(world, client, state);
    }
    for projectile in projectiles {
        spawn_row::<ProjectileRow>(world, projectile);
    }
    for mover in movers {
        spawn_row::<ScriptMoverRow>(world, mover);
    }
    for item in items {
        spawn_row::<DroppedItemRow>(world, item);
    }
}

/// Despawns every row and empties the index.
pub(super) fn clear_rows(world: &mut World) {
    let rows = index(world).every_row();
    for row in rows {
        despawn_row(world, row, "payload");
    }
    *index_mut(world) = PayloadIndex::default();
}
