use entity_t6::Trajectory;
use entity_t6::flight::path_kind::STRAIGHT;

use super::bounce::{comes_to_rest, unarmed_dud};
use super::facts::GadgetLiveStats;
use super::launch::{Spin, spin_of, throw_velocity};
use super::roll::roll_grenade;
use super::flying::{GrenadeLaunchKind, FlyingProjectile};
use super::vec3::norm;
use crate::match_ids::LifeCount;
use crate::{SeatNo, ThrownId, StepNo, level_time_ms};

/// The all-zero equipment row the tests below change one figure of.
fn stock() -> GadgetLiveStats {
    GadgetLiveStats::default()
}

/// A live missile at `origin` moving at `velocity` on a straight path from tick zero, with no deadlines.
fn missile_at(spot: [f32; 3], speed_vec: [f32; 3]) -> FlyingProjectile {
    let path = Trajectory { tr_type: STRAIGHT, began_at: 0, span_ms: 0, base_point: spot, rate: speed_vec };
    FlyingProjectile {
        id: ThrownId(1),
        owner: SeatNo(0),
        owner_respawn_count: LifeCount(0),
        gun: 1,
        ent_no: 70,
        origin: spot,
        motion: speed_vec,
        at: path,
        turn_path: Trajectory::default(),
        spawn_moment: 0,
        spawn_time_ms: 0,
        detonate_at_ms: None,
        cleanup_at_ms: i32::MAX,
        travel_distance: 0.0,
        in_flight: true,
        stuck_pane: None,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    }
}

/// A floor at z = 0: where a segment going from `from` to `to` meets it, facing up.
fn flat_floor(from: [f32; 3], till: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
    if till[2] > 0.0 {
        return None;
    }
    let along = from[2] / (from[2] - till[2]);
    Some((core::array::from_fn(|axis| from[axis] + (till[axis] - from[axis]) * along), [0.0, 0.0, 1.0]))
}

/// A frag (body radius 1.37) set rolling along +x at `speed`. Each tick it moves along its line and takes one floor hit;
/// returns (tick, distance rolled, speed) per tick until it rests.
fn roll_out(speed: f32) -> Vec<(u32, f32, f32)> {
    let mut frag = missile_at([0.0, 0.0, 1.37], [speed, 0.0, 0.0]);
    let mut log = vec![(0, 0.0, speed)];
    for k in 1..400 {
        let tick = StepNo(k);
        let at_ms = level_time_ms(tick);
        let before = frag.origin;
        frag.origin = frag.spot_at(at_ms);
        let step = core::array::from_fn(|axis| frag.origin[axis] - before[axis]);
        frag.travel_distance += norm(step);
        let motion_vec = frag.speed_vector_at(at_ms);
        // Free flight clears the ground flag every tick; the roll has to set it again or a ground-only fuse would never
        // go off.
        frag.grounded = false;
        // A bump roll of 1.0 never bumps, so the decay reads cleanly.
        let rested = roll_grenade(tick, &mut frag, 1.37, None, motion_vec, 1.0, &flat_floor);
        assert!(frag.grounded, "not grounded while rolling at tick {k}");
        if rested {
            log.push((k, frag.travel_distance, 0.0));
            return log;
        }
        log.push((k, frag.travel_distance, norm(frag.motion)));
    }
    log
}

#[test]
fn a_rolling_frag_loses_speed_each_hit_then_rests() {
    let log = roll_out(300.0);
    // Above 100 a hit keeps 0.6 of the speed (the wobble adds up to 10 either way).
    assert!((log[1].2 - 180.0).abs() < 12.0, "{:?}", &log[..3]);
    assert!((log[2].2 - 108.0).abs() < 12.0, "{:?}", &log[..3]);
    // At 100 or below a hit keeps 0.99 and the wobble swings it about 20 either way, so it never regains its speed.
    let rest = log.iter().position(|row| row.2 == 0.0).expect("never rested");
    assert!(log[3..rest].iter().all(|row| row.2 < 75.0), "{:?}", &log[3..rest.min(12)]);
    // A wobble's low takes it under the rest speed (20) once its mean is in the twenties: about a second and a half and
    // 80 along its path.
    let (tick, rolled, _) = log[rest];
    assert!((20..=80).contains(&tick), "rested at tick {tick}");
    assert!((50.0..=200.0).contains(&rolled), "rolled {rolled}");
}

#[test]
fn a_launcher_round_takes_only_the_speed_and_the_lift() {
    let facts = GadgetLiveStats {
        missile_velocity: 1000,
        missile_upward_velocity: 100,
        projectile_speed_forward: 50,
        projectile_speed_relative_up: 120,
        ..Default::default()
    };
    // Aimed level: a throw adds the flat push and the rise, a launcher does not.
    assert_eq!(throw_velocity([0.0; 3], &facts, [0.0; 3], false), [1050.0, 0.0, 220.0]);
    assert_eq!(throw_velocity([0.0; 3], &facts, [0.0; 3], true), [1000.0, 0.0, 100.0]);
}

#[test]
fn an_unarmed_round_duds_head_on_and_bounces_off_at_a_glance() {
    let launcher = GadgetLiveStats {
        gun_family_code: i32::from(weapon_t6::facts::gun_family_code::PROJECTILE),
        shell_arming_range: 300,
        ..Default::default()
    };
    let wall = [-1.0, 0.0, 0.0];
    let duds = |velocity: [f32; 3], flesh: bool| {
        unarmed_dud(&launcher, &missile_at([0.0; 3], velocity), StepNo(1), 0.5, wall, flesh)
    };
    assert!(duds([1200.0, 0.0, 0.0], false), "head-on and fast");
    assert!(duds([1200.0, 1000.0, 0.0], false), "within 45 degrees of head-on");
    assert!(!duds([1200.0, 1300.0, 0.0], false), "past 45 degrees");
    assert!(!duds([450.0, 0.0, 0.0], false), "under 500 even head-on");
    assert!(duds([300.0, 900.0, 0.0], true), "flesh always crumples it");
}

#[test]
fn only_projectiles_crumple_and_sticky_ones_never_bounce() {
    let thrown = GadgetLiveStats { gun_family_code: 1, ..stock() };
    let round = missile_at([0.0; 3], [1200.0, 0.0, 0.0]);
    let wall = [-1.0, 0.0, 0.0];
    assert!(!unarmed_dud(&thrown, &round, StepNo(1), 0.5, wall, true));
    for stickiness in [1, 5] {
        let sticky = GadgetLiveStats { stickiness, ..thrown };
        assert!(unarmed_dud(&sticky, &round, StepNo(1), 0.5, wall, false), "stickiness {stickiness}");
    }
}

#[test]
fn the_weapon_file_decides_what_spins() {
    let thrown = GrenadeLaunchKind::Thrown { remaining_fuse_ms: None };
    let bo2 = GadgetLiveStats { projectile_parent_velocity: Some(0.0), ..stock() };
    // The tomahawk (rotate 1, rotateType 1) turns end over end at 1500 a second.
    let hatchet = GadgetLiveStats { projectile_rotates: true, projectile_rotate_type: 1, ..bo2 };
    assert_eq!(spin_of(&hatchet, thrown), Spin::Fixed { pitch: 1500.0, roll: 0.0 });
    // A frag (rotate 1, rotateType 0) tumbles at whole degrees a second.
    let frag = GadgetLiveStats { projectile_rotates: true, ..bo2 };
    assert_eq!(
        spin_of(&frag, thrown),
        Spin::Tumble { pitch: (340.0, 800.0), roll: (180.0, 540.0), whole: true }
    );
    // A claymore (rotate 0) never turns, and nothing a launcher fires does.
    assert_eq!(spin_of(&bo2, thrown), Spin::None);
    assert_eq!(spin_of(&hatchet, GrenadeLaunchKind::Launcher), Spin::None);
    // A throwing knife without the parent-velocity row keeps its own blade spin.
    let knife = GadgetLiveStats { class_code: 9, ..stock() };
    assert_eq!(spin_of(&knife, thrown), Spin::Fixed { pitch: entity_t6::missile::BLADE_SPIN_PITCH, roll: 0.0 });
}

#[test]
fn a_bounced_item_lies_where_it_slowed_to_a_stop() {
    let floor = [0.0, 0.0, 1.0];
    let wall = [-1.0, 0.0, 0.0];
    // A frag (stickiness 0) and a tomahawk (5) alike: slow off a floor, or all but stopped anywhere, it rests.
    for stickiness in [0, 5] {
        let facts = GadgetLiveStats { stickiness, ..stock() };
        assert!(comes_to_rest(&facts, 15.0, floor), "stickiness {stickiness}");
        assert!(comes_to_rest(&facts, 0.5, wall), "stickiness {stickiness}");
        assert!(!comes_to_rest(&facts, 15.0, wall), "stickiness {stickiness}");
        assert!(!comes_to_rest(&facts, 120.0, floor), "stickiness {stickiness}");
    }
    // A rolling grenade rests in its roll, not off a bounce.
    let rolling = GadgetLiveStats { rolling_radius: Some(1.37), ..stock() };
    assert!(!comes_to_rest(&rolling, 15.0, floor));
}
