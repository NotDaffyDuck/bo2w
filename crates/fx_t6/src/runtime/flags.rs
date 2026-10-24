//! The element flag bits, typed as the `i32` the runtime's element defs carry.
//!
//! The bit values themselves live in [`crate::flags`]; this module re-types
//! the ones the runtime reads and adds the usual tests on them.

use crate::flags as bits;

/// The element collides with the world.
pub const FX_ELEM_USE_COLLISION: i32 = bits::FX_ELEM_USE_COLLISION as i32;
/// The element dies the first time it touches something.
pub const FX_ELEM_DIE_ON_TOUCH: i32 = bits::FX_ELEM_DIE_ON_TOUCH as i32;
/// The element is skipped at spawn when it is outside the view.
pub const FX_ELEM_SPAWN_FRUSTUM_CULL: i32 = bits::FX_ELEM_SPAWN_FRUSTUM_CULL as i32;
/// A runner element starts at a random roll.
pub const FX_ELEM_RUNNER_USES_RAND_ROT: i32 = bits::FX_ELEM_RUNNER_USES_RAND_ROT as i32;
/// The velocity graph is in the element's own axes.
pub const FX_ELEM_VEL_LOCAL: i32 = bits::FX_ELEM_VELOCITY_LOCAL as i32;
/// The velocity graph is in world axes.
pub const FX_ELEM_VEL_WORLD: i32 = bits::FX_ELEM_VELOCITY_WORLD as i32;

/// The bits that put an element on the velocity-graph update path.
const UPDATE_HAS_VEL_GRAPH: i32 = 0x0600_0000;
const USE_MODEL_PHYSICS: i32 = bits::FX_ELEM_MODEL_PHYSICS as i32;

/// The three bits that say which frame the element runs in.
pub const FX_ELEM_RUN_MASK: i32 = bits::FX_ELEM_RUN_MASK as i32;
/// Runs in the frame the effect had when the element spawned.
pub const FX_ELEM_RUN_RELATIVE_TO_SPAWN: i32 = bits::FX_ELEM_RUN_RELATIVE_TO_SPAWN as i32;
/// Runs in the effect's current frame.
pub const FX_ELEM_RUN_RELATIVE_TO_EFFECT: i32 = bits::FX_ELEM_RUN_RELATIVE_TO_EFFECT as i32;
/// Runs in a frame built at the spawn offset.
pub const FX_ELEM_RUN_RELATIVE_TO_OFFSET: i32 = bits::FX_ELEM_RUN_RELATIVE_TO_OFFSET as i32;
/// The same, with the effect-now bit set. Nothing says how it differs from
/// the plain offset mode, so it is run the same way.
pub const FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW: i32 =
    bits::FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW as i32;

/// The run-mode bits of `flags`.
pub const fn elem_run_mode(flags: i32) -> i32 {
    flags & FX_ELEM_RUN_MASK
}

/// Whether the element runs in an offset frame, with or without the
/// effect-now bit.
pub const fn elem_runs_relative_to_offset(flags: i32) -> bool {
    let mode = elem_run_mode(flags);
    mode == FX_ELEM_RUN_RELATIVE_TO_OFFSET || mode == FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW
}

/// Whether the element collides with the world.
pub const fn elem_uses_collision(flags: i32) -> bool {
    flags & FX_ELEM_USE_COLLISION != 0
}

/// Whether the element dies on first touch.
pub const fn elem_dies_on_touch(flags: i32) -> bool {
    flags & FX_ELEM_DIE_ON_TOUCH != 0
}

/// Whether the element is frustum-culled when it spawns.
pub const fn elem_spawn_frustum_cull(flags: i32) -> bool {
    flags & FX_ELEM_SPAWN_FRUSTUM_CULL != 0
}

/// Whether velocity is in the element's own axes.
pub const fn elem_uses_vel_local(flags: i32) -> bool {
    flags & FX_ELEM_VEL_LOCAL != 0
}

/// Whether velocity is in world axes.
pub const fn elem_uses_vel_world(flags: i32) -> bool {
    flags & FX_ELEM_VEL_WORLD != 0
}

/// Whether the element has a velocity graph to integrate.
pub const fn elem_update_has_velocity_graph(flags: i32) -> bool {
    flags & UPDATE_HAS_VEL_GRAPH != 0
}

/// Whether the update must leave the position alone: a model element driven
/// by physics (element type 7).
pub const fn elem_skips_position_update(elem_type: u8, flags: i32) -> bool {
    elem_type == 7 && flags & USE_MODEL_PHYSICS != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_and_touch_bits_are_the_black_ops_ones() {
        assert!(elem_uses_collision(0x200));
        assert!(!elem_uses_collision(0x100));
        assert!(elem_dies_on_touch(0x400));
        assert!(!elem_dies_on_touch(0x200));
    }

    #[test]
    fn the_run_mode_is_the_masked_bits() {
        assert_eq!(
            elem_run_mode(0x100 | 0x200),
            FX_ELEM_RUN_RELATIVE_TO_OFFSET_EFFECT_NOW
        );
        assert_eq!(elem_run_mode(0x40 | 0x400), FX_ELEM_RUN_RELATIVE_TO_SPAWN);
        assert_eq!(elem_run_mode(0x80), FX_ELEM_RUN_RELATIVE_TO_EFFECT);
    }

    #[test]
    fn both_offset_modes_run_relative_to_the_offset() {
        assert!(elem_runs_relative_to_offset(0xc0));
        assert!(elem_runs_relative_to_offset(0x100));
        assert!(!elem_runs_relative_to_offset(0x80));
        assert!(!elem_runs_relative_to_offset(0x40));
    }

    #[test]
    fn velocity_frame_bits_are_separate() {
        assert!(elem_uses_vel_local(0x100_0000));
        assert!(!elem_uses_vel_local(0x200_0000));
        assert!(elem_uses_vel_world(0x200_0000));
        assert!(elem_update_has_velocity_graph(0x400_0000));
        assert!(!elem_update_has_velocity_graph(0x100_0000));
    }

    #[test]
    fn only_physics_models_skip_the_position_update() {
        assert!(elem_skips_position_update(7, 0x800_0000));
        assert!(!elem_skips_position_update(7, 0));
        assert!(!elem_skips_position_update(3, 0x800_0000));
    }

    #[test]
    fn spawn_frustum_cull_is_bit_two() {
        assert!(elem_spawn_frustum_cull(0x4));
        assert!(!elem_spawn_frustum_cull(0x2));
    }
}
