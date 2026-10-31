//! The frame rows: what each store holds, which numbers the kernel lists, and how adoption, retirement and shutdown
//! leave them.

use bevy_ecs::prelude::World;
use entity_t6::EntitySnapshot;
use entity_t6::flight::path_kind::{STRAIGHT, AT_REST};

use super::adopt::reconcile;
use super::rows::{RowDirectory, ProjectileRow};
use super::{
    StepWorldView, gather_loot, gather_avatars, gather_shells, gather_movers,
    seat_state_holder, player_row_count, script_mover_row_count,
};
use crate::ent_slots::{EntityBehaviorClass, MoverSlot};
use crate::match_ids::ScriptedModel;
use crate::floor_guns::LootOnGround;
use crate::sim_parts::{SeatNo, MatchSimulation, StepNo};
use crate::{LifeCount, MissileGuide, ThrownId, FlyingProjectile};

fn fresh_world() -> World {
    let mut ecs = World::default();
    let at = ecs.spawn((MatchSimulation::default(), RowDirectory::default())).id();
    seat_state_holder(&mut ecs, at);
    ecs.insert_resource(crate::gsc_host::Runtime::default());
    ecs
}

/// Has the kernel hand out a slot of `kind` and returns its number.
fn slot(frame: &mut StepWorldView, kind: EntityBehaviorClass) -> i32 {
    frame.allocate_dynamic_entity(kind).expect("the kernel has room").slot()
}

fn projectile(entnum: i32) -> FlyingProjectile {
    FlyingProjectile {
        id: ThrownId(entnum as u32),
        owner: SeatNo(0),
        owner_respawn_count: LifeCount(0),
        gun: 1,
        origin: [0.0; 3],
        motion: Default::default(),
        at: Default::default(),
        turn_path: Default::default(),
        ent_no: entnum,
        spawn_moment: 0,
        spawn_time_ms: 0,
        detonate_at_ms: None,
        cleanup_at_ms: 0,
        travel_distance: 0.0,
        in_flight: false,
        stuck_pane: None,
        grounded: false,
        guide: MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    }
}

fn item(number: i32) -> LootOnGround {
    LootOnGround {
        state: EntitySnapshot { slot_index: number, e_type: entity_t6::et::ITEM, ..EntitySnapshot::default() },
        origin: [0.0; 3],
        dropping: Default::default(),
        right_mag: 0,
        left_mag: 0,
        spare: 0,
        stock2: 0,
        scavenge: false,
        owner_clear_ms: 0,
    }
}

fn mover(number: i32) -> MoverSlot {
    MoverSlot { state: EntitySnapshot { slot_index: number, ..EntitySnapshot::default() }, ..MoverSlot::default() }
}

fn client_ids(ecs: &World) -> Vec<u32> {
    gather_avatars(ecs).into_iter().map(|(id, _)| id.0).collect()
}

// ---- numbered rows ----

#[test]
fn a_projectile_row_is_found_changed_and_taken_back_out() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = slot(&mut frame, EntityBehaviorClass::Missile);
    frame.add_shell(projectile(number));

    assert_eq!(frame.projectile_in_slot(number).map(|p| p.ent_no), Some(number));
    frame.projectile_in_slot_mut(number).expect("row was just pushed").in_flight = true;
    assert!(frame.projectile_in_slot(number).expect("still there").in_flight);

    let taken = frame.drop_projectile_slot(number).expect("row exists");
    assert!(taken.in_flight);
    assert!(frame.projectile_in_slot(number).is_none());
    assert!(frame.drop_projectile_slot(number).is_none());
}

#[test]
#[should_panic(expected = "think occupancy")]
fn a_row_needs_the_kernel_to_hold_its_number_as_the_right_kind() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let missile_slot = slot(&mut frame, EntityBehaviorClass::Missile);
    frame.add_loot_row(item(missile_slot));
}

#[test]
fn listings_follow_the_kernel_and_skip_numbers_without_a_row() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let _unused_missile = slot(&mut frame, EntityBehaviorClass::Missile);
    let used_missile = slot(&mut frame, EntityBehaviorClass::Missile);
    frame.add_shell(projectile(used_missile));
    let _bare_mover_slot = slot(&mut frame, EntityBehaviorClass::ScriptedMover);

    assert_eq!(gather_shells(&ecs).iter().map(|p| p.ent_no).collect::<Vec<_>>(), vec![used_missile]);
    assert!(gather_movers(&ecs).is_empty());
    assert_eq!(script_mover_row_count(&ecs), 1, "movers are counted by kernel slot, not by row");
}

#[test]
fn dropped_items_are_listed_lowest_number_first() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let numbers: Vec<i32> = (0..3).map(|_| slot(&mut frame, EntityBehaviorClass::Item)).collect();
    for &number in numbers.iter().rev() {
        frame.add_loot_row(item(number));
    }

    assert_eq!(frame.ordered_loot_slots(), numbers);
    assert_eq!(frame.dropped_item_total(), 3);
    assert!(frame.take_loot_at_slot(numbers[1]).is_some());
    assert_eq!(frame.ordered_loot_slots(), vec![numbers[0], numbers[2]]);
    assert!(frame.loot_at_slot(numbers[1]).is_none());
}

#[test]
fn a_dropped_item_can_be_edited_in_place() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = slot(&mut frame, EntityBehaviorClass::Item);
    frame.add_loot_row(item(number));

    frame.loot_at_slot_mut(number).expect("item row").spare = 12;
    assert_eq!(frame.loot_at_slot(number).expect("item row").spare, 12);
    assert!(frame.loot_at_slot_mut(number + 1).is_none());
}

// ---- script movers ----

#[test]
fn the_same_script_model_gets_the_same_mover_back() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let crate_model = ScriptedModel::decode_net(7);
    let first = frame.create_scripted_mover(crate_model, [1.0, 2.0, 3.0], [0.0; 3]).expect("room for a mover");
    let again = frame.create_scripted_mover(crate_model, [9.0; 3], [0.0; 3]).expect("already spawned");
    let other = frame.create_scripted_mover(ScriptedModel::decode_net(8), [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert_eq!(first, again);
    assert_ne!(first, other);
    assert_eq!(frame.game_actor_number(crate_model), Some(first));
    let parked = frame.script_mover_by_number(first).expect("mover row");
    assert_eq!(parked.state.lerp.pos.base_point, [1.0, 2.0, 3.0], "the second spawn must not move it");
}

#[test]
fn removing_a_mover_gives_its_number_back() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let model = ScriptedModel::decode_net(3);
    let number = frame.create_scripted_mover(model, [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert!(frame.remove_script_mover_by_number(number));
    assert_eq!(frame.game_actor_number(model), None);
    assert!(!frame.remove_script_mover_by_number(number));
    assert_eq!(script_mover_row_count(&ecs), 0);
}

#[test]
fn a_pose_set_a_tick_after_the_last_glides_between_snapshots() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = frame.create_scripted_mover(ScriptedModel::decode_net(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let tick = crate::SIM_STEP_MILLIS as i32;
    let per_second = 1000.0 / tick as f32;

    // First pose: nothing was set a tick earlier, so it just parks.
    assert!(frame.set_script_mover_pose(number, 1000, [0.0; 3], [0.0; 3]));
    let parked = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked.state.lerp.pos.tr_type, AT_REST);

    // Second pose, exactly one tick on: it moves with the velocity that covers the gap in one tick.
    assert!(frame.set_script_mover_pose(number, 1000 + tick, [10.0, 0.0, 0.0], [0.0, 90.0, 0.0]));
    let gliding = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(gliding.state.lerp.pos.tr_type, STRAIGHT);
    assert_eq!(gliding.state.lerp.pos.rate, [10.0 * per_second, 0.0, 0.0]);
    assert_eq!(gliding.state.lerp.pos.base_point, [10.0, 0.0, 0.0]);
    assert_eq!(gliding.state.lerp.apos.tr_type, STRAIGHT);
    assert_eq!(gliding.state.lerp.apos.rate, [0.0, 90.0 * per_second, 0.0]);

    // A longer gap parks it again at the new pose.
    assert!(frame.set_script_mover_pose(number, 5000, [3.0, 3.0, 3.0], [0.0; 3]));
    let parked_again = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked_again.state.lerp.pos.tr_type, AT_REST);
    assert_eq!(parked_again.state.lerp.pos.rate, [0.0; 3]);
    assert_eq!(parked_again.state.lerp.pos.base_point, [3.0, 3.0, 3.0]);

    assert!(!frame.set_script_mover_pose(number + 100, 1000, [0.0; 3], [0.0; 3]));
}

#[test]
fn parking_a_mover_drops_its_velocity() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = frame.create_scripted_mover(ScriptedModel::decode_net(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let tick = crate::SIM_STEP_MILLIS as i32;
    frame.set_script_mover_pose(number, 1000, [0.0; 3], [0.0; 3]);
    frame.set_script_mover_pose(number, 1000 + tick, [5.0, 0.0, 0.0], [0.0; 3]);

    assert!(frame.place_mover(number, [7.0, 7.0, 7.0]));
    let parked = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked.state.lerp.pos.tr_type, AT_REST);
    assert_eq!(parked.state.lerp.pos.rate, [0.0; 3]);
    assert_eq!(parked.state.lerp.pos.base_point, [7.0, 7.0, 7.0]);
}

#[test]
fn blades_spin_on_pitch_unless_they_face_up_or_down() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let flat = ScriptedModel::decode_net(1);
    let on_edge = ScriptedModel::decode_net(2);
    let flat_number = frame.create_scripted_mover(flat, [0.0; 3], [0.0, 0.0, 0.0]).expect("room for a mover");
    let edge_number = frame.create_scripted_mover(on_edge, [0.0; 3], [0.0, 0.0, 90.0]).expect("room for a mover");

    assert_eq!(frame.start_rate_spins(100.0, 2000), 2);
    let spin_of = |frame: &StepWorldView, number| {
        frame.script_mover_by_number(number).expect("mover row").state.lerp.apos.rate
    };
    assert_eq!(spin_of(&frame, flat_number), [100.0, 0.0, 0.0]);
    assert_eq!(spin_of(&frame, edge_number), [0.0, 100.0, 0.0]);
}

#[test]
fn a_mover_box_can_be_set_by_number() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = frame.create_scripted_mover(ScriptedModel::decode_net(1), [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert!(frame.resize_scripted_mover_box(number, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    let boxed = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!((boxed.box_centre, boxed.box_span), ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    assert!(!frame.resize_scripted_mover_box(number + 100, [0.0; 3], [0.0; 3]));
}

// ---- players ----

#[test]
fn ensure_player_adds_a_blank_row_only_once() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let id = SeatNo(3);

    assert_eq!(frame.seat_slot_for(id).body_index, -1);
    frame.seat_slot_for(id).origin = [1.0, 2.0, 3.0];
    assert_eq!(frame.seat_slot_for(id).origin, [1.0, 2.0, 3.0], "a second call must not blank the player");
    assert_eq!(player_row_count(&ecs), 1);
}

#[test]
fn players_come_out_in_client_order_whatever_order_they_joined() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    for id in [5, 2, 9, 0] {
        frame.seat_slot_for(SeatNo(id));
    }

    let mut visited = Vec::new();
    frame.walk_avatars(|id, _| visited.push(id.0));
    assert_eq!(visited, vec![0, 2, 5, 9]);
    assert_eq!(client_ids(&ecs), vec![0, 2, 5, 9]);
}

#[test]
fn for_each_player_mut_edits_every_player() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    for id in [1, 2, 3] {
        frame.seat_slot_for(SeatNo(id));
    }

    frame.for_each_player_mut(|id, ps| ps.origin[0] = id.0 as f32 * 10.0);
    let xs: Vec<f32> = gather_avatars(&ecs).iter().map(|(_, ps)| ps.origin[0]).collect();
    assert_eq!(xs, vec![10.0, 20.0, 30.0]);
}

#[test]
fn set_origin_moves_and_stops_the_player() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let id = SeatNo(1);
    frame.seat_slot_for(id).velocity = [300.0, 0.0, -50.0];

    assert!(frame.place_at(id, [10.0, 20.0, 30.0]));
    let ps = frame.avatar(id).expect("player row");
    assert_eq!(ps.origin, [10.0, 20.0, 30.0]);
    assert_eq!(ps.velocity, [0.0; 3]);
    assert!(!frame.place_at(SeatNo(2), [0.0; 3]), "there is no such player");
}

#[test]
fn a_gate_nudge_shifts_the_player_and_keeps_their_speed() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let id = SeatNo(1);
    {
        let ps = frame.seat_slot_for(id);
        ps.origin = [1.0, 1.0, 1.0];
        ps.velocity = [5.0, 6.0, 7.0];
    }

    frame.nudge_gate(id, [2.0, 0.0, -1.0]);
    let ps = frame.avatar(id).expect("player row");
    assert_eq!(ps.origin, [3.0, 1.0, 0.0]);
    assert_eq!(ps.velocity, [5.0, 6.0, 7.0]);
    frame.nudge_gate(SeatNo(8), [1.0; 3]);
}

#[test]
fn the_single_field_setters_say_whether_the_player_exists() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let id = SeatNo(4);
    frame.seat_slot_for(id);

    assert!(frame.write_stride_clip(id, 17));
    assert!(frame.write_aim(id, [1.0, 2.0, 3.0]));
    assert!(frame.write_body_flags(id, 0x40));
    assert!(frame.write_lean(id, -0.5));
    assert!(frame.aim_eye_height(id, 11));
    let pstate = frame.avatar(id).expect("player row");
    assert_eq!(
        (pstate.legs_anim, pstate.look_angles, pstate.e_flags, pstate.tilt_sideways, pstate.eye_height_goal),
        (17, [1.0, 2.0, 3.0], 0x40, -0.5, 11)
    );

    let nobody = SeatNo(99);
    assert!(!frame.write_stride_clip(nobody, 1));
    assert!(!frame.write_aim(nobody, [0.0; 3]));
    assert!(!frame.write_body_flags(nobody, 1));
    assert!(!frame.write_lean(nobody, 1.0));
    assert!(!frame.aim_eye_height(nobody, 1));
}

#[test]
fn retiring_a_client_removes_their_row_and_is_safe_to_repeat() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    for id in [1, 2] {
        frame.seat_slot_for(SeatNo(id));
    }

    frame.drop_player_record(SeatNo(1));
    assert!(frame.avatar(SeatNo(1)).is_none());
    assert!(frame.avatar(SeatNo(2)).is_some());
    frame.drop_player_record(SeatNo(1));
    assert_eq!(client_ids(&ecs), vec![2]);
}

#[test]
fn only_living_clients_have_an_alive_origin() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    frame.seat_slot_for(SeatNo(1)).origin = [1.0, 0.0, 0.0];
    frame.seat_slot_for(SeatNo(2)).origin = [2.0, 0.0, 0.0];
    frame.client_meta_mut(SeatNo(1)).lifecycle = crate::PlayerSessionStage::Alive;
    frame.client_meta_mut(SeatNo(2)).lifecycle = crate::PlayerSessionStage::Dead;

    assert_eq!(frame.living_positions(), vec![[1.0, 0.0, 0.0]]);
}

// ---- snapshots, adoption and shutdown ----

#[test]
fn reconcile_updates_in_place_and_leaves_one_row_per_number() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let number = slot(&mut frame, EntityBehaviorClass::Missile);

    reconcile(&mut ecs, &[], vec![projectile(number)], vec![], vec![]);
    let mut changed = projectile(number);
    changed.in_flight = true;
    reconcile(&mut ecs, &[], vec![changed], vec![], vec![]);

    assert_eq!(ecs.query::<&ProjectileRow>().iter(&ecs).count(), 1);
    assert_eq!(gather_shells(&ecs), vec![changed]);
}

#[test]
fn reconcile_drops_every_kind_the_snapshot_no_longer_lists() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    let missile = slot(&mut frame, EntityBehaviorClass::Missile);
    let dropped = slot(&mut frame, EntityBehaviorClass::Item);
    let kept = slot(&mut frame, EntityBehaviorClass::ScriptedMover);
    frame.add_shell(projectile(missile));
    frame.add_loot_row(item(dropped));
    frame.seat_slot_for(SeatNo(6));
    reconcile(&mut ecs, &[], vec![], vec![mover(kept)], vec![]);

    assert!(gather_shells(&ecs).is_empty());
    assert!(gather_loot(&ecs).is_empty());
    assert_eq!(gather_movers(&ecs).len(), 1);
    assert_eq!(player_row_count(&ecs), 0, "a player the snapshot does not list goes too");
}

#[test]
fn a_snapshot_rebuilds_every_kind_of_row_in_a_fresh_world() {
    let mut source = fresh_world();
    let mut frame = StepWorldView::from_world(&mut source);
    frame.seat_slot_for(SeatNo(4)).origin = [4.0, 0.0, 0.0];
    frame.seat_slot_for(SeatNo(1)).origin = [1.0, 0.0, 0.0];
    let missile = slot(&mut frame, EntityBehaviorClass::Missile);
    frame.add_shell(projectile(missile));
    let dropped = slot(&mut frame, EntityBehaviorClass::Item);
    frame.add_loot_row(item(dropped));
    let platform = frame
        .create_scripted_mover(ScriptedModel::decode_net(2), [5.0; 3], [0.0; 3])
        .expect("room for a mover");
    let snapshot = frame.snapshot(StepNo(9));

    let mut copy = fresh_world();
    StepWorldView::from_world(&mut copy).take_over_from(&snapshot);

    let origins = |ecs: &World| {
        gather_avatars(ecs).into_iter().map(|(id, ps)| (id.0, ps.origin)).collect::<Vec<_>>()
    };
    assert_eq!(origins(&copy), origins(&source));
    assert_eq!(gather_shells(&copy), gather_shells(&source));
    assert_eq!(gather_loot(&copy), gather_loot(&source));
    assert_eq!(gather_movers(&copy), gather_movers(&source));
    assert_eq!(gather_movers(&copy)[0].state.slot_index, platform);
}

#[test]
fn a_prediction_snapshot_leaves_only_the_local_player() {
    let mut source = fresh_world();
    let mut host = StepWorldView::from_world(&mut source);
    host.seat_slot_for(SeatNo(1));
    host.seat_slot_for(SeatNo(2));
    let snapshot = host.snapshot(StepNo(3));

    let mut predicted = fresh_world();
    let mut frame = StepWorldView::from_world(&mut predicted);
    for id in [1, 2, 3] {
        frame.seat_slot_for(SeatNo(id));
    }
    frame.adopt_prediction_snapshot(&snapshot, SeatNo(1));

    assert_eq!(client_ids(&predicted), vec![1]);
}

#[test]
fn shutting_the_game_down_clears_every_row_and_lets_a_new_one_start() {
    let mut ecs = fresh_world();
    let mut frame = StepWorldView::from_world(&mut ecs);
    frame.seat_slot_for(SeatNo(1));
    let missile = slot(&mut frame, EntityBehaviorClass::Missile);
    frame.add_shell(projectile(missile));
    frame.create_scripted_mover(ScriptedModel::decode_net(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let dropped = slot(&mut frame, EntityBehaviorClass::Item);
    frame.add_loot_row(item(dropped));

    frame.end_game_session();
    assert_eq!(player_row_count(&ecs), 0);
    assert!(gather_shells(&ecs).is_empty());
    assert!(gather_movers(&ecs).is_empty());
    assert!(gather_loot(&ecs).is_empty());

    let mut frame = StepWorldView::from_world(&mut ecs);
    frame.seat_slot_for(SeatNo(1));
    assert_eq!(player_row_count(&ecs), 1);
}
