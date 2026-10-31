//! Reading and writing the rows straight on the ECS `World`, for callers that hold no `StepWorldView`.

use bevy_ecs::prelude::{Entity, Resource, World};
use playerstate_t6::game::AvatarState;

use super::rows::{DroppedItemRow, NumberedRow, RowDirectory, PlayerRow, ProjectileRow, ScriptMoverRow};
use crate::FlyingProjectile;
use crate::ent_slots::{EntitySlotArena, MoverSlot};
use crate::match_ids::ScriptedModel;
use crate::floor_guns::LootOnGround;
use crate::sim_parts::{SeatNo, MatchSimulation};

/// Where the one `MatchSimulation` entity lives.
#[derive(Resource, Clone, Copy)]
struct StateEntityHandle(Entity);

pub(crate) fn seat_state_holder(ecs: &mut World, thing: Entity) {
    ecs.insert_resource(StateEntityHandle(thing));
}

pub(crate) fn state_holder(ecs: &World) -> Entity {
    ecs.resource::<StateEntityHandle>().0
}

fn kernel(ecs: &World) -> &EntitySlotArena {
    let sim = ecs
        .get::<MatchSimulation>(state_holder(ecs))
        .expect("the ECS holds one SimState component, and this lookup found none");
    sim.entity_kernel()
}

pub(super) fn index(ecs: &World) -> &RowDirectory {
    ecs
        .get::<RowDirectory>(state_holder(ecs))
        .expect("every SimState entity carries a RowDirectory, but this one has none")
}

pub(super) fn index_mut(ecs: &mut World) -> &mut RowDirectory {
    let at = state_holder(ecs);
    ecs
        .get_mut::<RowDirectory>(at)
        .expect("every SimState entity carries a RowDirectory, but this one has none")
        .into_inner()
}

/// Despawns a row that must exist.
pub(super) fn despawn_row(ecs: &mut World, entry: Entity, what: &str) {
    assert!(ecs.despawn(entry), "{what} row vanished before it was despawned");
}

// ---- numbered rows (projectiles, script movers, dropped items) ----

pub(super) fn find<R: NumberedRow>(ecs: &World, number: i32) -> Option<&R::Payload> {
    let row = index(ecs).numbered(number)?;
    ecs.get::<R>(row).map(R::payload)
}

pub(super) fn find_mut<R: NumberedRow>(ecs: &mut World, number: i32) -> Option<&mut R::Payload> {
    let row = index(ecs).numbered(number)?;
    Some(ecs.get_mut::<R>(row)?.into_inner().payload_mut())
}

/// The first row, lowest number first, whose payload `wanted` accepts.
pub(super) fn find_where<R: NumberedRow>(
    ecs: &World,
    wanted: impl Fn(&R::Payload) -> bool,
) -> Option<&R::Payload> {
    listed_numbers::<R>(ecs)
        .into_iter()
        .filter_map(|number| find::<R>(ecs, number))
        .find(|&payload| wanted(payload))
}

/// Every number the kernel holds that counts as this kind of row, lowest first. A listed number may have no row.
pub(super) fn listed_numbers<R: NumberedRow>(ecs: &World) -> Vec<i32> {
    kernel(ecs).held_slots(R::listed)
}

/// The listed numbers that really do have a row.
pub(super) fn present_numbers<R: NumberedRow>(ecs: &World) -> Vec<i32> {
    let mut numbers = listed_numbers::<R>(ecs);
    numbers.retain(|&number| find::<R>(ecs, number).is_some());
    numbers
}

pub(super) fn visit_rows<R: NumberedRow>(ecs: &World, mut visit: impl FnMut(&R::Payload)) {
    for number in listed_numbers::<R>(ecs) {
        if let Some(payload) = find::<R>(ecs, number) {
            visit(payload);
        }
    }
}

pub(super) fn copy_rows<R: NumberedRow>(ecs: &World) -> Vec<R::Payload> {
    let mut rows = Vec::new();
    visit_rows::<R>(ecs, |payload| rows.push(*payload));
    rows
}

/// Gives `payload` a new row. The kernel must already hold its number as the right kind.
pub(super) fn spawn_row<R: NumberedRow>(ecs: &mut World, payload: R::Payload) {
    let number = R::number_of(&payload);
    assert!(
        kernel(ecs).holder_class(number) == Some(R::SPAWN_KIND),
        "{} has no {:?} think occupancy",
        R::LABEL,
        R::SPAWN_KIND,
    );
    let row = ecs.spawn(R::wrap(payload)).id();
    index_mut(ecs).bind_numbered(number, row);
}

/// Removes the row at `number` and hands back what it held; `None` when that number has no row of this kind.
pub(super) fn remove_row<R: NumberedRow>(ecs: &mut World, number: i32) -> Option<R::Payload> {
    let row = index(ecs).numbered(number)?;
    let payload = *ecs.get::<R>(row)?.payload();
    index_mut(ecs).release_numbered(number);
    despawn_row(ecs, row, R::LABEL);
    Some(payload)
}

/// Overwrites the row at the payload's number, or makes one. Whatever other kind of row held that number is replaced.
pub(super) fn upsert_row<R: NumberedRow>(ecs: &mut World, payload: R::Payload) {
    let number = R::number_of(&payload);
    if let Some(row) = index(ecs).numbered(number) {
        if let Some(mut held) = ecs.get_mut::<R>(row) {
            *held.payload_mut() = payload;
            return;
        }
        index_mut(ecs).release_numbered(number);
        despawn_row(ecs, row, R::LABEL);
    }
    spawn_row::<R>(ecs, payload);
}

// ---- players ----

pub(super) fn spawn_player(ecs: &mut World, who: SeatNo, state: AvatarState) {
    let row = ecs.spawn(PlayerRow { client: who, state }).id();
    index_mut(ecs).bind_client(who, row);
}

pub(crate) fn avatar_of(ecs: &World, id: SeatNo) -> Option<&AvatarState> {
    let row = index(ecs).client_row(id)?;
    ecs.get::<PlayerRow>(row).map(|row| &row.state)
}

pub(crate) fn gather_avatars(ecs: &World) -> Vec<(SeatNo, AvatarState)> {
    let mut rows = Vec::new();
    each_avatar_row(ecs, |id, state| rows.push((id, *state)));
    rows
}

pub(crate) fn each_avatar_row(ecs: &World, mut visit: impl FnMut(SeatNo, &AvatarState)) {
    let present = index(ecs).clients().iter().filter_map(|&(id, _)| Some((id, avatar_of(ecs, id)?)));
    present.for_each(|(id, state)| visit(id, state));
}

pub(crate) fn player_row_count(ecs: &World) -> usize {
    index(ecs).clients().len()
}

// ---- the other kinds, under the names the rest of the crate uses ----

pub(crate) fn gather_shells(ecs: &World) -> Vec<FlyingProjectile> {
    copy_rows::<ProjectileRow>(ecs)
}

pub(crate) fn visit_projectiles(ecs: &World, visit: impl FnMut(&FlyingProjectile)) {
    visit_rows::<ProjectileRow>(ecs, visit);
}

pub(crate) fn gather_movers(ecs: &World) -> Vec<MoverSlot> {
    copy_rows::<ScriptMoverRow>(ecs)
}

pub(crate) fn visit_script_movers(ecs: &World, visit: impl FnMut(&MoverSlot)) {
    visit_rows::<ScriptMoverRow>(ecs, visit);
}

/// How many script-mover slots the kernel holds. This counts slots, not rows.
pub(crate) fn script_mover_row_count(ecs: &World) -> usize {
    listed_numbers::<ScriptMoverRow>(ecs).len()
}

pub(crate) fn script_mover_by_number(ecs: &World, number: i32) -> Option<MoverSlot> {
    find::<ScriptMoverRow>(ecs, number).copied()
}

pub(crate) fn script_mover_by_id(ecs: &World, id: ScriptedModel) -> Option<MoverSlot> {
    find_where::<ScriptMoverRow>(ecs, |mover| mover.id == id).copied()
}

pub(crate) fn gather_loot(ecs: &World) -> Vec<LootOnGround> {
    copy_rows::<DroppedItemRow>(ecs)
}

// ---- whole-store operations ----

/// Builds every row from the given payloads (a fresh world being filled from a saved one).
pub(crate) fn seed_rows(
    target: &mut World,
    player_rows: Vec<(SeatNo, AvatarState)>,
    projectile_rows: Vec<FlyingProjectile>,
    mover_rows: Vec<MoverSlot>,
    item_rows: Vec<LootOnGround>,
) {
    player_rows.into_iter().for_each(|(client, state)| spawn_player(target, client, state));
    projectile_rows.into_iter().for_each(|payload| spawn_row::<ProjectileRow>(target, payload));
    mover_rows.into_iter().for_each(|payload| spawn_row::<ScriptMoverRow>(target, payload));
    item_rows.into_iter().for_each(|payload| spawn_row::<DroppedItemRow>(target, payload));
}

/// Despawns every row and empties the index.
pub(super) fn clear_rows(ecs: &mut World) {
    let rows = index(ecs).every_row();
    for entry in rows {
        despawn_row(ecs, entry, "payload");
    }
    *index_mut(ecs) = RowDirectory::default();
}
