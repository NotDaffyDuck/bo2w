//! Bringing the table in line with a set of claims: refresh the entries that still stand for
//! their claim, replace the ones that do not, retire the ones nobody claims.

use bevy::prelude::*;

use super::claims::Claims;
use super::{ClientEntityRecord, EntitySpawnTally, EntityTableCell, EntityTableCells, EntitySyncBranch};
use crate::client::presentation::entity_records::{
    ClientEntityPoseState, refresh_known, restart_body, tear_down_object,
};

/// What one pass over the table is doing.
pub(super) struct Pass {
    /// The time of the snapshot the claims came from.
    pub(super) for_instant_millis: i32,
    /// Set bodies where the claim has them instead of sliding them there.
    pub(super) reset_all: bool,
    pub(super) arm: EntitySyncBranch,
}

/// Once a pair of snapshots is ready, every body that is in the later one starts its blend from
/// where the later one put it.
pub(super) fn advance_blends(pose_states: &mut Query<&mut ClientEntityPoseState>) {
    for mut runtime in pose_states.iter_mut() {
        if runtime.present_in_newer_frame() {
            runtime.blend_copy_crossfade();
        }
    }
}

fn retire(commands: &mut Commands, pose_states: &mut Query<&mut ClientEntityPoseState>, gone: EntityTableCell) {
    if let Ok(mut runtime) = pose_states.get_mut(gone.held_entity) {
        tear_down_object(&mut runtime);
    }
    commands.entity(gone.held_entity).despawn();
}

/// The entity in `slot` if it still stands for `number` and `client`.
fn standing_entity(
    slot: Option<EntityTableCell>,
    slot_no: u16,
    client: Option<sim::ClientId>,
    id_records: &Query<&ClientEntityRecord>,
    pose_states: &Query<&mut ClientEntityPoseState>,
) -> Option<Entity> {
    let slot = slot?;
    let identity = id_records.get(slot.held_entity).ok()?;
    let stands = identity.slot_no == slot_no && identity.holder == client;
    (stands && pose_states.get(slot.held_entity).is_ok()).then_some(slot.held_entity)
}

/// Give every claim an entity: the one already in its slot if that still fits, else a new one.
pub(super) fn settle(
    commands: &mut Commands,
    claims: &Claims,
    table: &mut EntityTableCells,
    id_records: &Query<&ClientEntityRecord>,
    pose_states: &mut Query<&mut ClientEntityPoseState>,
    pass: &Pass,
    tally: &mut EntitySpawnTally,
) {
    for (slot_no, occupant) in &claims.queued {
        let seat = &mut table.cells[usize::from(*slot_no)];
        let state = occupant.state;
        if let Some(entity) = standing_entity(*seat, *slot_no, occupant.owner, id_records, pose_states)
        {
            if let Ok(mut runtime) = pose_states.get_mut(entity) {
                if pass.reset_all {
                    restart_body(&mut runtime, state, pass.for_instant_millis, false);
                } else {
                    refresh_known(&mut runtime, state, pass.for_instant_millis);
                }
            }
            tally.count_refresh(pass.arm);
            continue;
        }

        if let Some(gone) = seat.take() {
            retire(commands, pose_states, gone);
        }
        let mut runtime = ClientEntityPoseState::default();
        restart_body(&mut runtime, state, pass.for_instant_millis, true);
        let record = ClientEntityRecord {
            slot_no: *slot_no,
            holder: occupant.owner,
        };
        let entity = commands.spawn((record, runtime)).id();
        *seat = Some(EntityTableCell {
            holder: occupant.owner,
            held_entity: entity,
        });
        tally.record_spawn(pass.arm);
    }
}

/// Retire every entry whose number nobody claimed.
pub(super) fn retire_unclaimed(
    commands: &mut Commands,
    claims: &Claims,
    table: &mut EntityTableCells,
    pose_states: &mut Query<&mut ClientEntityPoseState>,
) {
    for (idx, seat) in table.cells.iter_mut().enumerate() {
        if claims.is_taken(idx) {
            continue;
        }
        if let Some(gone) = seat.take() {
            retire(commands, pose_states, gone);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use sim::{ClientId, PlayerCorpsePool};

    use super::*;
    use crate::client::presentation::entity_slots::claims::collect;
    use crate::identity_slips::LinkIdentityDeficits;

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<EntityTableCells>();
        world.init_resource::<EntitySpawnTally>();
        world
    }

    fn claims_of(entities: &[entity_t6::EntitySnapshot]) -> Claims {
        let mut gaps = LinkIdentityDeficits::default();
        collect(&[], &PlayerCorpsePool::default(), entities, None, &mut gaps)
    }

    fn missile(number: i32) -> entity_t6::EntitySnapshot {
        entity_t6::EntitySnapshot {
            slot_index: number,
            e_type: entity_t6::et::MISSILE,
            ..Default::default()
        }
    }

    fn player_entity(number: i32, seat_idx: u8) -> entity_t6::EntitySnapshot {
        entity_t6::EntitySnapshot {
            slot_index: number,
            e_type: entity_t6::et::PLAYER,
            client_num: seat_idx,
            ..Default::default()
        }
    }

    /// One pass of `settle` then `retire_unclaimed` over `claims`.
    fn run_pass(world: &mut World, claims: Claims, reset_all: bool, arm: EntitySyncBranch) {
        world
            .run_system_once(
                move |mut commands: Commands,
                      mut table: ResMut<EntityTableCells>,
                      records: Query<&ClientEntityRecord>,
                      mut pose_states: Query<&mut ClientEntityPoseState>,
                      mut tally: ResMut<EntitySpawnTally>| {
                    let pass = Pass {
                        for_instant_millis: 100,
                        reset_all,
                        arm,
                    };
                    settle(
                        &mut commands,
                        &claims,
                        &mut table,
                        &records,
                        &mut pose_states,
                        &pass,
                        &mut tally,
                    );
                    retire_unclaimed(&mut commands, &claims, &mut table, &mut pose_states);
                },
            )
            .expect("the pass runs");
    }

    fn entity_at(world: &World, slot_no: u16) -> Option<Entity> {
        world.resource::<EntityTableCells>().object_at_index(slot_no)
    }

    fn births_and_applies(world: &World) -> (u32, u32, u32) {
        let tally = world.resource::<EntitySpawnTally>();
        (tally.host_spawn_count, tally.receiver_spawn_count, tally.applied_by_receiver)
    }

    #[test]
    fn a_new_claim_gets_an_entity_that_knows_its_number_and_owner() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[player_entity(9, 9)]), false, EntitySyncBranch::Client);
        let entity = entity_at(&world, 9).expect("slot 9 is filled");
        let identity = *world.get::<ClientEntityRecord>(entity).expect("entry carries its identity");
        assert_eq!((identity.number(), identity.client()), (9, Some(ClientId(9))));
        assert!(world.get::<ClientEntityPoseState>(entity).is_some());
        assert_eq!(births_and_applies(&world), (0, 1, 0));
    }

    #[test]
    fn a_claim_that_still_fits_keeps_its_entity_and_counts_a_refresh() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Client);
        let first = entity_at(&world, 50).unwrap();
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Client);
        assert_eq!(entity_at(&world, 50), Some(first));
        run_pass(&mut world, claims_of(&[missile(50)]), true, EntitySyncBranch::Client);
        assert_eq!(entity_at(&world, 50), Some(first));
        assert_eq!(births_and_applies(&world), (0, 1, 2));
    }

    #[test]
    fn only_the_client_side_counts_refreshes() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Authority);
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Authority);
        assert_eq!(births_and_applies(&world), (1, 0, 0));
    }

    #[test]
    fn a_slot_whose_owner_changed_gets_a_new_entity_and_the_old_one_is_gone() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[player_entity(9, 3)]), false, EntitySyncBranch::Client);
        let old = entity_at(&world, 9).unwrap();
        run_pass(&mut world, claims_of(&[player_entity(9, 4)]), false, EntitySyncBranch::Client);
        let new = entity_at(&world, 9).unwrap();
        assert_ne!(old, new);
        assert!(world.get_entity(old).is_err());
        assert_eq!(world.get::<ClientEntityRecord>(new).unwrap().client(), Some(ClientId(4)));
        assert_eq!(births_and_applies(&world), (0, 2, 0));
    }

    #[test]
    fn a_slot_nobody_claims_is_emptied_and_its_entity_despawned() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[missile(50), missile(51)]), false, EntitySyncBranch::Client);
        let kept = entity_at(&world, 51).unwrap();
        let gone = entity_at(&world, 50).unwrap();
        run_pass(&mut world, claims_of(&[missile(51)]), false, EntitySyncBranch::Client);
        assert_eq!(entity_at(&world, 50), None);
        assert!(world.get_entity(gone).is_err());
        assert_eq!(entity_at(&world, 51), Some(kept));
    }

    #[test]
    fn an_entry_that_lost_its_runtime_is_replaced() {
        let mut world = world();
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Client);
        let broken = entity_at(&world, 50).unwrap();
        world.entity_mut(broken).remove::<ClientEntityPoseState>();
        run_pass(&mut world, claims_of(&[missile(50)]), false, EntitySyncBranch::Client);
        let repaired = entity_at(&world, 50).unwrap();
        assert_ne!(broken, repaired);
        assert!(world.get::<ClientEntityPoseState>(repaired).is_some());
    }
}
