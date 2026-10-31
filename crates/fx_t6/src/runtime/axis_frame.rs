//! The frame an element is positioned in.
//!
//! Which frame depends on the run-mode bits: none (the world), the effect as
//! it was at the element's spawn, the effect as it is now, or a frame built
//! at the element's own spawn offset.

use crate::runtime::mode_bits::FX_ELEM_RUN_RELATIVE_TO_EFFECT;
use crate::runtime::mode_bits::FX_ELEM_RUN_RELATIVE_TO_SPAWN;
use crate::runtime::mode_bits::elem_run_mode;
use crate::runtime::birth_spot::{
    elem_spawn_relative, offset_spawn_origin,
};
use crate::runtime::birth_spot::BIRTH_DISPLACEMENT_BITS;
use crate::runtime::birth_spot::FX_ELEM_SPAWN_OFFSET_SPHERE;
use crate::runtime::birth_spot::apply_spawn_origin;
use crate::runtime::vec::{vec3_cross, vec3_length_sq, vec3_normalize, contact_facing_basis};

/// A forward this close to straight up cannot be crossed with up.
const NEAR_VERTICAL: f32 = 0.999;
/// Below this squared length an offset has no direction to face.
const NO_DIRECTION_SQ: f32 = 1e-12;

const WORLD_AXIS: [[f32; 3]; 3] = [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A position and axes: the frame an element's local position is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectFacing {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

/// An effect's frame at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectFacingBasis {
    pub origin: [f32; 3],
    pub axis: [[f32; 3]; 3],
}

impl EffectFacingBasis {
    /// The world frame.
    pub const IDENTITY: Self = Self {
        origin: [0.0_f32; 3],
        axis: WORLD_AXIS,
    };
}

/// What it takes to rebuild an element's spawn frame from its def.
#[derive(Clone, Copy, Debug)]
pub struct FacingBirthKnobs {
    pub birth_point: [[f32; 2]; 3],
    pub birth_ring_reach: [f32; 2],
    pub birth_lift: [f32; 2],
    pub seed: u32,
}

impl From<EffectFacingBasis> for EffectFacing {
    fn from(frame: EffectFacingBasis) -> Self {
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
    effect_now: &EffectFacingBasis,
    effect_at_spawn: &EffectFacingBasis,
    spawn: Option<FacingBirthKnobs>,
) -> EffectFacing {
    match (elem_run_mode(flags), spawn) {
        (0, _) => EffectFacingBasis::IDENTITY.into(),
        (FX_ELEM_RUN_RELATIVE_TO_SPAWN, _) => (*effect_at_spawn).into(),
        (FX_ELEM_RUN_RELATIVE_TO_EFFECT, _) => (*effect_now).into(),
        (_, Some(params)) => spawn_offset_frame(flags, effect_at_spawn, params),
        (_, None) => (*effect_at_spawn).into(),
    }
}

/// The frame at the element's spawn point, facing along the offset it got.
fn spawn_offset_frame(
    flags: i32,
    effect_at_spawn: &EffectFacingBasis,
    params: FacingBirthKnobs,
) -> EffectFacing {
    let mut origin = apply_spawn_origin(
        effect_at_spawn.origin,
        effect_at_spawn.axis,
        params.birth_point,
        params.seed,
        elem_spawn_relative(flags),
    );
    let mut offset = [0.0; 3];
    offset_spawn_origin(
        &mut offset,
        effect_at_spawn.axis,
        flags,
        params.birth_ring_reach,
        params.birth_lift,
        params.seed,
    );
    for (point, shift) in origin.iter_mut().zip(offset) {
        *point += shift;
    }

    let forward = match vec3_length_sq(offset) {
        sq if sq <= NO_DIRECTION_SQ => WORLD_AXIS[0],
        _ => vec3_normalize(offset),
    };
    let axis = match flags & BIRTH_DISPLACEMENT_BITS {
        FX_ELEM_SPAWN_OFFSET_SPHERE => contact_facing_basis(forward),
        _ => {
            let steep = libm::fabsf(forward[2]) >= NEAR_VERTICAL;
            let up_guess = WORLD_AXIS[if steep { 1 } else { 2 }];
            let right = vec3_normalize(vec3_cross(up_guess, forward));
            [forward, right, vec3_cross(forward, right)]
        }
    };
    EffectFacing { axis, origin }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAWNED: EffectFacingBasis = EffectFacingBasis {
        origin: [10.0, 10.0, 10.0],
        axis: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    };
    const NOW: EffectFacingBasis = EffectFacingBasis {
        origin: [20.0, 0.0, 0.0],
        axis: WORLD_AXIS,
    };
    const PARAMS: FacingBirthKnobs = FacingBirthKnobs {
        birth_point: [[1.0, 0.0], [2.0, 0.0], [3.0, 0.0]],
        birth_ring_reach: [0.0; 2],
        birth_lift: [0.0; 2],
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
        let spawned = EffectFacingBasis {
            origin: [10.0, 10.0, 10.0],
            axis: WORLD_AXIS,
        };
        let got = get_orientation(0xc0, &NOW, &spawned, Some(PARAMS));
        assert_eq!(got.origin, [11.0, 12.0, 13.0]);
        assert_eq!(got.axis, WORLD_AXIS);
    }

    #[test]
    fn a_sphere_offset_frame_faces_along_the_offset() {
        let spawned = EffectFacingBasis {
            origin: Default::default(),
            axis: WORLD_AXIS,
        };
        let params = FacingBirthKnobs {
            birth_point: [[0.0; 2]; 3],
            birth_ring_reach: [25.0, 0.0],
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
