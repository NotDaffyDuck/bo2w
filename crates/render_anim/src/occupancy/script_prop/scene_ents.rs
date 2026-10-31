//! Telling the scene about each visible script model: where it is, which
//! skeletons it is made of, and at which LOD each is drawn.

use std::sync::Arc;

use bevy::prelude::*;
use render_scene::{FirstPersonLens, DetailRampSkinnedTunable, LevelScriptModelInstance, orientation_quat_from_euler};

use super::limp;
use super::lod::{DetailLevelWindow, lod_culled, submodel_camera_lod};
use super::skels::shown_skeleton_handles;
use super::state::ScriptPropSkeletons;
use crate::motion::scene_feed::{SceneSkeletonModel, SkeletonSceneHandoff};
use crate::occupancy::zm_ragdoll::Ragdolls;
use crate::occupancy::t6_body::SlowGuard;

/// One scene model per skeleton: at the LOD it is drawn at (-1 for none).
fn stage_models(pieces: &[Arc<asset_model::MeshRig>], detail_tiers: &[Option<u8>]) -> Vec<SceneSkeletonModel> {
    let mut out = Vec::with_capacity(pieces.len());
    for (part, lod) in pieces.iter().zip(detail_tiers) {
        let drawn_at = lod.map_or(-1, |level| i8::try_from(level).unwrap_or(-1));
        let bones = part.bones.len();
        out.push(SceneSkeletonModel {
            bone_rig: Arc::clone(part),
            lod: drawn_at,
            joint_total: bones.min(usize::from(u8::MAX)) as u8,
        });
    }
    out
}

/// Submits every shown, placed, in-range script model to the scene. Hidden
/// bones are cut when the model is posed, so no part bits go with it.
pub(super) fn occupy_scripted_model_scene_ents(
    mut out: MessageWriter<SkeletonSceneHandoff>,
    detail_skinned: Res<DetailRampSkinnedTunable>,
    persist: Res<ScriptPropSkeletons>,
    ragdolls: Res<Ragdolls>,
    scene_cat: Option<Res<asset_world::LevelModelPlacementRoster>>,
    lenses: Query<&GlobalTransform, With<FirstPersonLens>>,
    shown: Query<(&'static LevelScriptModelInstance, &'static Transform, &'static Visibility)>,
) {
    let _slow = SlowGuard::new("occupy_script_model_scene_ents");
    let Some(scene_cat) = scene_cat else {
        return;
    };
    let eye = lenses.single().ok().map(|lens| lens.translation().to_array());
    let ramp = detail_skinned.args();
    for (model, at, visibility) in shown.iter() {
        if matches!(visibility, Visibility::Hidden) {
            continue;
        }
        let Some(parts) = shown_skeleton_handles(&scene_cat, &model.puppet_state) else {
            continue;
        };
        // bo2zm: a limp body is placed where its fall has taken it.
        let shift = limp::shift(&ragdolls, model);
        let origin = (at.translation + shift).to_array();
        let refs: Vec<&asset_model::MeshRig> = parts.iter().map(Arc::as_ref).collect();
        if lod_culled(&refs, origin, eye, ramp) {
            continue;
        }
        let Some(entnum) = model.game_actor_number.map(u32::from) else {
            continue;
        };
        let lod_view = DetailLevelWindow {
            eye_pos: eye.map(Vec3::from_array),
            spot: origin,
            ramp,
        };
        let detail_tiers: Vec<_> = parts
            .iter()
            .map(|part| submodel_camera_lod(part, Some(lod_view)))
            .collect::<_>();
        let models = stage_models(&parts, &detail_tiers);
        let spin = persist.mover_angles.get(&model.id.origin_sequence_number());
        out.write(SkeletonSceneHandoff {
            ent_ix: entnum,
            origin,
            illum_origin: (at.transform_point(refs.first().map_or(Vec3::ZERO, |&model0| Vec3::from_array(crate::model_light_offset(model0)))) + shift).to_array(),
            radius: parts.iter().find_map(|part| part.radius),
            rot_quat: spin.map(|&angles| orientation_quat_from_euler(angles)),
            occupy_model_tally: models.len().min(usize::from(u8::MAX)) as u8,
            models,
            carries_anim_graph: model.puppet_state.tree.is_some(),
            render_effect_flags: 0,
            hidden_mask: [0; 5],
            keep_skinned_verts: true,
        });
    }
}

mod tests {
    #![cfg(test)]
    use super::*;
    use crate::motion::test_skel::skel_with_bones;

    #[test]
    fn each_skeleton_is_submitted_at_its_lod_with_its_bone_count() {
        let skels = [Arc::new(skel_with_bones("a", 3)), Arc::new(skel_with_bones("b", 7))];
        let models = stage_models(&skels, &[Some(2), None]);
        assert_eq!((models[0].lod, models[0].joint_total), (2, 3));
        assert_eq!((models[1].lod, models[1].joint_total), (-1, 7));
    }

    #[test]
    fn a_huge_skeleton_counts_as_the_most_bones_a_submission_can_say() {
        let skels = [Arc::new(skel_with_bones("big", 300))];
        assert_eq!(stage_models(&skels, &[Some(0)])[0].joint_total, u8::MAX);
    }
}
