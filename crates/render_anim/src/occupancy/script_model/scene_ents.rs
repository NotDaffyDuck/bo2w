//! Telling the scene about each visible script model: where it is, which
//! skeletons it is made of, and at which LOD each is drawn.

use std::sync::Arc;

use bevy::prelude::*;
use render_scene::{FpvLens, LodRampSkinnedDvar, WorldScriptModelInstance, scene_quat_from_angles};

use super::lod::{DObjLodView, lod_culled, submodel_camera_lod};
use super::skels::presented_skel_arcs;
use super::state::ScriptModelDobjs;
use crate::anim::scene_submission::{AnimDObjSceneModel, AnimDObjSceneSubmission};
use crate::occupancy::t6_body::SlowGuard;

/// One scene model per skeleton: at the LOD it is drawn at (-1 for none).
fn scene_models(
    skels: &[Arc<asset_model::ModelSkel>],
    camera_lods: &[Option<u8>],
) -> Vec<AnimDObjSceneModel> {
    skels
        .iter()
        .zip(camera_lods)
        .map(|(skel, lod)| AnimDObjSceneModel {
            lod: lod.and_then(|lod| i8::try_from(lod).ok()).unwrap_or(-1),
            bone_count: u8::try_from(skel.bones.len()).unwrap_or(u8::MAX),
            skel: Arc::clone(skel),
        })
        .collect()
}

/// Submits every shown, placed, in-range script model to the scene. Hidden
/// bones are cut when the model is posed, so no part bits go with it.
pub(super) fn occupy_script_model_scene_ents(
    assets: Option<Res<asset_world::MapXModelSceneCatalog>>,
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<FpvLens>>,
    owners: Query<(&WorldScriptModelInstance, &Transform, &Visibility)>,
    persist: Res<ScriptModelDobjs>,
    mut submissions: MessageWriter<AnimDObjSceneSubmission>,
    lod_skinned: Res<LodRampSkinnedDvar>,
) {
    let _slow = SlowGuard::new("occupy_script_model_scene_ents");
    let Some(assets) = assets else {
        return;
    };
    let eye = cameras
        .single()
        .ok()
        .map(|(transform, _, _)| transform.translation().to_array());
    let ramp = lod_skinned.args();
    for (owner, transform, visibility) in &owners {
        if *visibility == Visibility::Hidden {
            continue;
        }
        let Some(skels) = presented_skel_arcs(&assets, &owner.dobj_state) else {
            continue;
        };
        let origin = transform.translation.to_array();
        let borrowed: Vec<&asset_model::ModelSkel> = skels.iter().map(|skel| skel.as_ref()).collect();
        if lod_culled(&borrowed, origin, eye, ramp) {
            continue;
        }
        let Some(entnum) = owner.gentity_number.map(u32::from) else {
            continue;
        };
        let view = Some(DObjLodView {
            origin,
            eye: eye.map(Vec3::from_array),
            ramp,
        });
        let camera_lods: Vec<_> = skels
            .iter()
            .map(|skel| submodel_camera_lod(skel, view))
            .collect();
        let models = scene_models(&skels, &camera_lods);
        submissions.write(AnimDObjSceneSubmission {
            render_fx_flags: 0,
            has_tree: owner.dobj_state.tree.is_some(),
            origin,
            lighting_origin: owner.lighting_origin,
            radius: skels.iter().find_map(|skel| skel.radius),
            entnum,
            quat: persist
                .mover_angles
                .get(&owner.id.source_ordinal())
                .map(|&angles| scene_quat_from_angles(angles)),
            occupy_model_n: u8::try_from(models.len()).unwrap_or(u8::MAX),
            models,
            hide_part_bits: [0; 5],
            store_skin: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_skel::skel_with_bones;

    #[test]
    fn each_skeleton_is_submitted_at_its_lod_with_its_bone_count() {
        let skels = [Arc::new(skel_with_bones("a", 3)), Arc::new(skel_with_bones("b", 7))];
        let models = scene_models(&skels, &[Some(2), None]);
        assert_eq!((models[0].lod, models[0].bone_count), (2, 3));
        assert_eq!((models[1].lod, models[1].bone_count), (-1, 7));
    }

    #[test]
    fn a_huge_skeleton_counts_as_the_most_bones_a_submission_can_say() {
        let skels = [Arc::new(skel_with_bones("big", 300))];
        assert_eq!(scene_models(&skels, &[Some(0)])[0].bone_count, u8::MAX);
    }
}
