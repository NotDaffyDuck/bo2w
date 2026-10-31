use super::{WholeSim, AdvanceCause};
use crate::match_ids::ScriptedModel;
use crate::gsc_host::Runtime;
use crate::sim_parts::{SeatNo, StepNo};

fn model(n: u32) -> ScriptedModel {
    ScriptedModel(n)
}

fn world_with_player(n: u32) -> WholeSim {
    let mut world = WholeSim::new();
    world.frame().seat_slot_for(SeatNo(n));
    world
}

fn player_ids(world: &WholeSim) -> Vec<u32> {
    let mut found = Vec::new();
    world.each_avatar(|client, _| found.push(client.0));
    found.sort_unstable();
    found
}

// ---- a fresh world ----

#[test]
fn a_new_world_holds_nothing() {
    let mut world = WholeSim::new();
    assert_eq!(world.seated_player_total(), 0);
    assert_eq!(world.scripted_mover_total(), 0);
    assert!(world.script_seats().is_empty());
    assert_eq!(world.gsc_program_fingerprint(), None);
    assert!(world.take_script_fault().is_none());
    assert!(world.take_t6_test_clients().is_empty());
    assert!(world.take_t6_bot_leaves().is_empty());
    let mut projectiles = 0;
    world.visit_projectiles(|_| projectiles += 1);
    assert_eq!(projectiles, 0);
}

#[test]
fn the_level_exit_flag_reads_true_exactly_once() {
    let mut world = WholeSim::new();
    assert!(!world.take_script_exit_level());
    world.bevy.resource_mut::<Runtime>().exit_level = true;
    assert!(world.take_script_exit_level());
    assert!(!world.take_script_exit_level());
}

#[test]
fn debug_names_the_world() {
    let text = format!("{:?}", WholeSim::new());
    assert!(text.starts_with("SimWorld"), "{text}");
}

#[test]
fn the_world_reads_and_writes_through_to_its_state() {
    let mut world = WholeSim::new();
    let before = world.clone();
    // Writing through DerefMut must land on the state entity, not on a copy.
    let next = world.world_objects().map_round_epoch() + 1;
    world.world_objects_mut().set_map_round_epoch(next);
    assert_eq!(world.world_objects().map_round_epoch(), before.world_objects().map_round_epoch() + 1);
    assert_eq!(world.clone().world_objects().map_round_epoch(), world.world_objects().map_round_epoch());
}

// ---- why a tick runs ----

#[test]
fn only_the_authority_frame_moves_the_shared_world() {
    assert!(AdvanceCause::ServerStepFrame.moves_server_world_forward());
    assert!(!AdvanceCause::GuessAhead.moves_server_world_forward());
    assert!(!AdvanceCause::Replay.moves_server_world_forward());
    assert_eq!(AdvanceCause::default(), AdvanceCause::ServerStepFrame);
}

// ---- script movers ----

#[test]
fn spawning_a_mover_twice_gives_back_the_same_number() {
    let mut world = WholeSim::new();
    let first = world.create_scripted_mover(model(1), [1.0, 2.0, 3.0], [0.0; 3]).unwrap();
    let again = world.create_scripted_mover(model(1), [9.0, 9.0, 9.0], [0.0; 3]).unwrap();
    assert_eq!(first, again);
    assert_eq!(world.scripted_mover_total(), 1);
    assert_eq!(world.game_actor_number(model(1)), Some(first));
    assert_eq!(world.game_actor_number(model(2)), None);
}

#[test]
fn different_models_get_different_numbers() {
    let mut world = WholeSim::new();
    let a = world.create_scripted_mover(model(1), [0.0; 3], [0.0; 3]).unwrap();
    let b = world.create_scripted_mover(model(2), [0.0; 3], [0.0; 3]).unwrap();
    assert_ne!(a, b);
    assert_eq!(world.scripted_mover_total(), 2);
}

#[test]
fn movers_are_found_by_number_and_by_id() {
    let mut world = WholeSim::new();
    let number = world.create_scripted_mover(model(7), [4.0, 5.0, 6.0], [0.0; 3]).unwrap();
    let by_number = world.script_mover_by_number(number).unwrap();
    let by_id = world.mover_for_model(model(7)).unwrap();
    assert_eq!(by_number.id, model(7));
    assert_eq!(by_id.state.slot_index, number);
    assert!(world.script_mover_by_number(number + 1).is_none());
    assert!(world.mover_for_model(model(8)).is_none());

    let mut seen = Vec::new();
    world.each_mover(|mover| seen.push(mover.id));
    assert_eq!(seen, vec![model(7)]);
}

#[test]
fn a_brush_mover_wears_its_model_and_blocks() {
    let mut world = WholeSim::new();
    let number = world.spawn_brush_mover(model(3), 42, [0.0; 3], [0.0; 3]).unwrap();
    let mover = world.script_mover_by_number(number).unwrap();
    assert_eq!(mover.state.index, 42);
    assert_eq!(mover.state.solid, entity_t6::script_mover::SOLID_BMODEL);
}

#[test]
fn the_mover_box_and_origin_setters_take_a_number() {
    let mut world = WholeSim::new();
    assert!(!world.resize_scripted_mover_box(1, [0.0; 3], [1.0; 3]));
    assert!(!world.place_mover(1, [0.0; 3]));

    let number = world.create_scripted_mover(model(1), [0.0; 3], [0.0; 3]).unwrap();
    assert!(world.resize_scripted_mover_box(number, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]));
    assert!(world.place_mover(number, [10.0, 20.0, 30.0]));

    let mover = world.script_mover_by_number(number).unwrap();
    assert_eq!(mover.box_centre, [1.0, 2.0, 3.0]);
    assert_eq!(mover.box_span, [4.0, 5.0, 6.0]);
    assert_eq!(mover.state.lerp.pos.base_point, [10.0, 20.0, 30.0]);
    assert_eq!(mover.state.lerp.pos.rate, [0.0; 3]);
}

#[test]
fn spinning_a_mover_needs_it_to_exist() {
    let mut world = WholeSim::new();
    assert!(!world.start_scripted_spin(model(1), [0.0, 90.0, 0.0], 2.0, 1000));
    world.create_scripted_mover(model(1), [0.0; 3], [0.0; 3]).unwrap();
    assert!(world.start_scripted_spin(model(1), [0.0, 90.0, 0.0], 2.0, 1000));
}

#[test]
fn spinning_every_mover_counts_them() {
    let mut world = WholeSim::new();
    assert_eq!(world.start_rate_spins(100.0, 0), 0);
    for n in 1..=3 {
        world.create_scripted_mover(model(n), [0.0; 3], [0.0; 3]).unwrap();
    }
    assert_eq!(world.start_rate_spins(100.0, 0), 3);
}

// ---- players ----

#[test]
fn player_setters_refuse_a_client_that_is_not_there() {
    let mut world = WholeSim::new();
    let nobody = SeatNo(5);
    assert!(!world.set_origin(nobody, [1.0; 3]));
    assert!(!world.write_stride_clip(nobody, 3));
    assert!(!world.write_aim(nobody, [1.0; 3]));
    assert!(!world.write_body_flags(nobody, 1));
    assert!(!world.write_lean(nobody, 0.5));
    assert!(!world.aim_eye_height(nobody, 40));
    world.gate_nudge_origin(nobody, [1.0; 3]);
    assert_eq!(world.seated_player_total(), 0);
}

#[test]
fn player_setters_write_their_own_field() {
    let mut world = world_with_player(2);
    let id = SeatNo(2);
    assert!(world.set_origin(id, [10.0, 20.0, 30.0]));
    assert!(world.write_stride_clip(id, 77));
    assert!(world.write_aim(id, [5.0, 6.0, 7.0]));
    assert!(world.write_body_flags(id, 0x20));
    assert!(world.write_lean(id, 0.25));
    assert!(world.aim_eye_height(id, 11));

    let ps = world.player(id).unwrap();
    assert_eq!(ps.origin, [10.0, 20.0, 30.0]);
    assert_eq!(ps.legs_anim, 77);
    assert_eq!(ps.look_angles, [5.0, 6.0, 7.0]);
    assert_eq!(ps.e_flags, 0x20);
    assert_eq!(ps.tilt_sideways, 0.25);
    assert_eq!(ps.eye_height_goal, 11);
}

#[test]
fn setting_the_origin_stops_the_player_but_a_nudge_does_not() {
    let mut world = world_with_player(1);
    let id = SeatNo(1);
    world.frame().occupant_mut(id).unwrap().velocity = [5.0, 6.0, 7.0];
    world.gate_nudge_origin(id, [1.0, 2.0, 3.0]);
    let nudged = world.player(id).unwrap();
    assert_eq!(nudged.origin, [1.0, 2.0, 3.0]);
    assert_eq!(nudged.velocity, [5.0, 6.0, 7.0]);

    assert!(world.set_origin(id, [9.0, 9.0, 9.0]));
    assert_eq!(world.player(id).unwrap().velocity, [0.0; 3]);
}

#[test]
fn players_are_counted_and_visited() {
    let mut world = WholeSim::new();
    for n in [4, 1, 3] {
        world.frame().seat_slot_for(SeatNo(n));
    }
    assert_eq!(world.seated_player_total(), 3);
    assert_eq!(player_ids(&world), vec![1, 3, 4]);
    assert!(world.player(SeatNo(2)).is_none());
}

#[test]
fn retiring_a_client_the_scripts_never_saw_removes_it_at_once() {
    let mut world = world_with_player(1);
    world.frame().seat_slot_for(SeatNo(2));
    world.drop_player_record(SeatNo(1));
    assert_eq!(player_ids(&world), vec![2]);
    world.drop_player_record(SeatNo(9));
    assert_eq!(player_ids(&world), vec![2]);
}

// ---- copies and snapshots ----

#[test]
fn a_clone_shares_nothing_with_the_original() {
    let mut original = world_with_player(1);
    original.create_scripted_mover(model(1), [0.0; 3], [0.0; 3]).unwrap();
    let mut copy = original.clone();
    assert_eq!(copy.seated_player_total(), 1);
    assert_eq!(copy.scripted_mover_total(), 1);

    copy.frame().seat_slot_for(SeatNo(2));
    copy.create_scripted_mover(model(2), [0.0; 3], [0.0; 3]).unwrap();
    assert!(copy.set_origin(SeatNo(1), [8.0; 3]));

    assert_eq!(original.seated_player_total(), 1);
    assert_eq!(original.scripted_mover_total(), 1);
    assert_eq!(original.player(SeatNo(1)).unwrap().origin, [0.0; 3]);
}

#[test]
fn a_snapshot_carries_the_players_and_adopting_it_restores_them() {
    let mut source = WholeSim::new();
    for n in [1, 2] {
        source.frame().seat_slot_for(SeatNo(n));
        source.set_origin(SeatNo(n), [n as f32 * 100.0, 0.0, 0.0]);
    }
    let snapshot = source.snapshot(StepNo(6));
    assert_eq!(snapshot.tick, StepNo(6));
    assert_eq!(snapshot.players.len(), 2);

    let mut target = WholeSim::new();
    let report = target.take_over_from(&snapshot);
    assert_eq!(report.people, 2);
    assert_eq!(player_ids(&target), vec![1, 2]);
    assert_eq!(target.player(SeatNo(2)).unwrap().origin, [200.0, 0.0, 0.0]);
}

#[test]
fn a_prediction_snapshot_is_adopted_for_the_local_client() {
    let source = world_with_player(1);
    let snapshot = source.snapshot(StepNo(1));
    let mut target = WholeSim::new();
    target.adopt_prediction_snapshot(&snapshot, SeatNo(1));
    assert_eq!(player_ids(&target), vec![1]);
}

#[test]
fn shutting_down_an_idle_game_is_harmless() {
    let mut world = WholeSim::new();
    world.end_game_session();
    assert_eq!(world.seated_player_total(), 0);
    assert_eq!(world.gsc_program_fingerprint(), None);
}
