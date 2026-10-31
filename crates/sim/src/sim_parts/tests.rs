//! Tests of the world's parts that need no map: the roster, the clock, the blockers, snapshots both ways, and the
//! lag-compensation fallbacks.

use entity_t6::EntitySnapshot;

use super::*;
use crate::shot_trace::{LiveStateCause, RewindSampleRuling, ShotRecordOrigin};
use crate::match_ids::{GameRoundStage, ThrownId};
use crate::tick_publish::TickSnapshot;

fn two_client_world() -> MatchSimulation {
    let mut world = MatchSimulation::new();
    world.client_meta_mut(SeatNo(2));
    world.client_meta_mut(SeatNo(7));
    world
}

fn snapshot_of(world: &MatchSimulation, tick: u32) -> TickSnapshot {
    world.snapshot_plus_live_rows(StepNo(tick), Vec::new(), Vec::new(), Vec::new(), Vec::new())
}

// ---- player-state helpers -------------------------------------------------------------------------------------

#[test]
fn weapon_names_with_akimbo_are_dual_wield() {
    assert!(script_gun_wants_two_hands("kard_akimbo_mp"));
    assert!(!script_gun_wants_two_hands("kard_mp"));
}

#[test]
fn inventory_takes_a_weapon_once_and_never_the_empty_one() {
    let mut state = empty_avatar();
    pack_weapon(&mut state, 0, false);
    assert!(state.weapons.iter().all(|slot| *slot == 0));

    pack_weapon(&mut state, 12, false);
    pack_weapon(&mut state, 12, false);
    pack_weapon(&mut state, 30, false);
    assert_eq!(state.weapons.iter().filter(|slot| **slot == 12).count(), 1);
    assert_eq!(state.weapons.iter().position(|slot| *slot == 12), Some(0));
    assert_eq!(state.weapons.iter().position(|slot| *slot == 30), Some(1));
}

#[test]
fn giving_the_empty_weapon_empties_the_hands() {
    let mut state = empty_avatar();
    grant_dual_wield_weapon(&mut state, 12, false);
    assert_eq!((state.weapon, state.weapon_primary), (12, 12));

    grant_dual_wield_weapon(&mut state, 0, false);
    assert_eq!((state.weapon, state.weapon_primary, state.previous_gun_hand), (0, 0, 0));
}

#[test]
fn a_spawned_player_stands_where_asked_with_full_health() {
    let ps = fresh_avatar_at([10.0, 20.0, 30.0], [0.0, 90.0, 0.0]);
    assert_eq!(ps.origin, [10.0, 20.0, 30.0]);
    assert_eq!(ps.look_angles, [0.0, 90.0, 0.0]);
    assert_eq!((ps.health, ps.hit_point_ceiling), (100, 100));
    assert_eq!(ps.body_index, -1);
}

// ---- roster and clock ------------------------------------------------------------------------------------------

#[test]
fn a_new_client_row_is_blank_and_listed_in_order() {
    let mut world = MatchSimulation::new();
    for id in [9, 3, 5] {
        world.client_meta_mut(SeatNo(id));
    }
    assert_eq!(world.client_count(), 3);
    assert_eq!(world.ordered_player_ids(), [SeatNo(3), SeatNo(5), SeatNo(9)]);
    assert!(world.client_meta(SeatNo(4)).is_none());
    assert_eq!(world.packed_client_name(SeatNo(4)), [0; 16]);
}

#[test]
fn the_last_command_a_client_sent_is_remembered_per_client() {
    let mut world = MatchSimulation::new();
    world.store_prior_input(SeatNo(1), 0b101, [1, 2, 3]);
    world.set_old_cmd_move(SeatNo(1), [127, -127]);
    world.set_old_buttons(SeatNo(2), 8);

    assert_eq!(world.last_tick_buttons(SeatNo(1)), Some(0b101));
    assert_eq!(world.old_cmd_angles(SeatNo(1)), Some([1, 2, 3]));
    assert_eq!(world.old_cmd_move(SeatNo(1)), Some([127, -127]));
    assert_eq!(world.last_tick_buttons(SeatNo(2)), Some(8));
    assert_eq!(world.old_cmd_angles(SeatNo(2)), None);
}

#[test]
fn ids_count_up_and_a_projectile_id_never_comes_out_as_zero() {
    let mut world = MatchSimulation::new();
    let first = world.alloc_shot_id();
    assert_eq!(world.alloc_shot_id(), first.next());

    world.serials.projectile = ThrownId(u32::MAX);
    assert_eq!(world.reserve_projectile_slot(), ThrownId(u32::MAX));
    assert_eq!(world.reserve_projectile_slot(), ThrownId(1));
}

#[test]
fn a_new_world_waits_in_warmup_and_is_not_running() {
    let world = MatchSimulation::new();
    assert!(!world.is_running());
    assert_eq!(world.phase(), GameRoundStage::Warmup);
    assert_eq!(world.match_elapsed_ms(), 0);
    assert!(world.emits_state_snapshot());
}

#[test]
fn a_prediction_world_stops_publishing() {
    let mut world = MatchSimulation::new();
    world.hold_wire_meta();
    assert!(!world.emits_state_snapshot());
}

// ---- blockers --------------------------------------------------------------------------------------------------

#[test]
fn a_mantle_blocker_ends_where_its_owner_said_and_goes_after_its_time() {
    let mut world = MatchSimulation::new();
    let blocker = |owner, ends, until| MantleBlocker { owner, spot: [0.0; 3], ends, until };
    world.add_mantle_blocker(blocker(4, 1000, 1500));
    world.add_mantle_blocker(blocker(5, 1200, 1700));
    world.add_mantle_blocker(blocker(4, 2000, 2500));

    assert_eq!(world.mantle_end_time(4), Some(2000), "the newest climb of the owner answers");
    assert_eq!(world.mantle_end_time(9), None);

    world.prune_mantle_blockers(1500);
    assert_eq!(world.mantle_blockers().len(), 2, "a blocker whose time is exactly up is gone");
    world.prune_mantle_blockers(10_000);
    assert!(world.mantle_blockers().is_empty());
}

#[test]
fn tactical_insertion_boxes_are_replaced_whole() {
    let mut world = MatchSimulation::new();
    world.set_tac_insert_blockers(vec![([0.0; 3], [1.0; 3])]);
    world.set_tac_insert_blockers(vec![([2.0; 3], [3.0; 3]), ([4.0; 3], [5.0; 3])]);
    assert_eq!(world.tac_insert_blockers().len(), 2);
    assert_eq!(world.tac_insert_blockers()[0].0, [2.0; 3]);
}

// ---- snapshots -------------------------------------------------------------------------------------------------

#[test]
fn a_snapshot_lists_every_client_and_the_clock() {
    let mut world = two_client_world();
    world.clock.elapsed_ms = 4321;
    let snapshot = snapshot_of(&world, 12);

    assert_eq!(snapshot.tick, StepNo(12));
    let listed: Vec<SeatNo> = snapshot.meta.clients.iter().map(|(seat, _)| *seat).collect();
    assert_eq!(listed, [SeatNo(2), SeatNo(7)]);
    assert_eq!(snapshot.meta.match_clock_ms, 4321);
    assert_eq!(snapshot.meta.rng, world.random_debug_info());
}

#[test]
fn an_empty_world_snapshots_without_entities() {
    let snapshot = snapshot_of(&MatchSimulation::new(), 1);
    assert!(snapshot.meta.entities.is_empty());
    assert!(snapshot.meta.item_ammo.is_empty());
    assert!(snapshot.meta.script_movers.is_empty());
}

#[test]
fn adopting_a_snapshot_brings_the_clients_and_the_clock_over() {
    let mut authority = two_client_world();
    authority.clock.elapsed_ms = 777;
    let snapshot = snapshot_of(&authority, 3);

    let mut copy = MatchSimulation::new();
    copy.client_meta_mut(SeatNo(40));
    let (summary, shells, movers, items) = copy.adopt_snapshot_state(&snapshot);

    assert_eq!(summary.seats, 2);
    assert!(!summary.content_differs);
    assert_eq!(summary.let_go, 0, "the one client that was not in the snapshot was replaced, not lost");
    assert!(shells.is_empty() && movers.is_empty() && items.is_empty());
    assert_eq!(copy.ordered_player_ids(), [SeatNo(2), SeatNo(7)]);
    assert_eq!(copy.match_elapsed_ms(), 777);
    assert!(copy.is_running());
    assert_eq!(copy.random_debug_info(), authority.random_debug_info());
}

#[test]
fn a_snapshot_from_another_seed_is_reported_as_a_content_mismatch() {
    let authority = two_client_world();
    let mut copy = MatchSimulation::new();
    copy.dice.seed = 99;
    let (summary, ..) = copy.adopt_snapshot_state(&snapshot_of(&authority, 1));
    assert!(summary.content_differs);
}

#[test]
fn a_prediction_world_keeps_only_its_own_client() {
    let authority = two_client_world();
    let snapshot = snapshot_of(&authority, 8);

    let mut prediction = MatchSimulation::new();
    let (summary, ..) = prediction.adopt_prediction_snapshot_state(&snapshot, SeatNo(7));
    assert_eq!(summary.seats, 1);
    assert_eq!(prediction.ordered_player_ids(), [SeatNo(7)]);
    assert!(prediction.prediction_remote_bodies().is_empty());
}

#[test]
fn adopting_forgets_the_last_commands() {
    let authority = two_client_world();
    let mut copy = MatchSimulation::new();
    copy.store_prior_input(SeatNo(2), 1, [0; 3]);
    copy.set_old_cmd_move(SeatNo(2), [1, 1]);
    copy.adopt_snapshot_state(&snapshot_of(&authority, 1));
    assert_eq!(copy.last_tick_buttons(SeatNo(2)), None);
    assert_eq!(copy.old_cmd_angles(SeatNo(2)), None);
    assert_eq!(copy.old_cmd_move(SeatNo(2)), None);
}

#[test]
#[should_panic(expected = "ET_ITEM does not occupy an Item kernel slot")]
fn a_snapshot_with_an_item_the_kernel_does_not_know_is_refused() {
    let mut snapshot = snapshot_of(&MatchSimulation::new(), 1);
    snapshot.meta.entities.push(EntitySnapshot { slot_index: 40, e_type: entity_t6::et::ITEM, ..EntitySnapshot::default() });
    MatchSimulation::new().adopt_snapshot_state(&snapshot);
}

#[test]
#[should_panic(expected = "ET_MISSILE does not occupy a Missile kernel slot")]
fn a_snapshot_with_a_missile_the_kernel_does_not_know_is_refused() {
    let mut snapshot = snapshot_of(&MatchSimulation::new(), 1);
    snapshot.meta.entities.push(EntitySnapshot { slot_index: 41, e_type: entity_t6::et::MISSILE, ..EntitySnapshot::default() });
    MatchSimulation::new().adopt_snapshot_state(&snapshot);
}

#[test]
fn a_prediction_world_takes_on_the_authoritys_map_and_seed() {
    let mut authority = MatchSimulation::new();
    authority.dice.seed = 0xBEEF;
    authority.refresh_digest();

    let mut prediction = MatchSimulation::new();
    prediction.initialize_prediction_from(&authority);
    assert_eq!(prediction.root_seed(), 0xBEEF);
    assert_eq!(prediction.content_fingerprint(), authority.content_fingerprint());
}

// ---- lag compensation ------------------------------------------------------------------------------------------

#[test]
fn a_shot_without_a_claim_is_judged_against_the_present() {
    let world = two_client_world();
    let query = world.rewind_request_for(SeatNo(2), StepNo(10), |_| None);

    let expected = RewindSampleRuling::LiveNow { tick: StepNo(10), reason: LiveStateCause::ShooterNamedNoTick };
    assert_eq!(query.players.verdict, expected);
    assert_eq!(query.entities.verdict, expected);
    assert!(query.players.bodies_then.is_empty());
}

#[test]
fn a_command_without_a_claim_picks_the_no_claim_sample() {
    let mut world = MatchSimulation::new();
    world.store_rewind_inputs([((SeatNo(1), 100), ShotRecordOrigin::UNCLAIMED)]);
    world.pick_rewind_order(SeatNo(1), 100);
    assert_eq!(world.lagcomp_sample(SeatNo(1)), Some(ShotRecordOrigin::UNCLAIMED));

    world.pick_rewind_order(SeatNo(3), 555);
    assert_eq!(world.lagcomp_sample(SeatNo(3)), Some(ShotRecordOrigin::UNCLAIMED));
}

/// A world with frames for ticks 1..=20 and `claim` as seat 2's shot claim.
fn world_with_history(claim: ShotRecordOrigin) -> MatchSimulation {
    world_with_poses(claim, |_| Vec::new())
}

/// [`world_with_history`], with `poses_at(tick)` as the bodies kept in each frame.
fn world_with_poses(claim: ShotRecordOrigin, poses_at: impl Fn(u32) -> Vec<crate::shot_trace::PlayerHitboxPose>) -> MatchSimulation {
    world_with_frames(claim, 1..=20, poses_at, |_| Vec::new())
}

/// A world with frames for `ticks`: `poses_at(tick)` as the bodies and `rows_at(tick)` as the entities kept in each.
fn world_with_frames(
    claim: ShotRecordOrigin,
    ticks: std::ops::RangeInclusive<u32>,
    poses_at: impl Fn(u32) -> Vec<crate::shot_trace::PlayerHitboxPose>,
    rows_at: impl Fn(u32) -> Vec<crate::shot_trace::EntityRayShape>,
) -> MatchSimulation {
    let mut world = two_client_world();
    for tick in ticks {
        world.replay.history.push(StepNo(tick), poses_at(tick));
        world.replay.entity_history.record_sample(crate::shot_trace::EntityHitboxSnapshot {
            tick: StepNo(tick),
            phase: crate::shot_trace::RewindStage::AfterPlayerMove,
            rows: rows_at(tick),
        });
    }
    world.set_lagcomp_sample(SeatNo(2), claim);
    world
}

fn exact_claim(left: u32, right: u32) -> ShotRecordOrigin {
    ShotRecordOrigin { left: StepNo(left), right: StepNo(right), opacity: 0.0, quality: crate::shot_trace::ShotRecordGrade::Exact }
}

fn verdict_at_20(claim: ShotRecordOrigin) -> RewindSampleRuling {
    let query = world_with_history(claim).rewind_request_for(SeatNo(2), StepNo(20), |_| None);
    assert_eq!(query.players.verdict, query.entities.verdict);
    query.players.verdict
}

#[test]
fn the_rewind_window_is_bo2s_400_ms_and_its_skip_is_50_ms() {
    assert_eq!(crate::shot_trace::SHOT_STEP_MS, 50);
    assert_eq!(crate::shot_trace::ANTILAG_MAX_MS, 400);
    assert_eq!(crate::shot_trace::ANTILAG_SKIP_MS, 50);
    assert_eq!(crate::MAX_COMPENSATION_STEPS, 8);
}

#[test]
fn a_shot_400_ms_back_is_rewound_to_that_tick() {
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    assert_eq!(verdict_at_20(exact_claim(12, 12)), RewindSampleRuling::Exact { requested: StepNo(12), frame: StepNo(12), phase });
}

#[test]
fn a_shot_older_than_400_ms_is_judged_400_ms_back() {
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    let expected = RewindSampleRuling::Clamped {
        requested: StepNo(5),
        used: StepNo(12),
        frame: StepNo(12),
        phase,
        reason: crate::shot_trace::RewindLimitCause::BeyondRewindLimit,
    };
    assert_eq!(verdict_at_20(exact_claim(5, 6)), expected);
}

#[test]
fn a_shot_50_ms_back_is_judged_against_the_present() {
    let expected = RewindSampleRuling::LiveNow { tick: StepNo(20), reason: LiveStateCause::TooRecentToRewind { requested: StepNo(19) } };
    assert_eq!(verdict_at_20(exact_claim(19, 20)), expected);
}

#[test]
fn a_shot_100_ms_back_is_rewound() {
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    assert_eq!(verdict_at_20(exact_claim(18, 19)), RewindSampleRuling::Exact { requested: StepNo(18), frame: StepNo(18), phase });
}

#[test]
fn a_shot_between_two_snapshots_is_counted_from_the_earlier_one() {
    // BO2 rewinds to the shooter's own command time (`client->lastServerTime`), between the two kept frames around
    // it; the verdict names the earlier of the pair.
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    let claim = ShotRecordOrigin { opacity: 0.5, quality: crate::shot_trace::ShotRecordGrade::SmoothedBetweenFrames, ..exact_claim(14, 15) };
    assert_eq!(verdict_at_20(claim), RewindSampleRuling::Exact { requested: StepNo(14), frame: StepNo(14), phase });
}

/// Seat `client` standing at `x` facing `yaw`, with one box 10 units in front of him at chest height.
fn body_at(client: u32, x: f32, yaw: f32) -> crate::shot_trace::PlayerHitboxPose {
    let (sin, cos) = yaw.to_radians().sin_cos();
    crate::shot_trace::PlayerHitboxPose {
        client: SeatNo(client),
        origin: [x, 0.0, 0.0],
        yaw,
        lo: [-15.0, -15.0, 0.0],
        hi: [15.0, 15.0, 72.0],
        life_sequence: crate::match_ids::LifeCount(1),
        hit_volume: crate::shot_trace::HitboxShapeKind::Standing,
        joints: vec![xmodel_runtime::HitShapeBone {
            bone: 3,
            body_part_category: 1,
            center: [x + 10.0 * cos, 10.0 * sin, 40.0],
            axes: [[cos, sin, 0.0], [-sin, cos, 0.0], [0.0, 0.0, 1.0]],
            half_ext: [4.0, 4.0, 8.0],
        }],
        shield: None,
        shield_tag: None,
    }
}

/// Seat 9 walks 20 units a tick along x and turns 90 degrees between ticks 14 and 15.
fn walking_body(tick: u32) -> Vec<crate::shot_trace::PlayerHitboxPose> {
    vec![body_at(9, tick as f32 * 20.0, if tick >= 15 { 90.0 } else { 0.0 })]
}

fn smoothed_claim(left: u32, right: u32, opacity: f32) -> ShotRecordOrigin {
    ShotRecordOrigin { opacity, quality: crate::shot_trace::ShotRecordGrade::SmoothedBetweenFrames, ..exact_claim(left, right) }
}

fn bodies_at_20(claim: ShotRecordOrigin, poses_at: impl Fn(u32) -> Vec<crate::shot_trace::PlayerHitboxPose>) -> crate::shot_trace::RewindSample {
    world_with_poses(claim, poses_at).rewind_request_for(SeatNo(2), StepNo(20), |_| None).players
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn a_shot_between_two_snapshots_sees_the_bodies_between_them() {
    // 725 ms: halfway from tick 14 (x 280, facing 0) to tick 15 (x 300, facing 90).
    let sample = bodies_at_20(smoothed_claim(14, 15, 0.5), walking_body);
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    assert_eq!(sample.verdict, RewindSampleRuling::Exact { requested: StepNo(14), frame: StepNo(14), phase });
    let [body] = sample.bodies_then.as_slice() else { panic!("one body") };
    assert!(close(body.origin, [290.0, 0.0, 0.0]));
    assert!((body.yaw - 45.0).abs() < 1e-3);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    assert!(close(body.joints[0].center, [290.0 + 10.0 * half, 10.0 * half, 40.0]));
    assert!(close(body.joints[0].axes[0], [half, half, 0.0]));
    assert_eq!(body.joints[0].half_ext, [4.0, 4.0, 8.0]);
}

#[test]
fn a_shot_on_the_earlier_snapshot_sees_that_snapshot_unchanged() {
    let sample = bodies_at_20(smoothed_claim(14, 15, 0.0), walking_body);
    assert_eq!(sample.bodies_then, walking_body(14));
}

#[test]
fn a_shot_on_the_later_snapshot_sees_that_snapshot_unchanged() {
    let sample = bodies_at_20(smoothed_claim(14, 15, 1.0), walking_body);
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    assert_eq!(sample.verdict, RewindSampleRuling::Exact { requested: StepNo(14), frame: StepNo(15), phase });
    assert_eq!(sample.bodies_then, walking_body(15));
}

#[test]
fn a_body_kept_in_only_one_of_the_two_snapshots_stands_where_it_was() {
    let poses_at = |tick| match tick {
        14 => vec![body_at(10, 5.0, 30.0)],
        15 => vec![body_at(11, -5.0, -30.0)],
        _ => Vec::new(),
    };
    let sample = bodies_at_20(smoothed_claim(14, 15, 0.5), poses_at);
    assert_eq!(sample.bodies_then, vec![body_at(10, 5.0, 30.0), body_at(11, -5.0, -30.0)]);
}

#[test]
fn a_shot_held_to_400_ms_sees_that_snapshot_unchanged() {
    let sample = bodies_at_20(smoothed_claim(5, 6, 0.5), walking_body);
    assert_eq!(sample.bodies_then, walking_body(12));
}

#[test]
fn a_smoothed_shot_within_50_ms_is_judged_against_the_present() {
    // 960 ms is 40 ms back; 925 ms is 75 ms back and is rewound.
    let live = RewindSampleRuling::LiveNow { tick: StepNo(20), reason: LiveStateCause::TooRecentToRewind { requested: StepNo(19) } };
    assert_eq!(verdict_at_20(smoothed_claim(19, 20, 0.2)), live);
    let phase = crate::shot_trace::RewindStage::AfterPlayerMove;
    assert_eq!(verdict_at_20(smoothed_claim(18, 19, 0.5)), RewindSampleRuling::Exact { requested: StepNo(18), frame: StepNo(18), phase });
}

#[test]
fn the_shooters_moment_is_his_blend_between_the_two_snapshots() {
    assert_eq!(smoothed_claim(14, 15, 0.5).drawn_at_ms(50), 725);
    assert_eq!(smoothed_claim(14, 16, 0.25).drawn_at_ms(50), 725);
    assert_eq!(exact_claim(12, 12).drawn_at_ms(50), 600);
}

/// Script model `id` as a door: one brush at `x` along the shot line, turned `yaw`.
fn door_at(id: u32, x: f32, yaw: f32) -> crate::shot_trace::EntityRayShape {
    crate::shot_trace::EntityRayShape {
        owner: crate::shot_trace::ServerModelSource::ScriptModel(crate::match_ids::ScriptedModel(id)),
        epoch: crate::shot_trace::EntityCollisionMoment::LiveMoment,
        collision: None,
        model_content_mask: None,
        mesh_asset_name: None,
        attached_solids: vec![crate::shot_trace::AttachedSolidShape { collision_model_ref: 2, origin: [x, 0.0, 0.0], turn: [0.0, yaw, 0.0] }],
        placed_at: None,
    }
}

/// Door 4 slides 20 units a tick along x and swings 10 degrees a tick.
fn sliding_door(tick: u32) -> Vec<crate::shot_trace::EntityRayShape> {
    vec![door_at(4, tick as f32 * 20.0, tick as f32 * 10.0)]
}

fn doors_at_20(claim: ShotRecordOrigin, ticks: std::ops::RangeInclusive<u32>) -> crate::shot_trace::EntityHitboxReading {
    world_with_frames(claim, ticks, |_| Vec::new(), sliding_door).rewind_request_for(SeatNo(2), StepNo(20), |_| None).entities
}

fn door_place(reading: &crate::shot_trace::EntityHitboxReading) -> ([f32; 3], f32) {
    let [door] = reading.rows.as_slice() else { panic!("one door") };
    (door.attached_solids[0].origin, door.attached_solids[0].turn[1])
}

fn past_frame(tick: u32) -> crate::shot_trace::EntityCollisionMoment {
    crate::shot_trace::EntityCollisionMoment::PastFrame { frame: StepNo(tick) }
}

#[test]
fn a_shot_between_two_snapshots_sees_the_movers_between_them() {
    // 725 ms: halfway from tick 14 (x 280, 140 degrees) to tick 15 (x 300, 150 degrees).
    let reading = doors_at_20(smoothed_claim(14, 15, 0.5), 1..=20);
    let (origin, yaw) = door_place(&reading);
    assert!(close(origin, [290.0, 0.0, 0.0]) && (yaw - 145.0).abs() < 1e-3);
    assert_eq!(reading.rows[0].epoch, past_frame(14));
}

#[test]
fn a_shot_on_a_snapshot_sees_the_movers_as_kept_there() {
    for (claim, tick) in [(smoothed_claim(14, 15, 0.0), 14), (smoothed_claim(14, 15, 1.0), 15)] {
        let reading = doors_at_20(claim, 1..=20);
        assert_eq!(reading.rows, vec![crate::shot_trace::EntityRayShape { epoch: past_frame(tick), ..door_at(4, tick as f32 * 20.0, tick as f32 * 10.0) }]);
    }
}

#[test]
fn a_mover_kept_in_only_one_of_the_two_snapshots_stands_where_it_was() {
    let rows_at = |tick| match tick {
        14 => vec![door_at(5, 5.0, 30.0)],
        15 => vec![door_at(6, -5.0, -30.0)],
        _ => Vec::new(),
    };
    let reading = world_with_frames(smoothed_claim(14, 15, 0.5), 1..=20, |_| Vec::new(), rows_at)
        .rewind_request_for(SeatNo(2), StepNo(20), |_| None)
        .entities;
    let stamped = |row: crate::shot_trace::EntityRayShape| crate::shot_trace::EntityRayShape { epoch: past_frame(14), ..row };
    assert_eq!(reading.rows, vec![stamped(door_at(5, 5.0, 30.0)), stamped(door_at(6, -5.0, -30.0))]);
}

#[test]
fn movers_are_not_held_to_400_ms() {
    // The players stop at tick 12; BO2's movers go back to the 275 ms the shooter drew, between ticks 5 and 6.
    let reading = doors_at_20(smoothed_claim(5, 6, 0.5), 1..=20);
    assert!(matches!(reading.verdict, RewindSampleRuling::Clamped { used: StepNo(12), .. }));
    let (origin, yaw) = door_place(&reading);
    assert!(close(origin, [110.0, 0.0, 0.0]) && (yaw - 55.0).abs() < 1e-3);
}

#[test]
fn a_shot_older_than_every_kept_snapshot_sees_the_movers_in_the_oldest() {
    let reading = doors_at_20(smoothed_claim(5, 6, 0.5), 10..=20);
    let (origin, yaw) = door_place(&reading);
    assert!(close(origin, [200.0, 0.0, 0.0]) && (yaw - 100.0).abs() < 1e-3);
    assert_eq!(reading.rows[0].epoch, past_frame(10));
}

#[test]
fn a_shot_within_50_ms_still_sees_the_movers_where_he_drew_them() {
    // 960 ms: the players are judged as they are now; the door is 20% of the way from tick 19 to tick 20.
    let reading = doors_at_20(smoothed_claim(19, 20, 0.2), 1..=20);
    assert!(matches!(reading.verdict, RewindSampleRuling::LiveNow { reason: LiveStateCause::TooRecentToRewind { .. }, .. }));
    let (origin, yaw) = door_place(&reading);
    assert!(close(origin, [384.0, 0.0, 0.0]) && (yaw - 192.0).abs() < 1e-3);
}

#[test]
fn a_shot_after_the_newest_kept_snapshot_sees_the_movers_in_the_newest() {
    // 925 ms, with nothing kept after tick 18.
    let reading = doors_at_20(smoothed_claim(18, 19, 0.5), 1..=18);
    assert_eq!(door_place(&reading), ([360.0, 0.0, 0.0], 180.0));
    assert_eq!(reading.rows[0].epoch, past_frame(18));
}

#[test]
fn the_lunge_target_is_what_the_client_last_saw() {
    let mut world = MatchSimulation::new();
    world.set_lunge_seen([(SeatNo(1), [1.0, 2.0, 3.0], [0.0, 0.0, 1.0])]);
    assert_eq!(world.lunge_seen(SeatNo(1)), Some(([1.0, 2.0, 3.0], [0.0, 0.0, 1.0])));
    world.set_lunge_seen([]);
    assert_eq!(world.lunge_seen(SeatNo(1)), None);
}

// ---- census ----------------------------------------------------------------------------------------------------

#[test]
fn the_hitvol_dump_has_one_blank_row_before_any_body_is_recorded() {
    let rows = MatchSimulation::new().hitvol_dump(|_| None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].client, None);
    assert_eq!(rows[0].shape_data, "none");
    assert_eq!(rows[0].bones_total, 0);
}
