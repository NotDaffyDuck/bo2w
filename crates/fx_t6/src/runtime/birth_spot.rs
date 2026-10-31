//! Where a new element appears.
//!
//! The element def gives a box (`base + amplitude * random` per axis) for the
//! spawn point, and may add a random offset on a sphere or a cylinder around
//! the effect.

use core::f32::consts::TAU;

use crate::runtime::dice::{
    random_table_f32,

};
use crate::runtime::dice::DICE_LANE_BIRTH_LIFT;
use crate::runtime::dice::DICE_LANE_RING_TURN;
use crate::runtime::dice::DICE_LANE_BIRTH_ACROSS;
use super::dice::DICE_LANE_RING_REACH;
use super::dice::DICE_LANE_BIRTH_SIDE;
use super::dice::DICE_LANE_BIRTH_HEIGHT;

/// The spawn box is measured along the effect's axes, not the world's.
const SPAWN_RELATIVE: i32 = 0x2;
/// The two flag bits that choose the offset shape.
pub(super) const BIRTH_DISPLACEMENT_BITS: i32 = 0x30;
/// Offset on a sphere around the spawn point.
pub(super) const FX_ELEM_SPAWN_OFFSET_SPHERE: i32 = 0x10;
/// Offset on a cylinder around the effect's forward axis.
pub(super) const FX_ELEM_SPAWN_OFFSET_CYLINDER: i32 = 0x20;

/// The shape of the random offset around the spawn point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BirthDisplacementStyle {
    None,
    Sphere,
    Cylinder,
}

/// The offset shape the flags ask for. Both shape bits at once is not a
/// shape, so it gives `None`.
pub fn elem_spawn_offset_mode(flags: i32) -> Option<BirthDisplacementStyle> {
    const SHAPES: [(i32, BirthDisplacementStyle); 3] = [
        (0, BirthDisplacementStyle::None),
        (FX_ELEM_SPAWN_OFFSET_SPHERE, BirthDisplacementStyle::Sphere),
        (FX_ELEM_SPAWN_OFFSET_CYLINDER, BirthDisplacementStyle::Cylinder),
    ];
    let bits = flags & BIRTH_DISPLACEMENT_BITS;
    SHAPES.iter().find(|(shape_bits, _)| *shape_bits == bits).map(|&(_, mode)| mode)
}

/// Whether the spawn box follows the effect's axes.
pub(super) fn elem_spawn_relative(flags: i32) -> bool {
    flags & SPAWN_RELATIVE != 0
}

/// `base + amplitude * unit`, the way every authored range is sampled.
pub(super) fn sample_float_range(base: f32, spread: f32, unit: f32) -> f32 {
    base + spread * unit
}

/// The point picked inside the spawn box, in the box's own space.
fn sample_spawn_box(birth_point: [[f32; 2]; 3], salt: u32) -> [f32; 3] {
    const CHANNELS: [u32; 3] = [
        DICE_LANE_BIRTH_ACROSS,
        DICE_LANE_BIRTH_SIDE,
        DICE_LANE_BIRTH_HEIGHT,
    ];
    core::array::from_fn::<f32, 3, _>(|axis| {
        let [base, amp] = birth_point[axis];
        sample_float_range(base, amp, random_table_f32(salt, CHANNELS[axis]))
    })
}

/// A random direction. The height comes from two draws of the same channel
/// added together, which is why it is not uniform, and the yaw from another.
fn random_direction(seed: u32) -> [f32; 3] {
    let draw = random_table_f32(seed, DICE_LANE_BIRTH_LIFT);
    let height = draw + draw - 1.0;
    let horizontal = libm::sqrtf((1.0 - height * height).max(0.0));
    let yaw = random_table_f32(seed, DICE_LANE_RING_TURN) * TAU;
    [
        horizontal * libm::cosf(yaw),
        horizontal * libm::sinf(yaw),
        height,
    ]
}

/// The spawn point from the effect's origin and the spawn box: moved along
/// the effect's axes when the box is relative, along the world's otherwise.
pub(super) fn apply_spawn_origin(
    fx_spot: [f32; 3],
    turn: [[f32; 3]; 3],
    birth_point: [[f32; 2]; 3],
    seed: u32,
    rel_spawn: bool,
) -> [f32; 3] {
    let local = sample_spawn_box(birth_point, seed);
    let shift = |k: usize| match rel_spawn {
        true => local[0] * turn[0][k] + local[1] * turn[1][k] + local[2] * turn[2][k],
        false => local[k],
    };
    core::array::from_fn(|k| fx_spot[k] + shift(k))
}

/// Adds the random sphere or cylinder offset to `spot`. Flags that name no
/// shape change nothing.
pub(super) fn offset_spawn_origin(
    spot: &mut [f32; 3],
    turn: [[f32; 3]; 3],
    flags: i32,
    radius: [f32; 2],
    height: [f32; 2],
    seed: u32,
) {
    let reach = sample_float_range(
        radius[0],
        radius[1],
        random_table_f32(seed, DICE_LANE_RING_REACH),
    );
    match elem_spawn_offset_mode(flags) {
        None | Some(BirthDisplacementStyle::None) => {}
        Some(BirthDisplacementStyle::Sphere) => {
            let dir = random_direction(seed);
            spot.iter_mut().zip(dir).for_each(|(slot, d)| *slot += reach * d);
        }
        Some(BirthDisplacementStyle::Cylinder) => {
            let yaw = random_table_f32(seed, DICE_LANE_RING_TURN) * TAU;
            let across = reach * libm::cosf(yaw);
            let up = reach * libm::sinf(yaw);
            let [lift_floor, lift_spread] = height;
            let lift = sample_float_range(
                lift_floor,
                lift_spread,
                random_table_f32(seed, DICE_LANE_BIRTH_LIFT),
            );
            let sideways: [f32; 3] =
                core::array::from_fn(|k| across * turn[1][k] + up * turn[2][k]);
            for (k, slot) in spot.iter_mut().enumerate() {
                *slot = *slot + sideways[k] + lift * turn[0][k];
            }
        }
    }
}

/// Where in the world an element spawns: the spawn box, then the offset.
#[allow(clippy::too_many_arguments)]
pub fn spawn_origin_world(
    pin: [f32; 3],
    turn: [[f32; 3]; 3],
    birth_point: [[f32; 2]; 3],
    flags: i32,
    reach_min: f32,
    reach_span: f32,
    lift_floor: f32,
    lift_spread: f32,
    seed: u32,
) -> [f32; 3] {
    let mut spot = apply_spawn_origin(
        pin,
        turn,
        birth_point,
        seed,
        elem_spawn_relative(flags),
    );
    offset_spawn_origin(
        &mut spot,
        turn,
        flags,
        [reach_min, reach_span],
        [lift_floor, lift_spread],
        seed,
    );
    spot
}

/// A world point as seen from an origin and axes (the inverse of the
/// orientation transform).
pub fn world_delta_to_local(
    world: [f32; 3],
    fx_spot: [f32; 3],
    turn: [[f32; 3]; 3],
) -> [f32; 3] {
    let d: [f32; 3] = core::array::from_fn(|k| world[k] - fx_spot[k]);
    turn.map(|row| row[0] * d[0] + row[1] * d[1] + row[2] * d[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const TURNED: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    const BOX: [[f32; 2]; 3] = [[1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];

    #[test]
    fn offset_shapes_come_from_the_two_flag_bits() {
        assert_eq!(elem_spawn_offset_mode(0), Some(BirthDisplacementStyle::None));
        assert_eq!(
            elem_spawn_offset_mode(0x10),
            Some(BirthDisplacementStyle::Sphere)
        );
        assert_eq!(
            elem_spawn_offset_mode(0x20),
            Some(BirthDisplacementStyle::Cylinder)
        );
        assert_eq!(elem_spawn_offset_mode(0x30), None);
        assert_eq!(elem_spawn_offset_mode(0x2), Some(BirthDisplacementStyle::None));
    }

    #[test]
    fn a_fixed_box_in_world_space_just_adds() {
        let at = apply_spawn_origin([10.0, 10.0, 10.0], TURNED, BOX, 5, false);
        assert_eq!(at, [11.0, 12.0, 13.0]);
    }

    #[test]
    fn a_relative_box_turns_with_the_effect() {
        let at = apply_spawn_origin([0.0; 3], TURNED, BOX, 5, true);
        assert_eq!(at, [-2.0, 1.0, 3.0]);
    }

    #[test]
    fn no_shape_means_no_offset() {
        let mut origin = [4.0, 5.0, 6.0];
        offset_spawn_origin(&mut origin, IDENTITY, 0, [9.0, 9.0], [9.0, 9.0], 3);
        offset_spawn_origin(&mut origin, IDENTITY, 0x30, [9.0, 9.0], [9.0, 9.0], 3);
        assert_eq!(origin, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn a_sphere_offset_lands_at_the_sampled_radius() {
        let mut origin = [0.0; 3];
        offset_spawn_origin(&mut origin, IDENTITY, 0x10, [50.0, 0.0], [0.0; 2], 17);
        let dist = libm::sqrtf(origin.iter().map(|c| c * c).sum());
        assert!((dist - 50.0).abs() < 1e-3, "{dist}");
    }

    #[test]
    fn a_cylinder_offset_sits_in_the_plane_across_forward_plus_the_lift() {
        let mut origin = [0.0; 3];
        offset_spawn_origin(&mut origin, IDENTITY, 0x20, [30.0, 0.0], [7.0, 0.0], 40);
        assert_eq!(origin[0], 7.0);
        let across = libm::sqrtf(origin[1] * origin[1] + origin[2] * origin[2]);
        assert!((across - 30.0).abs() < 1e-3, "{across}");
    }

    #[test]
    fn world_to_local_undoes_the_axes() {
        let local = world_delta_to_local([1.0, 1.0, 5.0], [1.0, 0.0, 2.0], TURNED);
        assert_eq!(local, [1.0, 0.0, 3.0]);
    }
}
