use entity_t6::Trajectory;
use entity_t6::trajectory::tr_type::LINEAR;

use super::bounce::{comes_to_rest, unarmed_dud};
use super::facts::EquipmentRuntimeFacts;
use super::launch::{Spin, spin_of, throw_velocity};
use super::roll::roll_grenade;
use super::state::{GrenadeLaunchKind, ProjectileState};
use super::vec3::norm;
use crate::identities::LifeSequence;
use crate::{ClientId, ProjectileId, Tick, level_time_ms};

/// A live missile at `origin` moving at `velocity` on a straight path from tick zero, with no deadlines.
fn missile_at(origin: [f32; 3], velocity: [f32; 3]) -> ProjectileState {
    let pos = Trajectory { tr_type: LINEAR, tr_time: 0, tr_duration: 0, tr_base: origin, tr_delta: velocity };
    ProjectileState {
        id: ProjectileId(1),
        owner: ClientId(0),
        owner_life: LifeSequence(0),
        weapon: 1,
        entnum: 70,
        origin,
        velocity,
        pos,
        apos: Trajectory::default(),
        launch_time: 0,
        spawn_time_ms: 0,
        detonate_at_ms: None,
        cleanup_at_ms: i32::MAX,
        travel_distance: 0.0,
        live: true,
        stuck_pane: None,
        grounded: false,
        guide: crate::MissileGuide::default(),
        impact_target: None,
        rider: None,
        coasting: false,
    }
}

/// A floor at z = 0: where a segment going from `from` to `to` meets it, facing up.
fn flat_floor(from: [f32; 3], to: [f32; 3]) -> Option<([f32; 3], [f32; 3])> {
    if to[2] > 0.0 {
        return None;
    }
    let along = from[2] / (from[2] - to[2]);
    Some((core::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * along), [0.0, 0.0, 1.0]))
}

/// A frag (body radius 1.37) set rolling along +x at `speed`. Each tick it moves along its line and takes one floor hit;
/// returns (tick, distance rolled, speed) per tick until it rests.
fn roll_out(speed: f32) -> Vec<(u32, f32, f32)> {
    let mut frag = missile_at([0.0, 0.0, 1.37], [speed, 0.0, 0.0]);
    let mut log = vec![(0, 0.0, speed)];
    for k in 1..400 {
        let tick = Tick(k);
        let time = level_time_ms(tick);
        let before = frag.origin;
        frag.origin = frag.origin_at(time);
        let step = core::array::from_fn(|axis| frag.origin[axis] - before[axis]);
        frag.travel_distance += norm(step);
        let vel = frag.velocity_at(time);
        // Free flight clears the ground flag every tick; the roll has to set it again or a ground-only fuse would never
        // go off.
        frag.grounded = false;
        // A bump roll of 1.0 never bumps, so the decay reads cleanly.
        let rested = roll_grenade(tick, &mut frag, 1.37, None, vel, 1.0, &flat_floor);
        assert!(frag.grounded, "not grounded while rolling at tick {k}");
        if rested {
            log.push((k, frag.travel_distance, 0.0));
            return log;
        }
        log.push((k, frag.travel_distance, norm(frag.velocity)));
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
    let facts = EquipmentRuntimeFacts {
        projectile_speed: 1000,
        projectile_speed_up: 100,
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
    let launcher = EquipmentRuntimeFacts {
        weap_type: i32::from(weapon_t6::facts::weap_type::PROJECTILE),
        projectile_activate_dist: 300,
        ..Default::default()
    };
    let wall = [-1.0, 0.0, 0.0];
    let duds = |velocity: [f32; 3], flesh: bool| {
        unarmed_dud(&launcher, &missile_at([0.0; 3], velocity), Tick(1), 0.5, wall, flesh)
    };
    assert!(duds([1200.0, 0.0, 0.0], false), "head-on and fast");
    assert!(duds([1200.0, 1000.0, 0.0], false), "within 45 degrees of head-on");
    assert!(!duds([1200.0, 1300.0, 0.0], false), "past 45 degrees");
    assert!(!duds([450.0, 0.0, 0.0], false), "under 500 even head-on");
    assert!(duds([300.0, 900.0, 0.0], true), "flesh always crumples it");
}

#[test]
fn only_projectiles_crumple_and_sticky_ones_never_bounce() {
    let thrown = EquipmentRuntimeFacts { weap_type: 1, ..Default::default() };
    let round = missile_at([0.0; 3], [1200.0, 0.0, 0.0]);
    let wall = [-1.0, 0.0, 0.0];
    assert!(!unarmed_dud(&thrown, &round, Tick(1), 0.5, wall, true));
    for stickiness in [1, 5] {
        let sticky = EquipmentRuntimeFacts { stickiness, ..thrown };
        assert!(unarmed_dud(&sticky, &round, Tick(1), 0.5, wall, false), "stickiness {stickiness}");
    }
}

#[test]
fn the_weapon_file_decides_what_spins() {
    let thrown = GrenadeLaunchKind::Thrown { remaining_fuse_ms: None };
    let bo2 = EquipmentRuntimeFacts { projectile_parent_velocity: Some(0.0), ..Default::default() };
    // The tomahawk (rotate 1, rotateType 1) turns end over end at 1500 a second.
    let hatchet = EquipmentRuntimeFacts { projectile_rotates: true, projectile_rotate_type: 1, ..bo2 };
    assert_eq!(spin_of(&hatchet, thrown), Spin::Fixed { pitch: 1500.0, roll: 0.0 });
    // A frag (rotate 1, rotateType 0) tumbles at whole degrees a second.
    let frag = EquipmentRuntimeFacts { projectile_rotates: true, ..bo2 };
    assert_eq!(
        spin_of(&frag, thrown),
        Spin::Tumble { pitch: (340.0, 800.0), roll: (180.0, 540.0), whole: true }
    );
    // A claymore (rotate 0) never turns, and nothing a launcher fires does.
    assert_eq!(spin_of(&bo2, thrown), Spin::None);
    assert_eq!(spin_of(&hatchet, GrenadeLaunchKind::Launcher), Spin::None);
    // A throwing knife without the parent-velocity row keeps its own blade spin.
    let knife = EquipmentRuntimeFacts { weap_class: 9, ..Default::default() };
    assert_eq!(spin_of(&knife, thrown), Spin::Fixed { pitch: entity_t6::missile::BLADE_SPIN_PITCH, roll: 0.0 });
}

#[test]
fn a_bounced_item_lies_where_it_slowed_to_a_stop() {
    let floor = [0.0, 0.0, 1.0];
    let wall = [-1.0, 0.0, 0.0];
    // A frag (stickiness 0) and a tomahawk (5) alike: slow off a floor, or all but stopped anywhere, it rests.
    for stickiness in [0, 5] {
        let facts = EquipmentRuntimeFacts { stickiness, ..Default::default() };
        assert!(comes_to_rest(&facts, 15.0, floor), "stickiness {stickiness}");
        assert!(comes_to_rest(&facts, 0.5, wall), "stickiness {stickiness}");
        assert!(!comes_to_rest(&facts, 15.0, wall), "stickiness {stickiness}");
        assert!(!comes_to_rest(&facts, 120.0, floor), "stickiness {stickiness}");
    }
    // A rolling grenade rests in its roll, not off a bounce.
    let rolling = EquipmentRuntimeFacts { rolling_radius: Some(1.37), ..Default::default() };
    assert!(!comes_to_rest(&rolling, 15.0, floor));
}
