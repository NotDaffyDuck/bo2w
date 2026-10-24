//! Where a new element appears.
//!
//! The element def gives a box (`base + amplitude * random` per axis) for the
//! spawn point, and may add a random offset on a sphere or a cylinder around
//! the effect.

use core::f32::consts::TAU;

use crate::runtime::random::{
    FX_RAND_CH_SPAWN_OFFSET_HEIGHT, FX_RAND_CH_SPAWN_OFFSET_RADIUS, FX_RAND_CH_SPAWN_OFFSET_YAW,
    FX_RAND_CH_SPAWN_ORIGIN_X, FX_RAND_CH_SPAWN_ORIGIN_Y, FX_RAND_CH_SPAWN_ORIGIN_Z,
    random_table_f32,
};

/// The spawn box is measured along the effect's axes, not the world's.
const SPAWN_RELATIVE: i32 = 0x2;
/// The two flag bits that choose the offset shape.
pub(super) const FX_ELEM_SPAWN_OFFSET_MASK: i32 = 0x30;
/// Offset on a sphere around the spawn point.
pub(super) const FX_ELEM_SPAWN_OFFSET_SPHERE: i32 = 0x10;
/// Offset on a cylinder around the effect's forward axis.
pub(super) const FX_ELEM_SPAWN_OFFSET_CYLINDER: i32 = 0x20;

/// The shape of the random offset around the spawn point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxSpawnOffsetMode {
    None,
    Sphere,
    Cylinder,
}

/// The offset shape the flags ask for. Both shape bits at once is not a
/// shape, so it gives `None`.
pub fn elem_spawn_offset_mode(flags: i32) -> Option<FxSpawnOffsetMode> {
    match flags & FX_ELEM_SPAWN_OFFSET_MASK {
        0 => Some(FxSpawnOffsetMode::None),
        FX_ELEM_SPAWN_OFFSET_SPHERE => Some(FxSpawnOffsetMode::Sphere),
        FX_ELEM_SPAWN_OFFSET_CYLINDER => Some(FxSpawnOffsetMode::Cylinder),
        _ => None,
    }
}

/// Whether the spawn box follows the effect's axes.
pub(super) fn elem_spawn_relative(flags: i32) -> bool {
    flags & SPAWN_RELATIVE != 0
}

/// `base + amplitude * unit`, the way every authored range is sampled.
pub(super) fn sample_float_range(base: f32, amplitude: f32, unit: f32) -> f32 {
    base + amplitude * unit
}

/// The point picked inside the spawn box, in the box's own space.
fn sample_spawn_box(spawn_origin: [[f32; 2]; 3], seed: u32) -> [f32; 3] {
    let channels = [
        FX_RAND_CH_SPAWN_ORIGIN_X,
        FX_RAND_CH_SPAWN_ORIGIN_Y,
        FX_RAND_CH_SPAWN_ORIGIN_Z,
    ];
    let mut local = [0.0; 3];
    for (axis, channel) in channels.into_iter().enumerate() {
        local[axis] = sample_float_range(
            spawn_origin[axis][0],
            spawn_origin[axis][1],
            random_table_f32(seed, channel),
        );
    }
    local
}

/// A random direction. The height comes from two draws of the same channel
/// added together, which is why it is not uniform, and the yaw from another.
fn random_direction(seed: u32) -> [f32; 3] {
    let draw = random_table_f32(seed, FX_RAND_CH_SPAWN_OFFSET_HEIGHT);
    let height = draw + draw - 1.0;
    let horizontal = libm::sqrtf((1.0 - height * height).max(0.0));
    let yaw = random_table_f32(seed, FX_RAND_CH_SPAWN_OFFSET_YAW) * TAU;
    [
        horizontal * libm::cosf(yaw),
        horizontal * libm::sinf(yaw),
        height,
    ]
}

/// The spawn point from the effect's origin and the spawn box: moved along
/// the effect's axes when the box is relative, along the world's otherwise.
pub(super) fn apply_spawn_origin(
    effect_origin: [f32; 3],
    axis: [[f32; 3]; 3],
    spawn_origin: [[f32; 2]; 3],
    seed: u32,
    spawn_relative: bool,
) -> [f32; 3] {
    let local = sample_spawn_box(spawn_origin, seed);
    if !spawn_relative {
        return [
            effect_origin[0] + local[0],
            effect_origin[1] + local[1],
            effect_origin[2] + local[2],
        ];
    }
    let mut world = effect_origin;
    for (i, slot) in world.iter_mut().enumerate() {
        *slot = effect_origin[i]
            + local[0] * axis[0][i]
            + local[1] * axis[1][i]
            + local[2] * axis[2][i];
    }
    world
}

/// Adds the random sphere or cylinder offset to `origin`. Flags that name no
/// shape change nothing.
pub(super) fn offset_spawn_origin(
    origin: &mut [f32; 3],
    axis: [[f32; 3]; 3],
    flags: i32,
    radius: [f32; 2],
    height: [f32; 2],
    seed: u32,
) {
    let reach = sample_float_range(
        radius[0],
        radius[1],
        random_table_f32(seed, FX_RAND_CH_SPAWN_OFFSET_RADIUS),
    );
    match elem_spawn_offset_mode(flags) {
        None | Some(FxSpawnOffsetMode::None) => {}
        Some(FxSpawnOffsetMode::Sphere) => {
            let dir = random_direction(seed);
            for (slot, d) in origin.iter_mut().zip(dir) {
                *slot += reach * d;
            }
        }
        Some(FxSpawnOffsetMode::Cylinder) => {
            let yaw = random_table_f32(seed, FX_RAND_CH_SPAWN_OFFSET_YAW) * TAU;
            let across = reach * libm::cosf(yaw);
            let up = reach * libm::sinf(yaw);
            for (i, slot) in origin.iter_mut().enumerate() {
                *slot += across * axis[1][i] + up * axis[2][i];
            }
            let lift = sample_float_range(
                height[0],
                height[1],
                random_table_f32(seed, FX_RAND_CH_SPAWN_OFFSET_HEIGHT),
            );
            for (i, slot) in origin.iter_mut().enumerate() {
                *slot += lift * axis[0][i];
            }
        }
    }
}

/// Where in the world an element spawns: the spawn box, then the offset.
#[allow(clippy::too_many_arguments)]
pub fn spawn_origin_world(
    effect_origin: [f32; 3],
    axis: [[f32; 3]; 3],
    spawn_origin: [[f32; 2]; 3],
    flags: i32,
    radius_base: f32,
    radius_amp: f32,
    height_base: f32,
    height_amp: f32,
    seed: u32,
) -> [f32; 3] {
    let mut origin = apply_spawn_origin(
        effect_origin,
        axis,
        spawn_origin,
        seed,
        elem_spawn_relative(flags),
    );
    offset_spawn_origin(
        &mut origin,
        axis,
        flags,
        [radius_base, radius_amp],
        [height_base, height_amp],
        seed,
    );
    origin
}

/// A world point as seen from an origin and axes (the inverse of the
/// orientation transform).
pub fn world_delta_to_local(
    world: [f32; 3],
    effect_origin: [f32; 3],
    axis: [[f32; 3]; 3],
) -> [f32; 3] {
    let d = [
        world[0] - effect_origin[0],
        world[1] - effect_origin[1],
        world[2] - effect_origin[2],
    ];
    [
        d[0] * axis[0][0] + d[1] * axis[0][1] + d[2] * axis[0][2],
        d[0] * axis[1][0] + d[1] * axis[1][1] + d[2] * axis[1][2],
        d[0] * axis[2][0] + d[1] * axis[2][1] + d[2] * axis[2][2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const TURNED: [[f32; 3]; 3] = [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    const BOX: [[f32; 2]; 3] = [[1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];

    #[test]
    fn offset_shapes_come_from_the_two_flag_bits() {
        assert_eq!(elem_spawn_offset_mode(0), Some(FxSpawnOffsetMode::None));
        assert_eq!(
            elem_spawn_offset_mode(0x10),
            Some(FxSpawnOffsetMode::Sphere)
        );
        assert_eq!(
            elem_spawn_offset_mode(0x20),
            Some(FxSpawnOffsetMode::Cylinder)
        );
        assert_eq!(elem_spawn_offset_mode(0x30), None);
        assert_eq!(elem_spawn_offset_mode(0x2), Some(FxSpawnOffsetMode::None));
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
