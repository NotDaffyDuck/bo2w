//! The frame an element is positioned in.
//!
//! Which frame depends on the run-mode bits: none (the world), the effect as
//! it was at the element's spawn, the effect as it is now, or a frame built
//! at the element's own spawn offset.

use crate::runtime::flags::{
    FX_ELEM_RUN_RELATIVE_TO_EFFECT, FX_ELEM_RUN_RELATIVE_TO_SPAWN, elem_run_mode,
};
use crate::runtime::origin::{
    FX_ELEM_SPAWN_OFFSET_MASK, FX_ELEM_SPAWN_OFFSET_SPHERE, apply_spawn_origin,
    elem_spawn_relative, offset_spawn_origin,
};
use crate::runtime::vec::{vec3_cross, vec3_length_sq, vec3_normalize, vector_vectors};

/// A forward this close to straight up cannot be crossed with up.
const NEAR_VERTICAL: f32 = 0.999;
/// Below this squared length an offset has no direction to face.
const NO_DIRECTION_SQ: f32 = 1e-12;

const WORLD_AXIS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A position and axes: the frame an element's local position is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxOrientation {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

/// An effect's frame at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxOrientFrame {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

impl FxOrientFrame {
    /// The world frame.
    pub const IDENTITY: Self = Self {
        origin: [0.0; 3],
        axis: WORLD_AXIS,
    };
}

/// What it takes to rebuild an element's spawn frame from its def.
#[derive(Clone, Copy, Debug)]
pub struct FxOrientSpawnParams {
    pub spawn_origin: [[f32; 2]; 3],
    pub spawn_offset_radius: [f32; 2],
    pub spawn_offset_height: [f32; 2],
    pub seed: u32,
}

impl From<FxOrientFrame> for FxOrientation {
    fn from(frame: FxOrientFrame) -> Self {
        Self {
            origin: frame.origin,
            axis: frame.axis,
        }
    }
}

/// The frame for an element with these flags. The offset modes need the
/// def's spawn parameters; without them the spawn-time frame stands in.
pub fn get_orientation(
    flags: i32,
    effect_now: &FxOrientFrame,
    effect_at_spawn: &FxOrientFrame,
    spawn: Option<FxOrientSpawnParams>,
) -> FxOrientation {
    let mode = elem_run_mode(flags);
    if mode == 0 {
        return FxOrientFrame::IDENTITY.into();
    }
    if mode == FX_ELEM_RUN_RELATIVE_TO_SPAWN {
        return (*effect_at_spawn).into();
    }
    if mode == FX_ELEM_RUN_RELATIVE_TO_EFFECT {
        return (*effect_now).into();
    }
    match spawn {
        Some(params) => spawn_offset_frame(flags, effect_at_spawn, params),
        None => (*effect_at_spawn).into(),
    }
}

/// The frame at the element's spawn point, facing along the offset it got.
fn spawn_offset_frame(
    flags: i32,
    effect_at_spawn: &FxOrientFrame,
    params: FxOrientSpawnParams,
) -> FxOrientation {
    let mut origin = apply_spawn_origin(
        effect_at_spawn.origin,
        effect_at_spawn.axis,
        params.spawn_origin,
        params.seed,
        elem_spawn_relative(flags),
    );
    let mut offset = [0.0; 3];
    offset_spawn_origin(
        &mut offset,
        effect_at_spawn.axis,
        flags,
        params.spawn_offset_radius,
        params.spawn_offset_height,
        params.seed,
    );
    for (point, shift) in origin.iter_mut().zip(offset) {
        *point += shift;
    }

    let forward = if vec3_length_sq(offset) <= NO_DIRECTION_SQ {
        WORLD_AXIS[0]
    } else {
        vec3_normalize(offset)
    };
    let axis = if flags & FX_ELEM_SPAWN_OFFSET_MASK == FX_ELEM_SPAWN_OFFSET_SPHERE {
        vector_vectors(forward)
    } else {
        let up_guess = if libm::fabsf(forward[2]) < NEAR_VERTICAL {
            WORLD_AXIS[2]
        } else {
            WORLD_AXIS[1]
        };
        let right = vec3_normalize(vec3_cross(up_guess, forward));
        [forward, right, vec3_cross(forward, right)]
    };
    FxOrientation { origin, axis }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAWNED: FxOrientFrame = FxOrientFrame {
        origin: [10.0, 10.0, 10.0],
        axis: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    };
    const NOW: FxOrientFrame = FxOrientFrame {
        origin: [20.0, 0.0, 0.0],
        axis: WORLD_AXIS,
    };
    const PARAMS: FxOrientSpawnParams = FxOrientSpawnParams {
        spawn_origin: [[1.0, 0.0], [2.0, 0.0], [3.0, 0.0]],
        spawn_offset_radius: [0.0; 2],
        spawn_offset_height: [0.0; 2],
        seed: 4,
    };

    #[test]
    fn no_run_mode_is_the_world_frame() {
        let got = get_orientation(0, &NOW, &SPAWNED, None);
        assert_eq!(got.origin, [0.0; 3]);
        assert_eq!(got.axis, WORLD_AXIS);
    }

    #[test]
    fn spawn_and_effect_modes_pick_their_frames() {
        let at_spawn = get_orientation(FX_ELEM_RUN_RELATIVE_TO_SPAWN, &NOW, &SPAWNED, None);
        assert_eq!(at_spawn.origin, SPAWNED.origin);
        let now = get_orientation(FX_ELEM_RUN_RELATIVE_TO_EFFECT, &NOW, &SPAWNED, None);
        assert_eq!(now.origin, NOW.origin);
    }

    #[test]
    fn an_offset_mode_without_parameters_falls_back_to_the_spawn_frame() {
        let got = get_orientation(0xc0, &NOW, &SPAWNED, None);
        assert_eq!(got.origin, SPAWNED.origin);
        assert_eq!(got.axis, SPAWNED.axis);
    }

    #[test]
    fn an_offset_frame_with_no_offset_faces_forward_along_x() {
        let spawned = FxOrientFrame {
            origin: [10.0, 10.0, 10.0],
            axis: WORLD_AXIS,
        };
        let got = get_orientation(0xc0, &NOW, &spawned, Some(PARAMS));
        assert_eq!(got.origin, [11.0, 12.0, 13.0]);
        assert_eq!(got.axis, WORLD_AXIS);
    }

    #[test]
    fn a_sphere_offset_frame_faces_along_the_offset() {
        let spawned = FxOrientFrame {
            origin: [0.0; 3],
            axis: WORLD_AXIS,
        };
        let params = FxOrientSpawnParams {
            spawn_origin: [[0.0; 2]; 3],
            spawn_offset_radius: [25.0, 0.0],
            ..PARAMS
        };
        let got = get_orientation(0xc0 | 0x10, &NOW, &spawned, Some(params));
        let dist = libm::sqrtf(vec3_length_sq(got.origin));
        assert!((dist - 25.0).abs() < 1e-3, "{dist}");
        for (face, at) in got.axis[0].iter().zip(got.origin) {
            assert!((face * 25.0 - at).abs() < 1e-3);
        }
    }
}
