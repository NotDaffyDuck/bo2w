use entity_t6::{Trajectory, et, trajectory::tr_type};
use playerstate_t6::game::buttons;
use trace_t6::ENTITYNUM_WORLD;

use super::*;

const FIRST: i32 = GENTITY_RESERVED_COUNT;
const WORLD: i32 = ENTITYNUM_WORLD as i32;

fn kernel_at(ms: i32) -> EntityKernel {
    let mut kernel = EntityKernel::default();
    kernel.begin_frame(ms);
    kernel
}

/// The handle a snapshot-built kernel gives the `n`th number (generation 0).
fn nth(n: i32) -> EntityRef {
    EntityRef::new(FIRST + n, 0)
}

fn held(kind: EntityRunKind) -> EntityKernelOccupiedSnapshot {
    EntityKernelOccupiedSnapshot {
        kind,
        linked: false,
        relations: EntityRelations::default(),
        next_think_ms: None,
        transient_event_time_ms: None,
    }
}

fn live_slot(kind: EntityRunKind) -> EntityKernelSlotSnapshot {
    EntityKernelSlotSnapshot { generation: 0, occupied: Some(held(kind)), freed_at_ms: i32::MIN }
}

fn free_slot() -> EntityKernelSlotSnapshot {
    EntityKernelSlotSnapshot { generation: 1, occupied: None, freed_at_ms: 0 }
}

fn snapshot_of(slots: Vec<EntityKernelSlotSnapshot>, free_fifo: Vec<i32>) -> EntityKernelSnapshot {
    EntityKernelSnapshot { high_water: FIRST + slots.len() as i32, slots, free_fifo, ..Default::default() }
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

// ---- numbers and generations ----

#[test]
fn numbers_are_handed_out_upward_from_the_reserved_block() {
    let mut kernel = kernel_at(0);
    let first = kernel.allocate(EntityRunKind::General).unwrap();
    let second = kernel.allocate(EntityRunKind::Item).unwrap();
    let third = kernel.allocate(EntityRunKind::Missile).unwrap();
    assert_eq!([first.number(), second.number(), third.number()], [FIRST, FIRST + 1, FIRST + 2]);
    assert_eq!(first.generation(), 0);
    assert_eq!(kernel.high_water(), FIRST + 3);
    assert_eq!(kernel.occupied_kind(FIRST + 1), Some(EntityRunKind::Item));
}

#[test]
fn a_fresh_kernel_has_nothing_held() {
    let kernel = EntityKernel::default();
    assert_eq!(kernel.high_water(), FIRST);
    assert_eq!(kernel.current_ref(FIRST), Err(EntityRefError::Free));
    assert!(kernel.occupied_numbers(|_| true).is_empty());
}

#[test]
fn a_freed_number_rests_for_half_a_second() {
    let mut kernel = kernel_at(1000);
    let old = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.free(old).unwrap();

    kernel.begin_frame(1499);
    assert_eq!(kernel.allocate(EntityRunKind::General).unwrap().number(), FIRST + 1);

    kernel.begin_frame(1500);
    let again = kernel.allocate(EntityRunKind::General).unwrap();
    assert_eq!((again.number(), again.generation()), (FIRST, 1));
}

#[test]
fn freed_numbers_come_back_in_the_order_they_were_freed() {
    let mut kernel = kernel_at(0);
    let a = kernel.allocate(EntityRunKind::General).unwrap();
    let b = kernel.allocate(EntityRunKind::General).unwrap();
    let c = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.free(c).unwrap();
    kernel.free(a).unwrap();
    kernel.free(b).unwrap();

    kernel.begin_frame(10_000);
    let order: Vec<i32> = (0..3).map(|_| kernel.allocate(EntityRunKind::General).unwrap().number()).collect();
    assert_eq!(order, [c.number(), a.number(), b.number()]);
}

#[test]
fn an_old_handle_stops_working_once_the_number_is_reused() {
    let mut kernel = kernel_at(0);
    let old = kernel.allocate(EntityRunKind::Item).unwrap();
    kernel.free(old).unwrap();
    kernel.begin_frame(600);
    let new = kernel.allocate(EntityRunKind::Missile).unwrap();
    assert_eq!(new.number(), old.number());

    let stale = Err(EntityRefError::StaleGeneration { current: 1 });
    assert_eq!(kernel.resolve(old).map(|_| ()), stale);
    assert_eq!(kernel.set_linked(old, true), stale);
    assert_eq!(kernel.mark_transient_event(old, 5), stale);
    assert_eq!(kernel.take_due_think(old), stale.map(|()| false));
    assert_eq!(kernel.free(old), stale);
    assert_eq!(kernel.resolve(new).unwrap().kind, EntityRunKind::Missile);
}

#[test]
fn freeing_twice_is_refused() {
    let mut kernel = kernel_at(0);
    let entity = kernel.allocate(EntityRunKind::General).unwrap();
    assert_eq!(kernel.free(entity), Ok(()));
    assert_eq!(kernel.free(entity), Err(EntityRefError::StaleGeneration { current: 1 }));
}

#[test]
fn a_current_handle_to_a_free_slot_says_free() {
    let mut kernel = kernel_at(0);
    let entity = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.free(entity).unwrap();
    assert_eq!(kernel.current_ref(entity.number()), Err(EntityRefError::Free));

    let rebuilt = EntityRef::from_parts(entity.number(), 1).unwrap();
    assert_eq!(kernel.resolve(rebuilt).map(|_| ()), Err(EntityRefError::Free));
}

#[test]
fn reserved_and_world_numbers_are_never_entities() {
    let kernel = kernel_at(0);
    for number in [0, FIRST - 1, WORLD, WORLD + 1, -1] {
        assert_eq!(kernel.current_ref(number), Err(EntityRefError::ReservedOrOutOfRange), "number {number}");
        assert_eq!(EntityRef::from_parts(number, 0), Err(EntityRefError::ReservedOrOutOfRange), "number {number}");
        assert_eq!(kernel.occupied_kind(number), None);
    }
    let handle = EntityRef::from_parts(FIRST, 7).unwrap();
    assert_eq!((handle.number(), handle.generation()), (FIRST, 7));
}

#[test]
fn a_number_above_the_high_water_mark_reads_as_free() {
    let kernel = kernel_at(0);
    assert_eq!(kernel.current_ref(FIRST + 50), Err(EntityRefError::Free));
    let guess = EntityRef::from_parts(FIRST + 50, 0).unwrap();
    assert_eq!(kernel.resolve(guess).map(|_| ()), Err(EntityRefError::Free));
}

#[test]
fn a_full_table_refuses_then_reuses_at_once() {
    let mut kernel = kernel_at(0);
    let handles: Vec<EntityRef> =
        (FIRST..WORLD).map(|_| kernel.allocate(EntityRunKind::General).unwrap()).collect();
    assert_eq!(handles.last().unwrap().number(), WORLD - 1);
    assert_eq!(kernel.allocate(EntityRunKind::General), Err(EntityAllocError::CapacityExhausted));

    kernel.free(handles[10]).unwrap();
    let back = kernel.allocate(EntityRunKind::Item).unwrap();
    assert_eq!((back.number(), back.generation()), (handles[10].number(), 1));
    assert_eq!(kernel.allocate(EntityRunKind::General), Err(EntityAllocError::CapacityExhausted));
}

#[test]
fn linking_is_remembered_per_entity() {
    let mut kernel = kernel_at(0);
    let a = kernel.allocate(EntityRunKind::General).unwrap();
    let b = kernel.allocate(EntityRunKind::General).unwrap();
    assert!(!kernel.resolve(a).unwrap().linked);
    kernel.set_linked(a, true).unwrap();
    assert!(kernel.resolve(a).unwrap().linked);
    assert!(!kernel.resolve(b).unwrap().linked);
    kernel.set_linked(a, false).unwrap();
    assert!(!kernel.resolve(a).unwrap().linked);
}

#[test]
fn a_new_entity_starts_with_no_relations_and_no_think_time() {
    let mut kernel = kernel_at(0);
    let entity = kernel.allocate(EntityRunKind::PrimaryLight).unwrap();
    let view = kernel.resolve(entity).unwrap();
    assert_eq!(view.entity, entity);
    assert_eq!(view.relations, EntityRelations::default());
    assert_eq!(view.relations.parent_tag, -1);
    assert_eq!(view.next_think_ms, None);
}

#[test]
fn occupied_numbers_filters_by_kind_and_skips_the_free() {
    let mut kernel = kernel_at(0);
    let item_a = kernel.allocate(EntityRunKind::Item).unwrap();
    let missile = kernel.allocate(EntityRunKind::Missile).unwrap();
    let item_b = kernel.allocate(EntityRunKind::Item).unwrap();
    let gone = kernel.allocate(EntityRunKind::Item).unwrap();
    kernel.free(gone).unwrap();

    let items = kernel.occupied_numbers(|kind| kind == EntityRunKind::Item);
    assert_eq!(items, [item_a.number(), item_b.number()]);
    assert_eq!(kernel.occupied_numbers(|kind| kind == EntityRunKind::Missile), [missile.number()]);
    assert_eq!(kernel.occupied_numbers(|_| true).len(), 3);
}

// ---- thinking ----

#[test]
fn an_entity_thinks_once_per_frame() {
    let mut kernel = kernel_at(0);
    let entity = kernel.allocate(EntityRunKind::Missile).unwrap();
    let run = ThinkEnter::Run { entity, kind: EntityRunKind::Missile, parent: None };

    // Allocation clears the "already thought" mark, so it may think in the frame it was made in.
    assert_eq!(kernel.begin_think(entity.number()), Ok(run));
    assert_eq!(kernel.begin_think(entity.number()), Ok(ThinkEnter::Skip));

    kernel.begin_frame(50);
    assert_eq!(kernel.begin_think(entity.number()), Ok(run));
    assert_eq!(kernel.begin_think(entity.number()), Ok(ThinkEnter::Skip));
}

#[test]
fn a_restored_kernel_makes_its_entities_wait_a_frame_at_serial_zero() {
    let snapshot = snapshot_of(vec![live_slot(EntityRunKind::General)], vec![]);
    let mut kernel = EntityKernel::from_snapshot(&snapshot).unwrap();
    assert_eq!(kernel.frame_serial(), 0);
    assert_eq!(kernel.begin_think(FIRST), Ok(ThinkEnter::Skip));
    kernel.begin_frame(10);
    assert!(matches!(kernel.begin_think(FIRST), Ok(ThinkEnter::Run { .. })));
}

#[test]
fn nothing_thinks_in_a_free_or_reserved_slot() {
    let mut kernel = kernel_at(0);
    let entity = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.free(entity).unwrap();
    kernel.begin_frame(10);
    for number in [entity.number(), FIRST + 40, FIRST - 1, 0, WORLD, WORLD + 5, -3] {
        assert_eq!(kernel.begin_think(number), Ok(ThinkEnter::Skip), "number {number}");
    }
}

#[test]
fn a_reused_number_thinks_again_in_the_frame_it_comes_back() {
    let mut kernel = kernel_at(0);
    let old = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.begin_frame(10);
    assert!(matches!(kernel.begin_think(old.number()), Ok(ThinkEnter::Run { .. })));
    kernel.free(old).unwrap();

    kernel.begin_frame(700);
    let new = kernel.allocate(EntityRunKind::Item).unwrap();
    assert_eq!(new.number(), old.number());
    // Allocation cleared the "already thought" mark the old life left behind.
    assert_eq!(kernel.begin_think(new.number()), Ok(ThinkEnter::Run { entity: new, kind: EntityRunKind::Item, parent: None }));
}

fn kernel_with_think_time(at: Option<i32>) -> EntityKernel {
    let mut slot = live_slot(EntityRunKind::General);
    slot.occupied.as_mut().unwrap().next_think_ms = at;
    EntityKernel::from_snapshot(&snapshot_of(vec![slot], vec![])).unwrap()
}

#[test]
fn a_scheduled_think_comes_due_once() {
    let mut kernel = kernel_with_think_time(Some(1000));
    let entity = kernel.current_ref(FIRST).unwrap();

    kernel.begin_frame(999);
    assert_eq!(kernel.take_due_think(entity), Ok(false));
    assert_eq!(kernel.resolve(entity).unwrap().next_think_ms, Some(1000));

    kernel.begin_frame(1000);
    assert_eq!(kernel.take_due_think(entity), Ok(true));
    assert_eq!(kernel.resolve(entity).unwrap().next_think_ms, None);
    assert_eq!(kernel.take_due_think(entity), Ok(false));
}

#[test]
fn a_think_time_of_zero_or_less_is_not_a_schedule() {
    for at in [0, -1, -5000] {
        let mut kernel = kernel_with_think_time(Some(at));
        let entity = kernel.current_ref(FIRST).unwrap();
        kernel.begin_frame(100_000);
        assert_eq!(kernel.take_due_think(entity), Ok(false), "think time {at}");
        assert_eq!(kernel.resolve(entity).unwrap().next_think_ms, Some(at));
    }
}

#[test]
fn begin_frame_moves_the_clock_and_the_serial() {
    let mut kernel = EntityKernel::default();
    assert_eq!((kernel.level_time_ms(), kernel.frame_serial()), (0, 0));
    kernel.begin_frame(250);
    kernel.begin_frame(300);
    assert_eq!((kernel.level_time_ms(), kernel.frame_serial()), (300, 2));
}

#[test]
fn the_frame_serial_wraps() {
    let mut snapshot = snapshot_of(vec![], vec![]);
    snapshot.frame_serial = u32::MAX;
    let mut kernel = EntityKernel::from_snapshot(&snapshot).unwrap();
    kernel.begin_frame(1);
    assert_eq!(kernel.frame_serial(), 0);
}

// ---- one-shot events ----

#[test]
fn an_event_lives_three_hundred_milliseconds() {
    let mut kernel = kernel_at(100);
    let event = kernel.allocate(EntityRunKind::TempEvent).unwrap();
    let keeper = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.mark_transient_event(event, 100).unwrap();

    kernel.begin_frame(400);
    assert!(kernel.expire_transient_events().is_empty());
    assert!(kernel.resolve(event).is_ok());

    kernel.begin_frame(401);
    assert_eq!(kernel.expire_transient_events(), [event]);
    assert_eq!(kernel.current_ref(event.number()), Err(EntityRefError::Free));
    assert!(kernel.resolve(keeper).is_ok());
    assert!(kernel.expire_transient_events().is_empty());
}

#[test]
fn several_expired_events_go_together_lowest_number_first() {
    let mut kernel = kernel_at(0);
    let a = kernel.allocate(EntityRunKind::TempEvent).unwrap();
    let b = kernel.allocate(EntityRunKind::TempEvent).unwrap();
    let late = kernel.allocate(EntityRunKind::TempEvent).unwrap();
    kernel.mark_transient_event(a, 0).unwrap();
    kernel.mark_transient_event(b, 10).unwrap();
    kernel.mark_transient_event(late, 900).unwrap();

    kernel.begin_frame(1000);
    assert_eq!(kernel.expire_transient_events(), [a, b]);
    assert!(kernel.resolve(late).is_ok());
}

// ---- links die with their target ----

#[test]
fn freeing_an_entity_cuts_every_link_to_it() {
    let a = nth(0);
    let c = nth(2);
    let mut b = live_slot(EntityRunKind::General);
    b.occupied.as_mut().unwrap().relations = EntityRelations {
        owner: Some(a),
        parent: Some(a),
        ground: Some(c),
        parent_tag: 4,
        parent_link_origin: [1.0, 2.0, 3.0],
        ..EntityRelations::default()
    };
    let snapshot = snapshot_of(
        vec![live_slot(EntityRunKind::Item), b, live_slot(EntityRunKind::General)],
        vec![],
    );
    let mut kernel = EntityKernel::from_snapshot(&snapshot).unwrap();

    kernel.free(a).unwrap();
    let relations = kernel.resolve(nth(1)).unwrap().relations;
    assert_eq!((relations.owner, relations.parent, relations.ground), (None, None, Some(c)));
    assert_eq!(relations.parent_tag, 4);
    assert_eq!(relations.parent_link_origin, [1.0, 2.0, 3.0]);
}

// ---- snapshots ----

#[test]
fn a_snapshot_round_trips() {
    let mut kernel = kernel_at(2000);
    let a = kernel.allocate(EntityRunKind::Item).unwrap();
    let b = kernel.allocate(EntityRunKind::TempEvent).unwrap();
    let c = kernel.allocate(EntityRunKind::General).unwrap();
    kernel.set_linked(c, true).unwrap();
    kernel.mark_transient_event(b, 1900).unwrap();
    kernel.free(a).unwrap();

    let snapshot = kernel.to_snapshot();
    assert_eq!(snapshot.high_water, FIRST + 3);
    assert_eq!(snapshot.free_fifo, [a.number()]);
    assert_eq!(snapshot.slots[0].generation, 1);
    assert_eq!(snapshot.slots[0].freed_at_ms, 2000);

    let restored = EntityKernel::from_snapshot(&snapshot).unwrap();
    assert_eq!(restored, kernel);
    assert_eq!(restored.to_snapshot(), snapshot);
}

#[test]
fn a_default_snapshot_is_an_empty_kernel() {
    let snapshot = EntityKernelSnapshot::default();
    assert_eq!(snapshot.validate(), Ok(()));
    assert_eq!(EntityKernel::from_snapshot(&snapshot).unwrap(), EntityKernel::default());
    assert_eq!(EntityKernel::default().to_snapshot(), snapshot);
}

#[test]
fn a_snapshot_answers_lookups_like_the_kernel() {
    let mut kernel = kernel_at(0);
    let a = kernel.allocate(EntityRunKind::Missile).unwrap();
    let b = kernel.allocate(EntityRunKind::Item).unwrap();
    kernel.free(a).unwrap();
    let snapshot = kernel.to_snapshot();

    for number in [0, FIRST - 1, FIRST, FIRST + 1, WORLD] {
        assert_eq!(snapshot.current_ref(number), kernel.current_ref(number), "number {number}");
        assert_eq!(snapshot.occupied_kind(number), kernel.occupied_kind(number), "number {number}");
    }
    assert_eq!(snapshot.current_ref(b.number()), Ok(b));
}

#[test]
fn a_snapshot_with_a_missing_slot_is_malformed_not_a_panic() {
    let mut snapshot = snapshot_of(vec![live_slot(EntityRunKind::General)], vec![]);
    snapshot.slots.clear();
    assert_eq!(snapshot.current_ref(FIRST), Err(EntityRefError::SnapshotMalformed));
    assert_eq!(snapshot.occupied_kind(FIRST), None);
}

#[test]
fn a_snapshot_may_fill_the_whole_table() {
    let slots = (FIRST..WORLD).map(|_| live_slot(EntityRunKind::General)).collect();
    let snapshot = snapshot_of(slots, vec![]);
    assert_eq!(snapshot.high_water, WORLD);
    assert_eq!(snapshot.validate(), Ok(()));
    let mut kernel = EntityKernel::from_snapshot(&snapshot).unwrap();
    assert_eq!(kernel.allocate(EntityRunKind::General), Err(EntityAllocError::CapacityExhausted));
}

#[test]
fn validate_checks_the_high_water_mark() {
    for high_water in [FIRST - 1, 0, WORLD + 1] {
        let snapshot = EntityKernelSnapshot { high_water, ..Default::default() };
        assert_eq!(snapshot.validate(), Err(EntityKernelSnapshotError::HighWaterOutOfRange), "mark {high_water}");
        assert!(EntityKernel::from_snapshot(&snapshot).is_err());
    }
}

#[test]
fn validate_checks_the_slot_count() {
    let mut snapshot = snapshot_of(vec![live_slot(EntityRunKind::General)], vec![]);
    snapshot.high_water += 1;
    assert_eq!(snapshot.validate(), Err(EntityKernelSnapshotError::SlotCountMismatch));
    snapshot.high_water -= 2;
    assert_eq!(snapshot.validate(), Err(EntityKernelSnapshotError::SlotCountMismatch));
}

#[test]
fn validate_checks_the_free_list() {
    use EntityKernelSnapshotError::FreeFifoInvalid;
    let general = || live_slot(EntityRunKind::General);

    // A held number on the free list.
    assert_eq!(snapshot_of(vec![general()], vec![FIRST]).validate(), Err(FreeFifoInvalid));
    // A free slot nobody queued.
    assert_eq!(snapshot_of(vec![free_slot()], vec![]).validate(), Err(FreeFifoInvalid));
    // The same number queued twice.
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST, FIRST]).validate(), Err(FreeFifoInvalid));
    // Numbers the snapshot does not cover.
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST, FIRST + 9]).validate(), Err(FreeFifoInvalid));
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST - 1]).validate(), Err(FreeFifoInvalid));
    // The right answer.
    assert_eq!(snapshot_of(vec![free_slot(), general()], vec![FIRST]).validate(), Ok(()));
}

#[test]
fn validate_checks_every_link_points_at_a_live_entity() {
    use EntityKernelSnapshotError::RelationInvalid;

    let linked_to = |target: EntityRef, slot_of_target: EntityKernelSlotSnapshot, fifo: Vec<i32>| {
        let mut holder = live_slot(EntityRunKind::General);
        holder.occupied.as_mut().unwrap().relations.ground = Some(target);
        snapshot_of(vec![holder, slot_of_target], fifo).validate()
    };

    // Live and the same generation: fine.
    assert_eq!(linked_to(nth(1), live_slot(EntityRunKind::Item), vec![]), Ok(()));
    // A free slot of the same generation.
    let vacated = EntityKernelSlotSnapshot { generation: 0, ..free_slot() };
    assert_eq!(linked_to(nth(1), vacated, vec![FIRST + 1]), Err(RelationInvalid));
    // A live slot, but a different generation.
    assert_eq!(linked_to(EntityRef::new(FIRST + 1, 9), live_slot(EntityRunKind::Item), vec![]), Err(RelationInvalid));
    // A number past the end.
    assert_eq!(linked_to(nth(7), live_slot(EntityRunKind::Item), vec![]), Err(RelationInvalid));
}

#[test]
fn every_link_kind_is_validated() {
    for which in 0..3 {
        let mut holder = live_slot(EntityRunKind::General);
        let dangling = Some(nth(5));
        let relations = &mut holder.occupied.as_mut().unwrap().relations;
        match which {
            0 => relations.owner = dangling,
            1 => relations.parent = dangling,
            _ => relations.ground = dangling,
        }
        assert_eq!(
            snapshot_of(vec![holder], vec![]).validate(),
            Err(EntityKernelSnapshotError::RelationInvalid),
            "link {which}"
        );
    }
}

// ---- state builders ----

#[test]
fn a_script_mover_starts_parked_where_it_was_put() {
    let state = init_script_mover_state(FIRST + 4, [1.0, 2.0, 3.0], [0.0, 90.0, 0.0]);
    assert_eq!((state.number, state.e_type), (FIRST + 4, et::SCRIPTMOVER));
    assert_eq!(state.lerp.pos.tr_type, tr_type::STATIONARY);
    assert_eq!(state.lerp.pos.tr_base, [1.0, 2.0, 3.0]);
    assert_eq!(state.lerp.apos.tr_type, tr_type::STATIONARY);
    assert_eq!(state.lerp.apos.tr_base, [0.0, 90.0, 0.0]);
    assert_eq!(pos_from_entity_state(&state), state.lerp.pos);
    assert_eq!(apos_from_entity_state(&state), state.lerp.apos);
}

fn moving(base: [f32; 3], delta: [f32; 3], since: i32) -> Trajectory {
    Trajectory { tr_type: tr_type::LINEAR, tr_time: since, tr_base: base, tr_delta: delta, ..Trajectory::default() }
}

#[test]
fn a_missile_carries_its_weapon_trajectories_and_launch_time() {
    let pos = moving([0.0; 3], [100.0, 0.0, 0.0], 40);
    let apos = moving([0.0; 3], [0.0, 10.0, 0.0], 40);
    let state = init_missile_state(FIRST + 1, 17, pos, apos, 1234);
    assert_eq!((state.number, state.e_type), (FIRST + 1, et::MISSILE));
    assert_eq!(state.weapon, playerstate_t6::Weapon(17));
    assert_eq!(state.lerp.e_flags, 0);
    assert_eq!((state.lerp.pos, state.lerp.apos), (pos, apos));
    assert_eq!(state.lerp.u.missile().launch_time, 1234);
}

#[test]
fn an_item_remembers_who_dropped_it() {
    let pos = moving([5.0; 3], [0.0; 3], 0);
    let state = init_item_state(FIRST + 2, 9, pos, Trajectory::default(), 6);
    assert_eq!((state.number, state.e_type), (FIRST + 2, et::ITEM));
    assert_eq!(state.weapon, playerstate_t6::Weapon(9));
    assert_eq!(state.client_num, 6);
    assert_eq!(state.lerp.pos, pos);
    assert_eq!(state.lerp.apos, Trajectory::default());
}

#[test]
fn spinning_a_parked_mover_starts_from_its_resting_angles() {
    let parked = Trajectory { tr_base: [0.0, 45.0, 0.0], ..Trajectory::default() };
    let spin = rotate_velocity_apos(&parked, [0.0, 90.0, 0.0], 2.5, 3000);
    assert_eq!(spin.tr_type, tr_type::LINEAR_STOP);
    assert_eq!((spin.tr_time, spin.tr_duration), (3000, 2500));
    assert_eq!(spin.tr_base, [0.0, 45.0, 0.0]);
    assert_eq!(spin.tr_delta, [0.0, 90.0, 0.0]);
}

#[test]
fn spinning_a_turning_mover_starts_from_where_it_has_got_to() {
    let turning = moving([10.0, 0.0, 0.0], [100.0, 0.0, 0.0], 1000);
    let spin = rotate_velocity_apos(&turning, [0.0, 0.0, 30.0], 1.0, 1500);
    assert!(near(spin.tr_base, [60.0, 0.0, 0.0]), "{:?}", spin.tr_base);
    assert_eq!((spin.tr_time, spin.tr_duration), (1500, 1000));
}

#[test]
fn beginning_a_spin_changes_only_the_angles() {
    let mut state = init_script_mover_state(FIRST, [7.0, 8.0, 9.0], [0.0, 10.0, 0.0]);
    let before = state;
    begin_script_mover_rotate_velocity(&mut state, [0.0, 180.0, 0.0], 0.5, 200);
    assert_eq!(state.lerp.pos, before.lerp.pos);
    assert_eq!(state.lerp.apos.tr_type, tr_type::LINEAR_STOP);
    assert_eq!(state.lerp.apos.tr_duration, 500);
    assert_eq!(state.lerp.apos.tr_base, [0.0, 10.0, 0.0]);
    assert_eq!(state.number, before.number);
}

#[test]
fn a_script_mover_gentity_defaults_to_nothing() {
    let mover = ScriptMoverGentity::default();
    assert_eq!(mover.id, crate::identities::ScriptModelId::from_wire(0));
    assert!(!mover.nonsolid);
    assert_eq!(mover.shown_to, 0);
    assert_eq!(mover.state, entity_t6::EntityState::default());
}

// ---- phases ----

#[test]
fn the_phases_run_in_their_declared_order() {
    assert_eq!(KERNEL_PHASE_ORDER.len(), 9);
    assert!(KERNEL_PHASE_ORDER.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(KERNEL_PHASE_ORDER.first(), Some(&KernelPhase::AdvanceTime));
    assert_eq!(KERNEL_PHASE_ORDER.last(), Some(&KernelPhase::PublishSnapshot));
    assert_eq!(gentity_spawn_base(), GENTITY_RESERVED_COUNT);
}

// ---- use presses ----

const OTHER_BUTTON: u32 = 0x1;

fn press(client: u32, edge: bool) -> UsePress {
    UsePress { client, held: true, edge }
}

#[test]
fn use_gives_an_edge_only_on_the_first_command_it_is_down() {
    let cmds = [(3, buttons::USE), (3, buttons::USE), (3, OTHER_BUTTON), (3, buttons::USE)];
    assert_eq!(collect_use_presses(&cmds, &[]), [press(3, true), press(3, false), press(3, true)]);
}

#[test]
fn use_held_from_the_last_frame_is_not_a_new_press() {
    assert_eq!(collect_use_presses(&[(1, buttons::USE)], &[(1, buttons::USE)]), [press(1, false)]);
    assert_eq!(collect_use_presses(&[(1, buttons::USE)], &[(1, OTHER_BUTTON)]), [press(1, true)]);
}

#[test]
fn the_reload_use_button_counts_as_use() {
    let cmds = [(2, buttons::USE_RELOAD), (2, buttons::USE_RELOAD | OTHER_BUTTON), (2, buttons::USE)];
    assert_eq!(collect_use_presses(&cmds, &[]), [press(2, true), press(2, false), press(2, false)]);
}

#[test]
fn commands_without_use_give_no_press() {
    assert!(collect_use_presses(&[(1, OTHER_BUTTON), (2, 0)], &[(1, buttons::USE)]).is_empty());
    assert!(collect_use_presses(&[], &[(1, buttons::USE)]).is_empty());
}

#[test]
fn clients_are_tracked_separately() {
    let cmds = [(1, buttons::USE), (2, buttons::USE), (1, buttons::USE), (2, 0), (2, buttons::USE)];
    let old = [(2, buttons::USE)];
    assert_eq!(
        collect_use_presses(&cmds, &old),
        [press(1, true), press(2, false), press(1, false), press(2, true)]
    );
}

#[test]
fn the_first_old_entry_for_a_client_wins() {
    let old = [(5, buttons::USE), (5, 0)];
    assert_eq!(collect_use_presses(&[(5, buttons::USE)], &old), [press(5, false)]);
    let old = [(5, 0), (5, buttons::USE)];
    assert_eq!(collect_use_presses(&[(5, buttons::USE)], &old), [press(5, true)]);
}
