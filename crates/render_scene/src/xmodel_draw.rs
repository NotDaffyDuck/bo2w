//! One surface of a model queued for drawing.

use bevy::prelude::*;

use crate::ModelLightingRequest;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XModelSurfaceDraw {
    pub surface: u32,
    pub material: u32,
    pub world_from_local: Mat4,
    pub lighting_handle: u32,
    /// Lighting asked for this frame and not yet seated.
    pub pending_lighting: Option<ModelLightingRequest>,
    pub colour_refusal: Option<XModelColourRefusal>,
    pub object_id: u16,
    pub scene_light_index: u8,
    pub reflection_probe_index: u8,
    pub packed_lighting: Option<[u8; 4]>,
    pub is_scope: bool,
    pub scene_entnum: Option<u32>,
    /// World-space bounding sphere of the object this surface belongs to.
    /// `None` is "unbounded": the surface is admitted to every shadow
    /// partition, which is what a producer that cannot state a bound gets.
    pub caster_bound: Option<XModelCasterBound>,
}

/// A shadow caster's world-space bounding sphere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XModelCasterBound {
    pub origin: [f32; 3],
    pub radius: f32,
}

impl XModelCasterBound {
    /// `true` when the whole sphere is behind at least one plane, so no part of
    /// it is inside the volume the planes bound. Planes face inwards and carry
    /// their distance in `[3]`, as `dpvs_t6::bounds_culled` reads them.
    #[must_use]
    pub fn outside(self, planes: &[[f32; 4]]) -> bool {
        let [x, y, z] = self.origin;
        planes
            .iter()
            .any(|p| p[0] * x + p[1] * y + p[2] * z + p[3] + self.radius <= 0.0)
    }
}

/// Why a surface was drawn without its own colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XModelColourRefusal {
    CameraFrustum,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOOR: [f32; 4] = [0.0, 0.0, 1.0, 0.0];

    fn sphere(z: f32, radius: f32) -> XModelCasterBound {
        XModelCasterBound { origin: [0.0, 0.0, z], radius }
    }

    #[test]
    fn a_sphere_wholly_behind_a_plane_is_outside() {
        assert!(sphere(-5.0, 2.0).outside(&[FLOOR]));
    }

    #[test]
    fn a_sphere_crossing_the_plane_is_not_outside() {
        assert!(!sphere(-1.0, 2.0).outside(&[FLOOR]));
        assert!(!sphere(3.0, 2.0).outside(&[FLOOR]));
    }

    #[test]
    fn a_sphere_only_touching_the_plane_is_outside() {
        assert!(sphere(-2.0, 2.0).outside(&[FLOOR]));
    }

    #[test]
    fn being_behind_any_one_plane_is_enough() {
        let wall = [1.0, 0.0, 0.0, -10.0];
        assert!(sphere(5.0, 1.0).outside(&[FLOOR, wall]));
        assert!(!sphere(5.0, 1.0).outside(&[]));
    }
}
