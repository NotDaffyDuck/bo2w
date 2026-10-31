//! Keeping a prop's drawn model in step with its pieces' health.

use super::hit_log;
use crate::ServerModelState;

/// Hides the bones the pieces' current stages no longer show. When the set changes, the model is rebuilt at once, as
/// BO2's DObj does, so a bullet later in the same tick misses the part that just broke off instead of finding no shape
/// at all and stopping on the car's walk brushes.
pub(super) fn refresh_hidden_parts(dobj: &mut ServerModelState) {
    let (Some(state), Some(cap)) = (&dobj.breakable, &dobj.can_do) else {
        return;
    };
    let masked = state.blueprint.hidden_parts(&state.piece_health, &cap.pose.joint_labels);
    if masked == dobj.bone_role_state.hidden_mask {
        return;
    }
    if hit_log() {
        diag::info!(
            Sim,
            "bo2mp destructible {} hides {:08x?} (bones {:?}, health {:?})",
            state.blueprint.name,
            masked.words(),
            cap.pose.joint_labels,
            state.piece_health
        );
    }
    dobj.bone_role_state.hidden_mask = masked;
    dobj.pose_query.hidden_mask = masked;
    let revision = dobj.pose_rev.wrapping_add(1);
    dobj.pose_rev = revision;
    dobj.bone_role_state.pose_epoch = revision;
    (dobj.live_hitbox, dobj.built_pose_stamp) = (None, None);
    ServerModelState::bake_collision_shape(dobj);
}
