use entity_t6::{Trajectory, et, flight::path_kind};
use playerstate_t6::game::buttons;
use trace_t6::WORLD_ENTITY_SLOT;

use super::*;

const FIRST: i32 = PROTECTED_ENTITY_SLOTS;
const WORLD: i32 = WORLD_ENTITY_SLOT as i32;

fn kernel_at(ms: i32) -> EntitySlotArena {
    let mut kernel = EntitySlotArena::default();
    kernel.open_tick(ms);
    kernel
}

/// The handle a snapshot-built kernel gives the `n`th number (generation 0).
fn nth(n: i32) -> EntHandle {
    EntHandle::new(FIRST + n, 0)
}

fn held(kind: EntityBehaviorClass) -> BayHeldRow {
    let linked = false;
    BayHeldRow {
        kind,
        linked,
        relations: Default::default(),
        transient_event_time_ms: Option::default(),
        next_think_ms: Option::default(),
    }
}

fn live_slot(kind: EntityBehaviorClass) -> BayFrozenRow {
    BayFrozenRow { generation: 0, occupied: Some(held(kind)), freed_at_ms: i32::MIN }
}

fn free_slot() -> BayFrozenRow {
    BayFrozenRow { generation: 1, occupied: None, freed_at_ms: 0 }
}

fn snapshot_of(slots: Vec<BayFrozenRow>, free_fifo: Vec<i32>) -> FrozenSlotTable {
    FrozenSlotTable { high_water: FIRST + slots.len() as i32, slots, free_fifo, ..Default::default() }
}

fn near(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

// ---- numbers and generations ----

#[test]
fn numbers_are_handed_out_upward_from_the_reserved_block() {
    let mut kernel = kernel_at(0);
    let first = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    let second = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    let third = kernel.claim_slot(EntityBehaviorClass::Missile).unwrap();
    assert_eq!([first.slot(), second.slot(), third.slot()], [FIRST, FIRST + 1, FIRST + 2]);
    assert_eq!(first.generation(), 0);
    assert_eq!(kernel.high_water(), FIRST + 3);
    assert_eq!(kernel.holder_class(FIRST + 1), Some(EntityBehaviorClass::Item));
}

#[test]
fn a_fresh_kernel_has_nothing_held() {
    let kernel = EntitySlotArena::default();
    assert_eq!(kernel.high_water(), FIRST);
    assert_eq!(kernel.ref_for(FIRST), Err(EntityHandleFault::Free));
    assert!(kernel.held_slots(|_| true).is_empty());
}

#[test]
fn a_freed_number_rests_for_half_a_second() {
    let mut kernel = kernel_at(1000);
    let old = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.release_slot(old).unwrap();

    kernel.open_tick(1499);
    assert_eq!(kernel.claim_slot(EntityBehaviorClass::General).unwrap().slot(), FIRST + 1);

    kernel.open_tick(1500);
    let again = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    assert_eq!((again.slot(), again.generation()), (FIRST, 1));
}

#[test]
fn freed_numbers_come_back_in_the_order_they_were_freed() {
    let mut kernel = kernel_at(0);
    let a = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    let b = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    let c = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.release_slot(c).unwrap();
    kernel.release_slot(a).unwrap();
    kernel.release_slot(b).unwrap();

    kernel.open_tick(10_000);
    let order: Vec<i32> = (0..3).map(|_| kernel.claim_slot(EntityBehaviorClass::General).unwrap().slot()).collect();
    assert_eq!(order, [c.slot(), a.slot(), b.slot()]);
}

#[test]
fn an_old_handle_stops_working_once_the_number_is_reused() {
    let mut kernel = kernel_at(0);
    let old = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    kernel.release_slot(old).unwrap();
    kernel.open_tick(600);
    let new = kernel.claim_slot(EntityBehaviorClass::Missile).unwrap();
    assert_eq!(new.slot(), old.slot());

    let stale = Err(EntityHandleFault::Outlived { current: 1 });
    assert_eq!(kernel.look_up(old).map(|_| ()), stale);
    assert_eq!(kernel.mark_linked(old, true), stale);
    assert_eq!(kernel.mark_transient_event(old, 5), stale);
    assert_eq!(kernel.pop_due_think(old), stale.map(|()| false));
    assert_eq!(kernel.release_slot(old), stale);
    assert_eq!(kernel.look_up(new).unwrap().class, EntityBehaviorClass::Missile);
}

#[test]
fn freeing_twice_is_refused() {
    let mut kernel = kernel_at(0);
    let entity = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    assert_eq!(kernel.release_slot(entity), Ok(()));
    assert_eq!(kernel.release_slot(entity), Err(EntityHandleFault::Outlived { current: 1 }));
}

#[test]
fn a_current_handle_to_a_free_slot_says_free() {
    let mut kernel = kernel_at(0);
    let entity = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.release_slot(entity).unwrap();
    assert_eq!(kernel.ref_for(entity.slot()), Err(EntityHandleFault::Free));

    let rebuilt = EntHandle::from_parts(entity.slot(), 1).unwrap();
    assert_eq!(kernel.look_up(rebuilt).map(|_| ()), Err(EntityHandleFault::Free));
}

#[test]
fn reserved_and_world_numbers_are_never_entities() {
    let kernel = kernel_at(0);
    for number in [0, FIRST - 1, WORLD, WORLD + 1, -1] {
        assert_eq!(kernel.ref_for(number), Err(EntityHandleFault::NumberNotUsable), "number {number}");
        assert_eq!(EntHandle::from_parts(number, 0), Err(EntityHandleFault::NumberNotUsable), "number {number}");
        assert_eq!(kernel.holder_class(number), None);
    }
    let handle = EntHandle::from_parts(FIRST, 7).unwrap();
    assert_eq!((handle.slot(), handle.generation()), (FIRST, 7));
}

#[test]
fn a_number_above_the_high_water_mark_reads_as_free() {
    let kernel = kernel_at(0);
    assert_eq!(kernel.ref_for(FIRST + 50), Err(EntityHandleFault::Free));
    let guess = EntHandle::from_parts(FIRST + 50, 0).unwrap();
    assert_eq!(kernel.look_up(guess).map(|_| ()), Err(EntityHandleFault::Free));
}

#[test]
fn a_full_table_refuses_then_reuses_at_once() {
    let mut kernel = kernel_at(0);
    let handles: Vec<EntHandle> =
        (FIRST..WORLD).map(|_| kernel.claim_slot(EntityBehaviorClass::General).unwrap()).collect();
    assert_eq!(handles.last().unwrap().slot(), WORLD - 1);
    assert_eq!(kernel.claim_slot(EntityBehaviorClass::General), Err(EntitySlotShortage::TableFull));

    kernel.release_slot(handles[10]).unwrap();
    let back = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    assert_eq!((back.slot(), back.generation()), (handles[10].slot(), 1));
    assert_eq!(kernel.claim_slot(EntityBehaviorClass::General), Err(EntitySlotShortage::TableFull));
}

#[test]
fn linking_is_remembered_per_entity() {
    let mut kernel = kernel_at(0);
    let a = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    let b = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    assert!(!kernel.look_up(a).unwrap().in_world);
    kernel.mark_linked(a, true).unwrap();
    assert!(kernel.look_up(a).unwrap().in_world);
    assert!(!kernel.look_up(b).unwrap().in_world);
    kernel.mark_linked(a, false).unwrap();
    assert!(!kernel.look_up(a).unwrap().in_world);
}

#[test]
fn a_new_entity_starts_with_no_relations_and_no_think_time() {
    let mut kernel = kernel_at(0);
    let entity = kernel.claim_slot(EntityBehaviorClass::MainLightSource).unwrap();
    let view = kernel.look_up(entity).unwrap();
    assert_eq!(view.handle, entity);
    assert_eq!(view.ties, ThingTies::default());
    assert_eq!(view.ties.parent_tag, -1);
    assert_eq!(view.think_at_ms, None);
}

#[test]
fn occupied_numbers_filters_by_kind_and_skips_the_free() {
    let mut kernel = kernel_at(0);
    let item_a = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    let missile = kernel.claim_slot(EntityBehaviorClass::Missile).unwrap();
    let item_b = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    let gone = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    kernel.release_slot(gone).unwrap();

    let items = kernel.held_slots(|kind| kind == EntityBehaviorClass::Item);
    assert_eq!(items, [item_a.slot(), item_b.slot()]);
    assert_eq!(kernel.held_slots(|kind| kind == EntityBehaviorClass::Missile), [missile.slot()]);
    assert_eq!(kernel.held_slots(|_| true).len(), 3);
}

// ---- thinking ----

#[test]
fn an_entity_thinks_once_per_frame() {
    let mut kernel = kernel_at(0);
    let entity = kernel.claim_slot(EntityBehaviorClass::Missile).unwrap();
    let run = ThinkEntryResult::Run { entity, kind: EntityBehaviorClass::Missile, parent: None };

    // Allocation clears the "already thought" mark, so it may think in the frame it was made in.
    assert_eq!(kernel.open_think(entity.slot()), Ok(run));
    assert_eq!(kernel.open_think(entity.slot()), Ok(ThinkEntryResult::Skip));

    kernel.open_tick(50);
    assert_eq!(kernel.open_think(entity.slot()), Ok(run));
    assert_eq!(kernel.open_think(entity.slot()), Ok(ThinkEntryResult::Skip));
}

#[test]
fn a_restored_kernel_makes_its_entities_wait_a_frame_at_serial_zero() {
    let snapshot = snapshot_of(vec![live_slot(EntityBehaviorClass::General)], vec![]);
    let mut kernel = EntitySlotArena::from_world_state(&snapshot).unwrap();
    assert_eq!(kernel.frame_serial(), 0);
    assert_eq!(kernel.open_think(FIRST), Ok(ThinkEntryResult::Skip));
    kernel.open_tick(10);
    assert!(matches!(kernel.open_think(FIRST), Ok(ThinkEntryResult::Run { .. })));
}

#[test]
fn nothing_thinks_in_a_free_or_reserved_slot() {
    let mut kernel = kernel_at(0);
    let entity = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.release_slot(entity).unwrap();
    kernel.open_tick(10);
    for number in [entity.slot(), FIRST + 40, FIRST - 1, 0, WORLD, WORLD + 5, -3] {
        assert_eq!(kernel.open_think(number), Ok(ThinkEntryResult::Skip), "number {number}");
    }
}

#[test]
fn a_reused_number_thinks_again_in_the_frame_it_comes_back() {
    let mut kernel = kernel_at(0);
    let old = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.open_tick(10);
    assert!(matches!(kernel.open_think(old.slot()), Ok(ThinkEntryResult::Run { .. })));
    kernel.release_slot(old).unwrap();

    kernel.open_tick(700);
    let new = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    assert_eq!(new.slot(), old.slot());
    // Allocation cleared the "already thought" mark the old life left behind.
    assert_eq!(kernel.open_think(new.slot()), Ok(ThinkEntryResult::Run { entity: new, kind: EntityBehaviorClass::Item, parent: None }));
}

fn kernel_with_think_time(at: Option<i32>) -> EntitySlotArena {
    let mut slot = live_slot(EntityBehaviorClass::General);
    slot.occupied.as_mut().unwrap().next_think_ms = at;
    EntitySlotArena::from_world_state(&snapshot_of(vec![slot], vec![])).unwrap()
}

#[test]
fn a_scheduled_think_comes_due_once() {
    let mut kernel = kernel_with_think_time(Some(1000));
    let entity = kernel.ref_for(FIRST).unwrap();

    kernel.open_tick(999);
    assert_eq!(kernel.pop_due_think(entity), Ok(false));
    assert_eq!(kernel.look_up(entity).unwrap().think_at_ms, Some(1000));

    kernel.open_tick(1000);
    assert_eq!(kernel.pop_due_think(entity), Ok(true));
    assert_eq!(kernel.look_up(entity).unwrap().think_at_ms, None);
    assert_eq!(kernel.pop_due_think(entity), Ok(false));
}

#[test]
fn a_think_time_of_zero_or_less_is_not_a_schedule() {
    for at in [0, -1, -5000] {
        let mut kernel = kernel_with_think_time(Some(at));
        let entity = kernel.ref_for(FIRST).unwrap();
        kernel.open_tick(100_000);
        assert_eq!(kernel.pop_due_think(entity), Ok(false), "think time {at}");
        assert_eq!(kernel.look_up(entity).unwrap().think_at_ms, Some(at));
    }
}

#[test]
fn begin_frame_moves_the_clock_and_the_serial() {
    let mut kernel = EntitySlotArena::default();
    assert_eq!((kernel.level_time_ms(), kernel.frame_serial()), (0, 0));
    kernel.open_tick(250);
    kernel.open_tick(300);
    assert_eq!((kernel.level_time_ms(), kernel.frame_serial()), (300, 2));
}

#[test]
fn the_frame_serial_wraps() {
    let mut snapshot = snapshot_of(vec![], vec![]);
    snapshot.frame_serial = u32::MAX;
    let mut kernel = EntitySlotArena::from_world_state(&snapshot).unwrap();
    kernel.open_tick(1);
    assert_eq!(kernel.frame_serial(), 0);
}

// ---- one-shot events ----

#[test]
fn an_event_lives_three_hundred_milliseconds() {
    let mut kernel = kernel_at(100);
    let event = kernel.claim_slot(EntityBehaviorClass::TempEvent).unwrap();
    let keeper = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.mark_transient_event(event, 100).unwrap();

    kernel.open_tick(400);
    assert!(kernel.drop_old_events().is_empty());
    assert!(kernel.look_up(event).is_ok());

    kernel.open_tick(401);
    assert_eq!(kernel.drop_old_events(), [event]);
    assert_eq!(kernel.ref_for(event.slot()), Err(EntityHandleFault::Free));
    assert!(kernel.look_up(keeper).is_ok());
    assert!(kernel.drop_old_events().is_empty());
}

#[test]
fn several_expired_events_go_together_lowest_number_first() {
    let mut kernel = kernel_at(0);
    let a = kernel.claim_slot(EntityBehaviorClass::TempEvent).unwrap();
    let b = kernel.claim_slot(EntityBehaviorClass::TempEvent).unwrap();
    let late = kernel.claim_slot(EntityBehaviorClass::TempEvent).unwrap();
    kernel.mark_transient_event(a, 0).unwrap();
    kernel.mark_transient_event(b, 10).unwrap();
    kernel.mark_transient_event(late, 900).unwrap();

    kernel.open_tick(1000);
    assert_eq!(kernel.drop_old_events(), [a, b]);
    assert!(kernel.look_up(late).is_ok());
}

// ---- links die with their target ----

#[test]
fn freeing_an_entity_cuts_every_link_to_it() {
    let a = nth(0);
    let c = nth(2);
    let mut b = live_slot(EntityBehaviorClass::General);
    b.occupied.as_mut().unwrap().relations = ThingTies {
        owner: Some(a),
        parent: Some(a),
        ground: Some(c),
        parent_tag: 4,
        parent_link_origin: [1.0, 2.0, 3.0],
        ..ThingTies::default()
    };
    let snapshot = snapshot_of(
        vec![live_slot(EntityBehaviorClass::Item), b, live_slot(EntityBehaviorClass::General)],
        vec![],
    );
    let mut kernel = EntitySlotArena::from_world_state(&snapshot).unwrap();

    kernel.release_slot(a).unwrap();
    let relations = kernel.look_up(nth(1)).unwrap().ties;
    assert_eq!((relations.owner, relations.parent, relations.ground), (None, None, Some(c)));
    assert_eq!(relations.parent_tag, 4);
    assert_eq!(relations.parent_link_origin, [1.0, 2.0, 3.0]);
}

// ---- snapshots ----

#[test]
fn a_snapshot_round_trips() {
    let mut kernel = kernel_at(2000);
    let a = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    let b = kernel.claim_slot(EntityBehaviorClass::TempEvent).unwrap();
    let c = kernel.claim_slot(EntityBehaviorClass::General).unwrap();
    kernel.mark_linked(c, true).unwrap();
    kernel.mark_transient_event(b, 1900).unwrap();
    kernel.release_slot(a).unwrap();

    let snapshot = kernel.export_snapshot();
    assert_eq!(snapshot.high_water, FIRST + 3);
    assert_eq!(snapshot.free_fifo, [a.slot()]);
    assert_eq!(snapshot.slots[0].generation, 1);
    assert_eq!(snapshot.slots[0].freed_at_ms, 2000);

    let restored = EntitySlotArena::from_world_state(&snapshot).unwrap();
    assert_eq!(restored, kernel);
    assert_eq!(restored.export_snapshot(), snapshot);
}

#[test]
fn a_default_snapshot_is_an_empty_kernel() {
    let snapshot = FrozenSlotTable::default();
    assert_eq!(snapshot.validate(), Ok(()));
    assert_eq!(EntitySlotArena::from_world_state(&snapshot).unwrap(), EntitySlotArena::default());
    assert_eq!(EntitySlotArena::default().export_snapshot(), snapshot);
}

#[test]
fn a_snapshot_answers_lookups_like_the_kernel() {
    let mut kernel = kernel_at(0);
    let a = kernel.claim_slot(EntityBehaviorClass::Missile).unwrap();
    let b = kernel.claim_slot(EntityBehaviorClass::Item).unwrap();
    kernel.release_slot(a).unwrap();
    let snapshot = kernel.export_snapshot();

    for number in [0, FIRST - 1, FIRST, FIRST + 1, WORLD] {
        assert_eq!(snapshot.ref_for(number), kernel.ref_for(number), "number {number}");
        assert_eq!(snapshot.holder_class(number), kernel.holder_class(number), "number {number}");
    }
    assert_eq!(snapshot.ref_for(b.slot()), Ok(b));
}

#[test]
fn a_snapshot_with_a_missing_slot_is_malformed_not_a_panic() {
    let mut snapshot = snapshot_of(vec![live_slot(EntityBehaviorClass::General)], vec![]);
    snapshot.slots.clear();
    assert_eq!(snapshot.ref_for(FIRST), Err(EntityHandleFault::BadSnapshot));
    assert_eq!(snapshot.holder_class(FIRST), None);
}

#[test]
fn a_snapshot_may_fill_the_whole_table() {
    let slots = (FIRST..WORLD).map(|_| live_slot(EntityBehaviorClass::General)).collect();
    let snapshot = snapshot_of(slots, vec![]);
    assert_eq!(snapshot.high_water, WORLD);
    assert_eq!(snapshot.validate(), Ok(()));
    let mut kernel = EntitySlotArena::from_world_state(&snapshot).unwrap();
    assert_eq!(kernel.claim_slot(EntityBehaviorClass::General), Err(EntitySlotShortage::TableFull));
}

#[test]
fn validate_checks_the_high_water_mark() {
    for peak_slot in [FIRST - 1, 0, WORLD + 1] {
        let snapshot = FrozenSlotTable { high_water: peak_slot, ..Default::default() };
        assert_eq!(snapshot.validate(), Err(EntityTableSnapshotFault::TopMarkOffTable), "mark {peak_slot}");
        assert!(EntitySlotArena::from_world_state(&snapshot).is_err());
    }
}

#[test]
fn validate_checks_the_slot_count() {
    let mut snapshot = snapshot_of(vec![live_slot(EntityBehaviorClass::General)], vec![]);
    snapshot.high_water += 1;
    assert_eq!(snapshot.validate(), Err(EntityTableSnapshotFault::WrongSlotCount));
    snapshot.high_water -= 2;
    assert_eq!(snapshot.validate(), Err(EntityTableSnapshotFault::WrongSlotCount));
}

#[test]
fn validate_checks_the_free_list() {
    use EntityTableSnapshotFault::FreeQueueBroken;
    let general = || live_slot(EntityBehaviorClass::General);

    // A held number on the free list.
    assert_eq!(snapshot_of(vec![general()], vec![FIRST]).validate(), Err(FreeQueueBroken));
    // A free slot nobody queued.
    assert_eq!(snapshot_of(vec![free_slot()], vec![]).validate(), Err(FreeQueueBroken));
    // The same number queued twice.
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST, FIRST]).validate(), Err(FreeQueueBroken));
    // Numbers the snapshot does not cover.
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST, FIRST + 9]).validate(), Err(FreeQueueBroken));
    assert_eq!(snapshot_of(vec![free_slot()], vec![FIRST - 1]).validate(), Err(FreeQueueBroken));
    // The right answer.
    assert_eq!(snapshot_of(vec![free_slot(), general()], vec![FIRST]).validate(), Ok(()));
}

#[test]
fn validate_checks_every_link_points_at_a_live_entity() {
    use EntityTableSnapshotFault::BadLink;

    let linked_to = |target: EntHandle, slot_of_target: BayFrozenRow, fifo: Vec<i32>| {
        let mut holder = live_slot(EntityBehaviorClass::General);
        holder.occupied.as_mut().unwrap().relations.ground = Some(target);
        snapshot_of(vec![holder, slot_of_target], fifo).validate()
    };

    // Live and the same generation: fine.
    assert_eq!(linked_to(nth(1), live_slot(EntityBehaviorClass::Item), vec![]), Ok(()));
    // A free slot of the same generation.
    let vacated = BayFrozenRow { generation: 0, ..free_slot() };
    assert_eq!(linked_to(nth(1), vacated, vec![FIRST + 1]), Err(BadLink));
    // A live slot, but a different generation.
    assert_eq!(linked_to(EntHandle::new(FIRST + 1, 9), live_slot(EntityBehaviorClass::Item), vec![]), Err(BadLink));
    // A number past the end.
    assert_eq!(linked_to(nth(7), live_slot(EntityBehaviorClass::Item), vec![]), Err(BadLink));
}

#[test]
fn every_link_kind_is_validated() {
    for which in 0..3 {
        let mut holder = live_slot(EntityBehaviorClass::General);
        let dangling = Some(nth(5));
        let relations = &mut holder.occupied.as_mut().unwrap().relations;
        match which {
            0 => relations.owner = dangling,
            1 => relations.parent = dangling,
            _ => relations.ground = dangling,
        }
        assert_eq!(
            snapshot_of(vec![holder], vec![]).validate(),
            Err(EntityTableSnapshotFault::BadLink),
            "link {which}"
        );
    }
}

// ---- state builders ----

#[test]
fn a_script_mover_starts_parked_where_it_was_put() {
    let state = build_scripted_mover(FIRST + 4, [1.0, 2.0, 3.0], [0.0, 90.0, 0.0]);
    assert_eq!((state.slot_index, state.e_type), (FIRST + 4, et::SCRIPTMOVER));
    assert_eq!(state.lerp.pos.tr_type, path_kind::AT_REST);
    assert_eq!(state.lerp.pos.base_point, [1.0, 2.0, 3.0]);
    assert_eq!(state.lerp.apos.tr_type, path_kind::AT_REST);
    assert_eq!(state.lerp.apos.base_point, [0.0, 90.0, 0.0]);
    assert_eq!(move_path_of(&state), state.lerp.pos);
    assert_eq!(turn_path_of(&state), state.lerp.apos);
}

fn moving(base: [f32; 3], shift: [f32; 3], since: i32) -> Trajectory {
    Trajectory { tr_type: path_kind::STRAIGHT, began_at: since, base_point: base, rate: shift, ..Trajectory::default() }
}

#[test]
fn a_missile_carries_its_weapon_trajectories_and_launch_time() {
    let pos = moving([0.0; 3], [100.0, 0.0, 0.0], 40);
    let apos = moving([0.0; 3], [0.0, 10.0, 0.0], 40);
    let state = shell_state_at(FIRST + 1, 17, pos, apos, 1234);
    assert_eq!((state.slot_index, state.e_type), (FIRST + 1, et::MISSILE));
    assert_eq!(state.weapon, playerstate_t6::Weapon(17));
    assert_eq!(state.lerp.e_flags, 0);
    assert_eq!((state.lerp.pos, state.lerp.apos), (pos, apos));
    assert_eq!(state.lerp.u.missile().spawn_moment, 1234);
}

#[test]
fn an_item_remembers_who_dropped_it() {
    let pos = moving([5.0; 3], [0.0; 3], 0);
    let state = loot_state_at(FIRST + 2, 9, pos, Trajectory::default(), 6);
    assert_eq!((state.slot_index, state.e_type), (FIRST + 2, et::ITEM));
    assert_eq!(state.weapon, playerstate_t6::Weapon(9));
    assert_eq!(state.client_num, 6);
    assert_eq!(state.lerp.pos, pos);
    assert_eq!(state.lerp.apos, Trajectory::default());
}

#[test]
fn spinning_a_parked_mover_starts_from_its_resting_angles() {
    let parked = Trajectory { base_point: [0.0, 45.0, 0.0], ..Trajectory::default() };
    let spin = spin_rate_pose(&parked, [0.0, 90.0, 0.0], 2.5, 3000);
    assert_eq!(spin.tr_type, path_kind::STRAIGHT_THEN_STOP);
    assert_eq!((spin.began_at, spin.span_ms), (3000, 2500));
    assert_eq!(spin.base_point, [0.0, 45.0, 0.0]);
    assert_eq!(spin.rate, [0.0, 90.0, 0.0]);
}

#[test]
fn spinning_a_turning_mover_starts_from_where_it_has_got_to() {
    let turning = moving([10.0, 0.0, 0.0], [100.0, 0.0, 0.0], 1000);
    let spin = spin_rate_pose(&turning, [0.0, 0.0, 30.0], 1.0, 1500);
    assert!(near(spin.base_point, [60.0, 0.0, 0.0]), "{:?}", spin.base_point);
    assert_eq!((spin.began_at, spin.span_ms), (1500, 1000));
}

#[test]
fn beginning_a_spin_changes_only_the_angles() {
    let mut state = build_scripted_mover(FIRST, [7.0, 8.0, 9.0], [0.0, 10.0, 0.0]);
    let before = state;
    start_scripted_spin(&mut state, [0.0, 180.0, 0.0], 0.5, 200);
    assert_eq!(state.lerp.pos, before.lerp.pos);
    assert_eq!(state.lerp.apos.tr_type, path_kind::STRAIGHT_THEN_STOP);
    assert_eq!(state.lerp.apos.span_ms, 500);
    assert_eq!(state.lerp.apos.base_point, [0.0, 10.0, 0.0]);
    assert_eq!(state.slot_index, before.slot_index);
}

#[test]
fn a_script_mover_gentity_defaults_to_nothing() {
    let mover = MoverSlot::default();
    assert_eq!(mover.id, crate::match_ids::ScriptedModel::decode_net(0));
    assert!(!mover.nonsolid);
    assert_eq!(mover.shown_to, 0);
    assert_eq!(mover.state, entity_t6::EntitySnapshot::default());
}

// ---- phases ----

#[test]
fn the_phases_run_in_their_declared_order() {
    assert_eq!(PASS_STAGE_SEQUENCE.len(), 9);
    assert!(PASS_STAGE_SEQUENCE.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(PASS_STAGE_SEQUENCE.first(), Some(&EntityPassStage::TimerStepResult));
    assert_eq!(PASS_STAGE_SEQUENCE.last(), Some(&EntityPassStage::SendOut));
    assert_eq!(first_spawnable_slot(), PROTECTED_ENTITY_SLOTS);
}

// ---- use presses ----

const OTHER_BUTTON: u32 = 0x1;

fn press(client: u32, edge: bool) -> InteractTap {
    InteractTap { client, holding: true, rim: edge }
}

#[test]
fn use_gives_an_edge_only_on_the_first_command_it_is_down() {
    let cmds = [(3, buttons::INTERACT_KEY), (3, buttons::INTERACT_KEY), (3, OTHER_BUTTON), (3, buttons::INTERACT_KEY)];
    assert_eq!(gather_presses(&cmds, &[]), [press(3, true), press(3, false), press(3, true)]);
}

#[test]
fn use_held_from_the_last_frame_is_not_a_new_press() {
    assert_eq!(gather_presses(&[(1, buttons::INTERACT_KEY)], &[(1, buttons::INTERACT_KEY)]), [press(1, false)]);
    assert_eq!(gather_presses(&[(1, buttons::INTERACT_KEY)], &[(1, OTHER_BUTTON)]), [press(1, true)]);
}

#[test]
fn the_reload_use_button_counts_as_use() {
    let cmds = [(2, buttons::INTERACT_RELOAD_KEY), (2, buttons::INTERACT_RELOAD_KEY | OTHER_BUTTON), (2, buttons::INTERACT_KEY)];
    assert_eq!(gather_presses(&cmds, &[]), [press(2, true), press(2, false), press(2, false)]);
}

#[test]
fn commands_without_use_give_no_press() {
    assert!(gather_presses(&[(1, OTHER_BUTTON), (2, 0)], &[(1, buttons::INTERACT_KEY)]).is_empty());
    assert!(gather_presses(&[], &[(1, buttons::INTERACT_KEY)]).is_empty());
}

#[test]
fn clients_are_tracked_separately() {
    let cmds = [(1, buttons::INTERACT_KEY), (2, buttons::INTERACT_KEY), (1, buttons::INTERACT_KEY), (2, 0), (2, buttons::INTERACT_KEY)];
    let old = [(2, buttons::INTERACT_KEY)];
    assert_eq!(
        gather_presses(&cmds, &old),
        [press(1, true), press(2, false), press(1, false), press(2, true)]
    );
}

#[test]
fn the_first_old_entry_for_a_client_wins() {
    let old = [(5, buttons::INTERACT_KEY), (5, 0)];
    assert_eq!(gather_presses(&[(5, buttons::INTERACT_KEY)], &old), [press(5, false)]);
    let old = [(5, 0), (5, buttons::INTERACT_KEY)];
    assert_eq!(gather_presses(&[(5, buttons::INTERACT_KEY)], &old), [press(5, true)]);
}
