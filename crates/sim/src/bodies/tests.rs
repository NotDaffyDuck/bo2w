use super::fall::{fall_step, lying_angles};
use super::*;
use crate::shot_trace::PLAYER_BODY_MASK;
use entity_t6::evaluate_trajectory;
use entity_t6::path_kind::{BLEND, LIMP_FALLING, RAGDOLL_INTERPOLATE};
use trace_t6::{Trace, TraceHitType};

fn worn(entnum: i32, spot: [f32; 3]) -> CorpseBay {
    CorpseBay { occupied: true, entnum, origin: spot, ..CorpseBay::default() }
}

fn pool_of_eight(origin_of: impl Fn(usize) -> [f32; 3]) -> CorpseYard {
    let mut pool = CorpseYard::default();
    for i in 0..BODIES {
        pool.slots[i] = worn(30 + i as i32, origin_of(i));
    }
    pool
}

// ---- the entity-number ring ----

#[test]
fn ring_runs_two_laps_and_flips_the_bit_on_the_second() {
    let mut pool = CorpseYard::default();
    for lap in 0..2 {
        for i in 0..BODIES {
            let (entnum, bit) = pool.next_clone_entity();
            assert_eq!(entnum, BODY_ENTITY_FIRST_SLOT + i as i32);
            assert_eq!(bit, if lap == 0 { 0 } else { CLONE_NEW_BODY_FLAG });
        }
    }
    assert_eq!(pool.spawn_ring, 0, "sixteen hand-outs bring the counter home");
    let (entnum, bit) = pool.next_clone_entity();
    assert_eq!((entnum, bit), (BODY_ENTITY_FIRST_SLOT, 0));
}

#[test]
fn a_reused_number_always_flips_the_bit_it_wore_before() {
    let mut pool = CorpseYard::default();
    let mut last_bit = [None; BODIES];
    for round in 0..40 {
        let (entnum, bit) = pool.next_clone_entity();
        let index = (entnum - BODY_ENTITY_FIRST_SLOT) as usize;
        if let Some(previous) = last_bit[index] {
            assert_ne!(previous, bit, "round {round}: number {entnum} repeated its bit");
        }
        last_bit[index] = Some(bit);
    }
}

#[test]
fn counter_out_of_range_is_folded_back_into_the_ring() {
    let mut bank = CorpseYard { spawn_ring: 21, ..CorpseYard::default() };
    let (entnum, bit) = bank.next_clone_entity();
    // 21 % 16 = 5: lap one, entity five.
    assert_eq!(entnum, BODY_ENTITY_FIRST_SLOT + 5);
    assert_eq!(bit, 0);
    assert_eq!(bank.spawn_ring, 6);
}

#[test]
fn handing_out_a_number_frees_the_slot_that_wore_it() {
    let mut pool = CorpseYard::default();
    pool.slots[3] = worn(BODY_ENTITY_FIRST_SLOT, [1.0, 2.0, 3.0]);
    pool.slots[4] = worn(BODY_ENTITY_FIRST_SLOT + 1, [4.0, 5.0, 6.0]);
    pool.next_clone_entity();
    assert_eq!(pool.slots[3], CorpseBay::default());
    assert!(pool.slots[4].occupied, "a different number keeps its body");
}

// ---- choosing a slot ----

#[test]
fn a_free_slot_is_taken_before_anyone_is_evicted() {
    let mut pool = CorpseYard::default();
    pool.slots[0] = worn(40, [0.0; 3]);
    pool.slots[1] = worn(41, [0.0; 3]);
    assert_eq!(pool.slot_for_new_body([500.0, 0.0, 0.0]), 2);
    assert!(pool.slots[0].occupied && pool.slots[1].occupied);
}

#[test]
fn a_full_pool_gives_up_the_body_farthest_from_the_new_death() {
    let mut pool = pool_of_eight(|i| [i as f32 * 100.0, 0.0, 0.0]);
    let slot = pool.slot_for_new_body([0.0, 0.0, 0.0]);
    assert_eq!(slot, 7);
    assert_eq!(pool.slots[7], CorpseBay::default());
    assert!(pool.slots[6].occupied);
}

#[test]
fn distance_counts_height_too() {
    let mut pool = pool_of_eight(|i| if i == 2 { [0.0, 0.0, 900.0] } else { [50.0, 0.0, 0.0] });
    assert_eq!(pool.slot_for_new_body([0.0; 3]), 2);
}

#[test]
fn equally_far_bodies_lose_the_lowest_slot() {
    let mut pool = pool_of_eight(|_| [10.0, 10.0, 10.0]);
    assert_eq!(pool.slot_for_new_body([0.0; 3]), 0);
}

// ---- bookkeeping ----

#[test]
fn clear_entnum_leaves_other_bodies_alone() {
    let mut pool = CorpseYard::default();
    pool.slots[0] = worn(20, [1.0; 3]);
    pool.slots[5] = worn(21, [2.0; 3]);
    pool.release_entity_slot(20);
    assert_eq!(pool.slots[0], CorpseBay::default());
    assert_eq!(pool.slots[5].entnum, 21);
    pool.release_entity_slot(99);
    assert_eq!(pool.slots[5].entnum, 21, "an unknown number changes nothing");
}

#[test]
fn entnums_lists_each_slot_with_minus_one_for_free_ones() {
    let mut pool = CorpseYard::default();
    pool.slots[1] = worn(22, [0.0; 3]);
    pool.slots[6] = worn(18, [0.0; 3]);
    assert_eq!(pool.slot_numbers(), [-1, 22, -1, -1, -1, -1, 18, -1]);
}

#[test]
fn place_marks_the_body_and_wraps_the_slot_number() {
    let mut pool = CorpseYard::default();
    let used = pool.place(9, 25, CorpseBay::default());
    assert_eq!(used, 1);
    assert!(pool.slots[1].occupied);
    assert_eq!(pool.slots[1].entnum, 25);
}

#[test]
fn level_time_is_ticks_times_the_tick_length() {
    assert_eq!(level_time_ms(StepNo(0)), 0);
    assert_eq!(level_time_ms(StepNo(3)), 3 * SIM_STEP_MILLIS as i32);
    assert_eq!(level_time_ms(StepNo(u32::MAX)), 0, "a tick past i32 range reads as zero");
}

#[test]
fn trajectory_mirrors_the_slots_fields() {
    let slot = CorpseBay {
        tr_type: FALLING,
        tr_time: 700,
        tr_duration: 5,
        tr_delta: [1.0, 2.0, 3.0],
        tr_base: [4.0, 5.0, 6.0],
        ..Default::default()
    };
    let path = slot.trajectory();
    assert_eq!(
        (path.tr_type, path.began_at, path.span_ms, path.rate, path.base_point),
        (FALLING, 700, 5, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0])
    );
}

// ---- the body a death leaves ----

fn dying_player() -> AvatarState {
    let mut ps = AvatarState::ZERO;
    ps.origin = [100.0, 200.0, 30.0];
    ps.look_angles = [10.0, 123.0, 0.0];
    ps.eye_height_now = 40.0;
    ps.weapon = 17;
    ps.velocity = [200.0, -300.0, 55.0];
    ps
}

#[test]
fn a_body_starts_tossed_from_where_the_player_died() {
    let ps = dying_player();
    let body = CorpseBay::left_by(SeatNo(4), &ps, 2000, 0);
    assert_eq!(body.victim, SeatNo(4));
    assert_eq!(body.origin, ps.origin);
    assert_eq!(body.tr_base, ps.origin);
    assert_eq!(body.viewangles, [0.0, 123.0, 0.0], "only the yaw is kept");
    assert_eq!((body.view_height_current, body.weapon), (40.0, 17));
    assert_eq!((body.tr_type, body.tr_time, body.tr_duration), (FALLING, 2000, 0));
    assert!(body.falling);
    assert_eq!(body.ground_entity_num, NO_ENTITY_SLOT);
}

#[test]
fn the_toss_is_capped_sideways_from_above_only() {
    let body = CorpseBay::left_by(SeatNo(0), &dying_player(), 0, 0);
    assert_eq!(body.tr_delta, [80.0, -300.0, 55.0], "fast one way is cut, the other way is not");
    assert_eq!(capped_toss([0.0, 80.0, 9999.0]), [0.0, 80.0, 9999.0]);
    assert!(capped_toss([f32::NAN, 0.0, 0.0])[0].is_nan(), "NaN passes through unchanged");
}

#[test]
fn a_script_held_camera_decides_which_yaw_the_body_faces() {
    let mut ps = dying_player();
    ps.link_weapon_angles = [0.0, 77.0, 0.0];
    assert_eq!(facing_yaw(&ps), 123.0);
    ps.attach_bits |= LINK_FLAGS_FORCE_THIRD_PERSON;
    assert_eq!(facing_yaw(&ps), 77.0);
    assert_eq!(CorpseBay::left_by(SeatNo(0), &ps, 0, 0).viewangles[1], 77.0);
}

#[test]
fn flags_swap_the_teleport_bit_for_the_new_body_bit() {
    let mut ps = dying_player();
    ps.e_flags = 0x100 | CLONE_NEW_BODY_FLAG;
    let first = CorpseBay::left_by(SeatNo(0), &ps, 0, 0);
    assert_eq!(first.e_flags, 0x100);
    let second = CorpseBay::left_by(SeatNo(0), &ps, 0, CLONE_NEW_BODY_FLAG);
    assert_eq!(second.e_flags, 0x100 | CLONE_NEW_BODY_FLAG);
}

// ---- falling ----

/// A body tossed from `origin` at rest, so the arc is pure gravity.
fn dropped_from(spot: [f32; 3]) -> CorpseBay {
    let mut body = CorpseBay { entnum: 20, occupied: true, ..Default::default() };
    body.origin = spot;
    body.viewangles = [0.0, 90.0, 0.0];
    body.tr_type = FALLING;
    body.tr_base = spot;
    body.falling = true;
    body
}

fn open_air(_: [f32; 3], _: [f32; 3], _: u32) -> Trace {
    Trace::new()
}

fn stopped_at(fraction: f32, face: [f32; 3]) -> Trace {
    Trace { fraction, normal: face, ..Trace::new() }
}

#[test]
fn in_open_air_the_body_just_follows_its_arc() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let expected = evaluate_trajectory(&body.trajectory(), 400);
    fall_step(&mut body, 400, open_air);
    assert_eq!(body.origin, expected);
    assert!(body.falling);
    assert_eq!(body.tr_type, FALLING, "nothing changes while it is still in the air");
}

#[test]
fn the_sweep_runs_from_the_body_to_the_arc_with_the_player_mask() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let expected = evaluate_trajectory(&body.trajectory(), 400);
    let mut seen = Vec::new();
    fall_step(&mut body, 400, |from, to, mask| {
        seen.push((from, to, mask));
        Trace::new()
    });
    assert_eq!(seen, vec![([0.0, 0.0, 500.0], expected, PLAYER_BODY_MASK)]);
}

#[test]
fn landing_on_level_ground_stops_the_body_there() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let target = evaluate_trajectory(&body.trajectory(), 400);
    let hit = Trace {
        impact_category: TraceHitType::Entity,
        struck_entity_num: 7,
        ..stopped_at(0.5, [0.0, 0.0, 1.0])
    };
    fall_step(&mut body, 400, |_, _, _| hit);

    let rested_at = hit.finish_location([0.0, 0.0, 500.0], target);
    assert_eq!(body.origin, rested_at);
    assert!(!body.falling);
    assert_eq!(body.tr_type, BLEND);
    assert_eq!(body.tr_base, rested_at);
    assert_eq!((body.tr_time, body.tr_duration, body.tr_delta), (0, 0, [0.0; 3]));
    assert_eq!(body.ground_entity_num, 7);
}

#[test]
fn a_body_resting_on_the_map_reports_the_world_as_its_ground() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let hit = Trace {
        impact_category: TraceHitType::DynEntBrush,
        struck_entity_num: 3,
        ..stopped_at(0.2, [0.0, 0.0, 1.0])
    };
    fall_step(&mut body, 400, |_, _, _| hit);
    assert_eq!(body.ground_entity_num, i32::from(trace_t6::WORLD_ENTITY_SLOT));
}

#[test]
fn lying_on_flat_ground_keeps_the_facing_and_levels_the_body() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    fall_step(&mut body, 400, |_, _, _| stopped_at(0.5, [0.0, 0.0, 1.0]));
    let [pitch, yaw, roll] = body.viewangles;
    assert!(pitch.abs() < 1e-3 && roll.abs() < 1e-3, "pitch {pitch}, roll {roll}");
    assert!((yaw - 90.0).abs() < 1e-3, "yaw {yaw}");
}

#[test]
fn lying_on_a_slope_tilts_the_body_but_keeps_the_heading() {
    // A slope that rises toward +y (the way a body at yaw 90 faces) by about 30 degrees.
    let (s, c) = (0.5f32, 0.866_025_4f32);
    let aim = lying_angles(90.0, [0.0, -s, c]);
    assert!((aim[1] - 90.0).abs() < 1e-2, "yaw {}", aim[1]);
    assert!(aim[0].abs() > 20.0, "pitch {}", aim[0]);
}

#[test]
fn a_ragdoll_settles_into_the_ragdoll_resting_type() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    body.tr_type = LIMP_FALLING;
    fall_step(&mut body, 400, |_, _, _| stopped_at(0.5, [0.0, 0.0, 1.0]));
    assert_eq!(body.tr_type, RAGDOLL_INTERPOLATE);
    assert!(!body.falling);
}

#[test]
fn against_a_wall_the_body_eases_off_and_keeps_falling() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    body.tr_delta = [60.0, 0.0, 0.0];
    let hit = stopped_at(0.4, [-1.0, 0.0, 0.0]);
    let target = evaluate_trajectory(&body.trajectory(), 400);
    let rested_at = hit.finish_location([0.0, 0.0, 500.0], target);

    fall_step(&mut body, 400, |_, _, _| hit);

    assert!(body.falling);
    assert_eq!(body.tr_type, FALLING);
    assert_eq!(body.origin, [rested_at[0] - 1.0, rested_at[1], rested_at[2]]);
    assert_eq!(body.tr_base, body.origin);
    assert_eq!((body.tr_time, body.tr_delta), (400, [0.0; 3]), "the arc restarts from here");
}

#[test]
fn a_ceiling_hit_is_treated_like_a_wall() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    fall_step(&mut body, 400, |_, _, _| stopped_at(0.3, [0.0, 0.0, -1.0]));
    assert!(body.falling);
    assert_eq!(body.tr_time, 400);
}

#[test]
fn a_sweep_that_starts_inside_something_lands_even_at_full_fraction() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let hit = Trace { starts_inside: true, ..stopped_at(1.0, [0.0, 0.0, 1.0]) };
    fall_step(&mut body, 400, |_, _, _| hit);
    assert!(!body.falling);
}

#[test]
fn a_buried_body_is_lifted_and_swept_again_without_the_dropped_clip_bit() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let target = evaluate_trajectory(&body.trajectory(), 400);
    let buried = Trace { fully_embedded: true, starts_inside: true, fraction: 0.0, ..Trace::new() };
    let freed = stopped_at(0.5, [0.0, 0.0, 1.0]);
    let mut calls = Vec::new();
    fall_step(&mut body, 400, |from, to, mask| {
        calls.push((from, to, mask));
        if calls.len() == 1 { buried } else { freed }
    });

    assert_eq!(calls.len(), 2);
    let lifted = [0.0, 0.0, 532.0];
    assert_eq!(calls[1], (lifted, target, PLAYER_BODY_MASK & !0x0001_0000));
    assert_eq!(body.origin, freed.finish_location(lifted, target));
    assert!(!body.falling, "an allsolid first sweep always ends in a rest");
}

#[test]
fn a_body_still_buried_after_the_lift_stays_put_and_keeps_its_facing() {
    let mut body = dropped_from([0.0, 0.0, 500.0]);
    let buried = Trace { fully_embedded: true, starts_inside: true, fraction: 0.0, ..Trace::new() };
    fall_step(&mut body, 400, |_, _, _| buried);
    assert_eq!(body.origin, [0.0, 0.0, 500.0]);
    assert!(!body.falling);
    assert_eq!(body.viewangles, [0.0, 90.0, 0.0], "no surface to lie on, so the angles stay");
}
