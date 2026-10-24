//! The frame rows: what each store holds, which numbers the kernel lists, and how adoption, retirement and shutdown
//! leave them.

use bevy_ecs::prelude::World;
use entity_t6::EntityState;
use entity_t6::trajectory::tr_type::{LINEAR, STATIONARY};

use super::adopt::reconcile;
use super::rows::{PayloadIndex, ProjectileRow};
use super::{
    FrameWorld, collect_dropped_items, collect_players, collect_projectiles, collect_script_movers,
    install_state_entity, player_row_count, script_mover_row_count,
};
use crate::gentity::{EntityRunKind, ScriptMoverGentity};
use crate::identities::ScriptModelId;
use crate::item::DroppedItem;
use crate::world::{ClientId, SimState, Tick};
use crate::{LifeSequence, MissileGuide, ProjectileId, ProjectileState};

fn fresh_world() -> World {
    let mut ecs = World::new();
    let at = ecs.spawn((SimState::default(), PayloadIndex::default())).id();
    install_state_entity(&mut ecs, at);
    ecs.insert_resource(crate::script::Runtime::default());
    ecs
}

/// Has the kernel hand out a slot of `kind` and returns its number.
fn slot(frame: &mut FrameWorld, kind: EntityRunKind) -> i32 {
    frame.allocate_dynamic_entity(kind).expect("the kernel has room").number()
}

fn projectile(entnum: i32) -> ProjectileState {
    ProjectileState {
        id: ProjectileId(entnum as u32),
        owner: ClientId(0),
        owner_life: LifeSequence(0),
        weapon: 1,
        origin: [0.0; 3],
        velocity: [0.0; 3],
        pos: Default::default(),
        apos: Default::default(),
        entnum,
        launch_time: 0,
        spawn_time_ms: 0,
        detonate_at_ms: None,
        cleanup_at_ms: 0,
        travel_distance: 0.0,
        live: false,
        stuck_pane: None,
        grounded: false,
        guide: MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    }
}

fn item(number: i32) -> DroppedItem {
    DroppedItem {
        state: EntityState { number, e_type: entity_t6::et::ITEM, ..EntityState::default() },
        origin: [0.0; 3],
        falling: false,
        clip_r: 0,
        clip_l: 0,
        stock: 0,
        stock2: 0,
        scavenger: false,
        owner_clear_ms: 0,
    }
}

fn mover(number: i32) -> ScriptMoverGentity {
    ScriptMoverGentity { state: EntityState { number, ..EntityState::default() }, ..ScriptMoverGentity::default() }
}

fn client_ids(ecs: &World) -> Vec<u32> {
    collect_players(ecs).into_iter().map(|(id, _)| id.0).collect()
}

// ---- numbered rows ----

#[test]
fn a_projectile_row_is_found_changed_and_taken_back_out() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = slot(&mut frame, EntityRunKind::Missile);
    frame.push_projectile(projectile(number));

    assert_eq!(frame.projectile_by_number(number).map(|p| p.entnum), Some(number));
    frame.projectile_mut_by_number(number).expect("row was just pushed").live = true;
    assert!(frame.projectile_by_number(number).expect("still there").live);

    let taken = frame.remove_projectile_by_number(number).expect("row exists");
    assert!(taken.live);
    assert!(frame.projectile_by_number(number).is_none());
    assert!(frame.remove_projectile_by_number(number).is_none());
}

#[test]
#[should_panic(expected = "think occupancy")]
fn a_row_needs_the_kernel_to_hold_its_number_as_the_right_kind() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let missile_slot = slot(&mut frame, EntityRunKind::Missile);
    frame.push_dropped_item(item(missile_slot));
}

#[test]
fn listings_follow_the_kernel_and_skip_numbers_without_a_row() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let _unused_missile = slot(&mut frame, EntityRunKind::Missile);
    let used_missile = slot(&mut frame, EntityRunKind::Missile);
    frame.push_projectile(projectile(used_missile));
    let _bare_mover_slot = slot(&mut frame, EntityRunKind::ScriptMover);

    assert_eq!(collect_projectiles(&ecs).iter().map(|p| p.entnum).collect::<Vec<_>>(), vec![used_missile]);
    assert!(collect_script_movers(&ecs).is_empty());
    assert_eq!(script_mover_row_count(&ecs), 1, "movers are counted by kernel slot, not by row");
}

#[test]
fn dropped_items_are_listed_lowest_number_first() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let numbers: Vec<i32> = (0..3).map(|_| slot(&mut frame, EntityRunKind::Item)).collect();
    for &number in numbers.iter().rev() {
        frame.push_dropped_item(item(number));
    }

    assert_eq!(frame.dropped_item_numbers_sorted(), numbers);
    assert_eq!(frame.dropped_item_count(), 3);
    assert!(frame.remove_dropped_item_by_number(numbers[1]).is_some());
    assert_eq!(frame.dropped_item_numbers_sorted(), vec![numbers[0], numbers[2]]);
    assert!(frame.dropped_item_by_number(numbers[1]).is_none());
}

#[test]
fn a_dropped_item_can_be_edited_in_place() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = slot(&mut frame, EntityRunKind::Item);
    frame.push_dropped_item(item(number));

    frame.dropped_item_mut_by_number(number).expect("item row").stock = 12;
    assert_eq!(frame.dropped_item_by_number(number).expect("item row").stock, 12);
    assert!(frame.dropped_item_mut_by_number(number + 1).is_none());
}

// ---- script movers ----

#[test]
fn the_same_script_model_gets_the_same_mover_back() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let crate_model = ScriptModelId::from_wire(7);
    let first = frame.spawn_script_mover(crate_model, [1.0, 2.0, 3.0], [0.0; 3]).expect("room for a mover");
    let again = frame.spawn_script_mover(crate_model, [9.0; 3], [0.0; 3]).expect("already spawned");
    let other = frame.spawn_script_mover(ScriptModelId::from_wire(8), [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert_eq!(first, again);
    assert_ne!(first, other);
    assert_eq!(frame.gentity_number(crate_model), Some(first));
    let parked = frame.script_mover_by_number(first).expect("mover row");
    assert_eq!(parked.state.lerp.pos.tr_base, [1.0, 2.0, 3.0], "the second spawn must not move it");
}

#[test]
fn removing_a_mover_gives_its_number_back() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let model = ScriptModelId::from_wire(3);
    let number = frame.spawn_script_mover(model, [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert!(frame.remove_script_mover_by_number(number));
    assert_eq!(frame.gentity_number(model), None);
    assert!(!frame.remove_script_mover_by_number(number));
    assert_eq!(script_mover_row_count(&ecs), 0);
}

#[test]
fn a_pose_set_a_tick_after_the_last_glides_between_snapshots() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = frame.spawn_script_mover(ScriptModelId::from_wire(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let tick = crate::MATCH_TICK_MS as i32;
    let per_second = 1000.0 / tick as f32;

    // First pose: nothing was set a tick earlier, so it just parks.
    assert!(frame.set_script_mover_pose(number, 1000, [0.0; 3], [0.0; 3]));
    let parked = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked.state.lerp.pos.tr_type, STATIONARY);

    // Second pose, exactly one tick on: it moves with the velocity that covers the gap in one tick.
    assert!(frame.set_script_mover_pose(number, 1000 + tick, [10.0, 0.0, 0.0], [0.0, 90.0, 0.0]));
    let gliding = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(gliding.state.lerp.pos.tr_type, LINEAR);
    assert_eq!(gliding.state.lerp.pos.tr_delta, [10.0 * per_second, 0.0, 0.0]);
    assert_eq!(gliding.state.lerp.pos.tr_base, [10.0, 0.0, 0.0]);
    assert_eq!(gliding.state.lerp.apos.tr_type, LINEAR);
    assert_eq!(gliding.state.lerp.apos.tr_delta, [0.0, 90.0 * per_second, 0.0]);

    // A longer gap parks it again at the new pose.
    assert!(frame.set_script_mover_pose(number, 5000, [3.0, 3.0, 3.0], [0.0; 3]));
    let parked_again = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked_again.state.lerp.pos.tr_type, STATIONARY);
    assert_eq!(parked_again.state.lerp.pos.tr_delta, [0.0; 3]);
    assert_eq!(parked_again.state.lerp.pos.tr_base, [3.0, 3.0, 3.0]);

    assert!(!frame.set_script_mover_pose(number + 100, 1000, [0.0; 3], [0.0; 3]));
}

#[test]
fn parking_a_mover_drops_its_velocity() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = frame.spawn_script_mover(ScriptModelId::from_wire(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let tick = crate::MATCH_TICK_MS as i32;
    frame.set_script_mover_pose(number, 1000, [0.0; 3], [0.0; 3]);
    frame.set_script_mover_pose(number, 1000 + tick, [5.0, 0.0, 0.0], [0.0; 3]);

    assert!(frame.set_script_mover_origin(number, [7.0, 7.0, 7.0]));
    let parked = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!(parked.state.lerp.pos.tr_type, STATIONARY);
    assert_eq!(parked.state.lerp.pos.tr_delta, [0.0; 3]);
    assert_eq!(parked.state.lerp.pos.tr_base, [7.0, 7.0, 7.0]);
}

#[test]
fn blades_spin_on_pitch_unless_they_face_up_or_down() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let flat = ScriptModelId::from_wire(1);
    let on_edge = ScriptModelId::from_wire(2);
    let flat_number = frame.spawn_script_mover(flat, [0.0; 3], [0.0, 0.0, 0.0]).expect("room for a mover");
    let edge_number = frame.spawn_script_mover(on_edge, [0.0; 3], [0.0, 0.0, 90.0]).expect("room for a mover");

    assert_eq!(frame.begin_script_movers_rotate_velocity_supplied(100.0, 2000), 2);
    let spin_of = |frame: &FrameWorld, number| {
        frame.script_mover_by_number(number).expect("mover row").state.lerp.apos.tr_delta
    };
    assert_eq!(spin_of(&frame, flat_number), [100.0, 0.0, 0.0]);
    assert_eq!(spin_of(&frame, edge_number), [0.0, 100.0, 0.0]);
}

#[test]
fn a_mover_box_can_be_set_by_number() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = frame.spawn_script_mover(ScriptModelId::from_wire(1), [0.0; 3], [0.0; 3]).expect("room for a mover");

    assert!(frame.set_script_mover_r_box(number, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    let boxed = frame.script_mover_by_number(number).expect("mover row");
    assert_eq!((boxed.box_mid, boxed.box_half), ([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    assert!(!frame.set_script_mover_r_box(number + 100, [0.0; 3], [0.0; 3]));
}

// ---- players ----

#[test]
fn ensure_player_adds_a_blank_row_only_once() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let id = ClientId(3);

    assert_eq!(frame.ensure_player(id).corpse_index, -1);
    frame.ensure_player(id).origin = [1.0, 2.0, 3.0];
    assert_eq!(frame.ensure_player(id).origin, [1.0, 2.0, 3.0], "a second call must not blank the player");
    assert_eq!(player_row_count(&ecs), 1);
}

#[test]
fn players_come_out_in_client_order_whatever_order_they_joined() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    for id in [5, 2, 9, 0] {
        frame.ensure_player(ClientId(id));
    }

    let mut visited = Vec::new();
    frame.visit_players(|id, _| visited.push(id.0));
    assert_eq!(visited, vec![0, 2, 5, 9]);
    assert_eq!(client_ids(&ecs), vec![0, 2, 5, 9]);
}

#[test]
fn for_each_player_mut_edits_every_player() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    for id in [1, 2, 3] {
        frame.ensure_player(ClientId(id));
    }

    frame.for_each_player_mut(|id, ps| ps.origin[0] = id.0 as f32 * 10.0);
    let xs: Vec<f32> = collect_players(&ecs).iter().map(|(_, ps)| ps.origin[0]).collect();
    assert_eq!(xs, vec![10.0, 20.0, 30.0]);
}

#[test]
fn set_origin_moves_and_stops_the_player() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let id = ClientId(1);
    frame.ensure_player(id).velocity = [300.0, 0.0, -50.0];

    assert!(frame.set_origin(id, [10.0, 20.0, 30.0]));
    let ps = frame.player(id).expect("player row");
    assert_eq!(ps.origin, [10.0, 20.0, 30.0]);
    assert_eq!(ps.velocity, [0.0; 3]);
    assert!(!frame.set_origin(ClientId(2), [0.0; 3]), "there is no such player");
}

#[test]
fn a_gate_nudge_shifts_the_player_and_keeps_their_speed() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let id = ClientId(1);
    {
        let ps = frame.ensure_player(id);
        ps.origin = [1.0, 1.0, 1.0];
        ps.velocity = [5.0, 6.0, 7.0];
    }

    frame.gate_nudge_origin(id, [2.0, 0.0, -1.0]);
    let ps = frame.player(id).expect("player row");
    assert_eq!(ps.origin, [3.0, 1.0, 0.0]);
    assert_eq!(ps.velocity, [5.0, 6.0, 7.0]);
    frame.gate_nudge_origin(ClientId(8), [1.0; 3]);
}

#[test]
fn the_single_field_setters_say_whether_the_player_exists() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let id = ClientId(4);
    frame.ensure_player(id);

    assert!(frame.set_legs_anim(id, 17));
    assert!(frame.set_viewangles(id, [1.0, 2.0, 3.0]));
    assert!(frame.set_e_flags(id, 0x40));
    assert!(frame.set_leanf(id, -0.5));
    assert!(frame.set_view_height_target(id, 11));
    let ps = frame.player(id).expect("player row");
    assert_eq!(
        (ps.legs_anim, ps.viewangles, ps.e_flags, ps.leanf, ps.view_height_target),
        (17, [1.0, 2.0, 3.0], 0x40, -0.5, 11)
    );

    let nobody = ClientId(99);
    assert!(!frame.set_legs_anim(nobody, 1));
    assert!(!frame.set_viewangles(nobody, [0.0; 3]));
    assert!(!frame.set_e_flags(nobody, 1));
    assert!(!frame.set_leanf(nobody, 1.0));
    assert!(!frame.set_view_height_target(nobody, 1));
}

#[test]
fn retiring_a_client_removes_their_row_and_is_safe_to_repeat() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    for id in [1, 2] {
        frame.ensure_player(ClientId(id));
    }

    frame.retire_client(ClientId(1));
    assert!(frame.player(ClientId(1)).is_none());
    assert!(frame.player(ClientId(2)).is_some());
    frame.retire_client(ClientId(1));
    assert_eq!(client_ids(&ecs), vec![2]);
}

#[test]
fn only_living_clients_have_an_alive_origin() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    frame.ensure_player(ClientId(1)).origin = [1.0, 0.0, 0.0];
    frame.ensure_player(ClientId(2)).origin = [2.0, 0.0, 0.0];
    frame.client_meta_mut(ClientId(1)).lifecycle = crate::ClientLifecycle::Alive;
    frame.client_meta_mut(ClientId(2)).lifecycle = crate::ClientLifecycle::Dead;

    assert_eq!(frame.alive_origins(), vec![[1.0, 0.0, 0.0]]);
}

// ---- snapshots, adoption and shutdown ----

#[test]
fn reconcile_updates_in_place_and_leaves_one_row_per_number() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let number = slot(&mut frame, EntityRunKind::Missile);

    reconcile(&mut ecs, &[], vec![projectile(number)], vec![], vec![]);
    let mut changed = projectile(number);
    changed.live = true;
    reconcile(&mut ecs, &[], vec![changed], vec![], vec![]);

    assert_eq!(ecs.query::<&ProjectileRow>().iter(&ecs).count(), 1);
    assert_eq!(collect_projectiles(&ecs), vec![changed]);
}

#[test]
fn reconcile_drops_every_kind_the_snapshot_no_longer_lists() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    let missile = slot(&mut frame, EntityRunKind::Missile);
    let dropped = slot(&mut frame, EntityRunKind::Item);
    let kept = slot(&mut frame, EntityRunKind::ScriptMover);
    frame.push_projectile(projectile(missile));
    frame.push_dropped_item(item(dropped));
    frame.ensure_player(ClientId(6));
    reconcile(&mut ecs, &[], vec![], vec![mover(kept)], vec![]);

    assert!(collect_projectiles(&ecs).is_empty());
    assert!(collect_dropped_items(&ecs).is_empty());
    assert_eq!(collect_script_movers(&ecs).len(), 1);
    assert_eq!(player_row_count(&ecs), 0, "a player the snapshot does not list goes too");
}

#[test]
fn a_snapshot_rebuilds_every_kind_of_row_in_a_fresh_world() {
    let mut source = fresh_world();
    let mut frame = FrameWorld::from_world(&mut source);
    frame.ensure_player(ClientId(4)).origin = [4.0, 0.0, 0.0];
    frame.ensure_player(ClientId(1)).origin = [1.0, 0.0, 0.0];
    let missile = slot(&mut frame, EntityRunKind::Missile);
    frame.push_projectile(projectile(missile));
    let dropped = slot(&mut frame, EntityRunKind::Item);
    frame.push_dropped_item(item(dropped));
    let platform = frame
        .spawn_script_mover(ScriptModelId::from_wire(2), [5.0; 3], [0.0; 3])
        .expect("room for a mover");
    let snapshot = frame.snapshot(Tick(9));

    let mut copy = fresh_world();
    FrameWorld::from_world(&mut copy).adopt_snapshot(&snapshot);

    let origins = |ecs: &World| {
        collect_players(ecs).into_iter().map(|(id, ps)| (id.0, ps.origin)).collect::<Vec<_>>()
    };
    assert_eq!(origins(&copy), origins(&source));
    assert_eq!(collect_projectiles(&copy), collect_projectiles(&source));
    assert_eq!(collect_dropped_items(&copy), collect_dropped_items(&source));
    assert_eq!(collect_script_movers(&copy), collect_script_movers(&source));
    assert_eq!(collect_script_movers(&copy)[0].state.number, platform);
}

#[test]
fn a_prediction_snapshot_leaves_only_the_local_player() {
    let mut source = fresh_world();
    let mut host = FrameWorld::from_world(&mut source);
    host.ensure_player(ClientId(1));
    host.ensure_player(ClientId(2));
    let snapshot = host.snapshot(Tick(3));

    let mut predicted = fresh_world();
    let mut frame = FrameWorld::from_world(&mut predicted);
    for id in [1, 2, 3] {
        frame.ensure_player(ClientId(id));
    }
    frame.adopt_prediction_snapshot(&snapshot, ClientId(1));

    assert_eq!(client_ids(&predicted), vec![1]);
}

#[test]
fn shutting_the_game_down_clears_every_row_and_lets_a_new_one_start() {
    let mut ecs = fresh_world();
    let mut frame = FrameWorld::from_world(&mut ecs);
    frame.ensure_player(ClientId(1));
    let missile = slot(&mut frame, EntityRunKind::Missile);
    frame.push_projectile(projectile(missile));
    frame.spawn_script_mover(ScriptModelId::from_wire(1), [0.0; 3], [0.0; 3]).expect("room for a mover");
    let dropped = slot(&mut frame, EntityRunKind::Item);
    frame.push_dropped_item(item(dropped));

    frame.shutdown_game();
    assert_eq!(player_row_count(&ecs), 0);
    assert!(collect_projectiles(&ecs).is_empty());
    assert!(collect_script_movers(&ecs).is_empty());
    assert!(collect_dropped_items(&ecs).is_empty());

    let mut frame = FrameWorld::from_world(&mut ecs);
    frame.ensure_player(ClientId(1));
    assert_eq!(player_row_count(&ecs), 1);
}
