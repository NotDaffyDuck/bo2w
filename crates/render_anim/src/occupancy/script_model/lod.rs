//! Which level of detail each model of a DObj is drawn at, for a viewer.

use asset_model::ModelSkel;
use bevy::prelude::*;
use render_scene::{LodRampArgs, smodel_camera_lod};

/// Where a DObj stands in relation to the camera.
#[derive(Clone, Copy, Debug)]
pub struct DObjLodView {
    pub origin: [f32; 3],
    pub eye: Option<Vec3>,
    pub ramp: LodRampArgs,
}

/// The LOD `skel` is drawn at, `None` if it is too far to draw. With no
/// viewer it is the finest.
pub(crate) fn submodel_camera_lod(skel: &ModelSkel, view: Option<DObjLodView>) -> Option<u8> {
    match view {
        None => Some(0),
        Some(view) => smodel_camera_lod(skel.lod, view.origin, 1.0, view.eye, view.ramp),
    }
}

/// Whether every model of a DObj is too far to draw. Unknown without an eye,
/// and a DObj with no models has nothing to cull.
pub(super) fn lod_culled(
    skels: &[&ModelSkel],
    origin: [f32; 3],
    eye: Option<[f32; 3]>,
    ramp: LodRampArgs,
) -> bool {
    let Some(eye) = eye.map(Vec3::from_array) else {
        return false;
    };
    !skels.is_empty()
        && skels
            .iter()
            .all(|skel| smodel_camera_lod(skel.lod, origin, 1.0, Some(eye), ramp).is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_skel::blank_skel;

    #[test]
    fn without_a_viewer_a_model_is_drawn_finest() {
        assert_eq!(submodel_camera_lod(&blank_skel("crate"), None), Some(0));
    }

    #[test]
    fn nothing_is_culled_without_an_eye() {
        let skel = blank_skel("crate");
        assert!(!lod_culled(&[&skel], [0.0; 3], None, LodRampArgs::default()));
    }

    #[test]
    fn a_dobj_without_models_is_not_culled() {
        assert!(!lod_culled(&[], [0.0; 3], Some([1.0; 3]), LodRampArgs::default()));
    }
}
