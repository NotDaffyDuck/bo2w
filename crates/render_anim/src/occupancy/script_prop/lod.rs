//! Which level of detail each model of a DObj is drawn at, for a viewer.

use asset_model::MeshRig;
use bevy::prelude::*;
use render_scene::{DetailRampArgs, prop_camera_detail};

/// Where a DObj stands in relation to the camera.
#[derive(Clone, Copy, Debug)]
pub struct DetailLevelWindow {
    pub spot: [f32; 3],
    pub eye_pos: Option<Vec3>,
    pub ramp: DetailRampArgs,
}

/// The LOD `bone_rig` is drawn at, `None` if it is too far to draw. With no
/// viewer it is the finest.
pub(crate) fn submodel_camera_lod(bone_rig: &MeshRig, vw: Option<DetailLevelWindow>) -> Option<u8> {
    match vw {
        None => Some(0),
        Some(vw) => prop_camera_detail(bone_rig.lod, vw.spot, 1.0, vw.eye_pos, vw.ramp),
    }
}

/// Whether every model of a DObj is too far to draw. Unknown without an eye,
/// and a DObj with no models has nothing to cull.
pub(super) fn lod_culled(
    skels: &[&MeshRig],
    pos: [f32; 3],
    eye_in: Option<[f32; 3]>,
    ramp: DetailRampArgs,
) -> bool {
    let Some(eye_in) = eye_in.map(Vec3::from_array) else {
        return false;
    };
    !skels.is_empty()
        && skels
            .iter()
            .all(|bone_rig| prop_camera_detail(bone_rig.lod, pos, 1.0, Some(eye_in), ramp).is_none())
}

mod tests {
    #![cfg(test)]
    use super::*;
    use crate::motion::test_skel::blank_skel;

    #[test]
    fn without_a_viewer_a_model_is_drawn_finest() {
        assert_eq!(submodel_camera_lod(&blank_skel("crate"), None), Some(0));
    }

    #[test]
    fn nothing_is_culled_without_an_eye() {
        let bone_rig = blank_skel("crate");
        assert!(!lod_culled(&[&bone_rig], [0.0; 3], None, DetailRampArgs::default()));
    }

    #[test]
    fn a_dobj_without_models_is_not_culled() {
        assert!(!lod_culled(&[], [0.0; 3], Some([1.0; 3]), DetailRampArgs::default()));
    }
}
