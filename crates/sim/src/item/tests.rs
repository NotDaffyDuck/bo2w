use playerstate_t6::game::{PERK_SCAVENGER, PlayerState};

use super::inventory::{Rounds, take_weapon};
use super::{aabb_overlap, has_scavenger_perk};

const UNIT_MINS: [f32; 3] = [-1.0; 3];
const UNIT_MAXS: [f32; 3] = [1.0; 3];

fn overlap(a: [f32; 3], b: [f32; 3]) -> bool {
    aabb_overlap(a, UNIT_MINS, UNIT_MAXS, b, UNIT_MINS, UNIT_MAXS)
}

#[test]
fn boxes_that_share_space_or_a_face_overlap() {
    assert!(overlap([0.0; 3], [0.0; 3]));
    assert!(overlap([0.0; 3], [1.5, -1.5, 0.5]));
    // Exactly touching faces count, on each axis.
    assert!(overlap([0.0; 3], [2.0, 0.0, 0.0]));
    assert!(overlap([0.0; 3], [0.0, -2.0, 0.0]));
    assert!(overlap([0.0; 3], [0.0, 0.0, 2.0]));
}

#[test]
fn boxes_with_a_gap_on_any_one_axis_do_not_overlap() {
    for axis in 0..3 {
        for gap in [2.01, -2.01, 50.0] {
            let mut away = [0.0; 3];
            away[axis] = gap;
            assert!(!overlap([0.0; 3], away), "axis {axis} gap {gap}");
            assert!(!overlap(away, [0.0; 3]), "axis {axis} gap {gap}");
        }
    }
}

#[test]
fn different_sized_boxes_are_compared_from_their_own_corners() {
    let tall = ([0.0; 3], [-15.0, -15.0, 0.0], [15.0, 15.0, 70.0]);
    let item_at = |z: f32| aabb_overlap(tall.0, tall.1, tall.2, [10.0, 0.0, z], UNIT_MINS, UNIT_MAXS);
    assert!(item_at(35.0));
    assert!(item_at(71.0));
    assert!(!item_at(71.5));
    assert!(item_at(-1.0));
    assert!(!item_at(-1.5));
}

#[test]
fn only_the_scavenger_perk_bit_counts() {
    let mut ps = PlayerState::ZERO;
    assert!(!has_scavenger_perk(&ps));
    ps.perks[0] = PERK_SCAVENGER;
    assert!(has_scavenger_perk(&ps));
    ps.perks[0] = !PERK_SCAVENGER;
    assert!(!has_scavenger_perk(&ps));
}

#[test]
fn rounds_total_adds_both_magazines_and_the_reserve() {
    assert_eq!(Rounds::default().total(), 0);
    assert_eq!(Rounds { right: 7, left: 5, reserve: 90 }.total(), 102);
}

#[test]
fn taking_a_weapon_away_empties_its_slots_and_the_hands_only_if_it_was_out() {
    let mut ps = PlayerState::ZERO;
    ps.weapons[0] = 11;
    ps.weapons[1] = 22;
    ps.weapons[4] = 11;
    ps.weapon = 22;
    ps.weapon_primary = 22;

    take_weapon(&mut ps, 11);
    assert_eq!(&ps.weapons[..5], &[0, 22, 0, 0, 0]);
    assert_eq!((ps.weapon, ps.weapon_primary), (22, 22), "the weapon in hand was another one");

    take_weapon(&mut ps, 22);
    assert!(ps.weapons.iter().all(|&slot| slot == 0));
    assert_eq!((ps.weapon, ps.weapon_primary), (0, 0));
}

#[test]
fn an_alt_mode_in_hand_is_cleared_with_its_primary() {
    let mut ps = PlayerState::ZERO;
    ps.weapons[0] = 33;
    ps.weapon = 34;
    ps.weapon_primary = 33;
    take_weapon(&mut ps, 33);
    assert_eq!((ps.weapon, ps.weapon_primary), (0, 0));
}
