//! bo2zm (Die Rise chat): a body gone limp (`startragdoll`) is posed by its
//! fall from the pose it had then, not by its animation.

use bevy::prelude::*;
use xmodel_runtime::{PosedSkeleton, DObjRagdoll, RigPoseRecord};

use super::advance_script_anim;
use crate::occupancy::zm_ragdoll::{AHEAD_S, RagdollPose, Ragdolls};
use render_scene::LevelScriptModelInstance;
use diag::info;

fn looping(animation_refs: Option<&assets::StagedXAnims>) -> impl Fn(&str) -> Option<bool> + '_ {
    move |clip| {
        let clip = animation_refs?.0.clip(asset_core::AssetCategoryScope::T6, clip)?;
        Some(clip.looping)
    }
}

/// How far a limp body has fallen from where it went limp (where to cull,
/// light and shade it); zero for any other model.
pub(super) fn shift(ragdolls: &Ragdolls, owner_in: &LevelScriptModelInstance) -> Vec3 {
    owner_in
        .puppet_state
        .ragdoll
        .and(owner_in.server_owned_model_owner)
        .and_then(|owner_in| owner_in.scripted_model())
        .and_then(|id| ragdolls.shift(id.encode_for_net()))
        .unwrap_or(Vec3::ZERO)
}

/// BO2MP_RAGLOG=1 (a test aid): once per fall, the check that stopped a limp
/// body being drawn.
pub(super) fn raglog_skip(owner: &LevelScriptModelInstance, why: &str) {
    let Some(ragdoll) = owner.puppet_state.ragdoll.as_ref() else {
        return;
    };
    if std::env::var_os("BO2MP_RAGLOG").is_none() {
        return;
    }
    static SEEN: std::sync::Mutex<Vec<(u32, i32, String)>> = std::sync::Mutex::new(Vec::new());
    let key = (
        owner.game_actor_number.map_or(u32::MAX, u32::from),
        ragdoll.began_at_ms,
        why.to_string(),
    );
    let mut seen = SEEN.lock().unwrap();
    if !seen.contains(&key) {
        info!(World, "raglog ent{} skipped: {why}", key.0);
        seen.push(key);
    }
}

/// The pose of limp body `id` now, run on to `clock_millis`, with the fall it is
/// in. `None` when `owner` is not limp, or the fall cannot be posed (it then
/// keeps its animation).
#[allow(clippy::too_many_arguments, reason = "kept as written")]
pub(super) fn pose_limp(
    ragdolls: &mut Ragdolls,
    id: u32,
    owner: &LevelScriptModelInstance,
    state: &RigPoseRecord,
    obj: &PosedSkeleton,
    scene_from_local: Mat4,
    clock_millis: i32,
    solids: Option<&asset_world::CollisionClipData>,
    ahead_s: f32,
    animation_refs: Option<&assets::StagedXAnims>,
) -> Option<(DObjRagdoll, RagdollPose)> {
    let ragdoll = owner.puppet_state.ragdoll.as_ref()?;
    let request = match state.look_up_pose_query(|name| animation_refs?.0.clip(asset_core::AssetCategoryScope::T6, name)) {
        Ok(request) => request,
        Err(_) => {
            raglog_skip(owner, "request");
            return None;
        }
    };
    let pose = ragdolls.pose(id, ragdoll, obj, scene_from_local, clock_millis, solids, || {
        let ident = Mat4::IDENTITY;
    let now = xmodel_runtime::pose_dobj(obj, &request, ident).ok()?;
        let ahead = advance_script_anim(&owner.puppet_state, ahead_s + AHEAD_S, looping(animation_refs))
            .and_then(|later| {
                later
                    .look_up_pose_query(|name| animation_refs?.0.clip(asset_core::AssetCategoryScope::T6, name))
                    .ok()
            })
            .and_then(|later| xmodel_runtime::pose_dobj(obj, &later, Mat4::IDENTITY).ok())
            .unwrap_or_else(|| now.clone());
        Some((now, ahead))
    })?;
    Some((*ragdoll, pose))
}

/// The cache key a limp body's drawn pose is stored under: the body's
/// number and its fall's step, so each step is posed once and a still body
/// never again.
pub(super) fn limp_key(
    owner: &LevelScriptModelInstance,
    id: u32,
    ragdoll: &DObjRagdoll,
    pose: &RagdollPose,
) -> RigPoseRecord {
    let mut key = owner.puppet_state.clone();
    key.tree = None;
    key.pose_epoch = id;
    key.ragdoll = Some(DObjRagdoll {
        began_at_ms: ragdoll.began_at_ms,
        launch: [0.0; 3],
        launches: pose.stamp as u16,
    });
    key
}
