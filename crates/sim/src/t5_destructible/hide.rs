//! Keeping a prop's drawn model in step with its pieces' health.

use super::hit_log;
use crate::AuthorityDObjState;

/// Hides the bones the pieces' current stages no longer show. When the set changes, the model is rebuilt at once, as
/// BO2's DObj does, so a bullet later in the same tick misses the part that just broke off instead of finding no shape
/// at all and stopping on the car's walk brushes.
pub(super) fn refresh_hidden_parts(dobj: &mut AuthorityDObjState) {
    let (Some(state), Some(cap)) = (&dobj.t5_destructible, &dobj.capability) else {
        return;
    };
    let hidden = state.definition.hide_parts(&state.health, &cap.pose.bone_names);
    if hidden == dobj.semantic_state.hide_part_bits {
        return;
    }
    if hit_log() {
        diag::info!(
            Sim,
            "bo2mp destructible {} hides {:08x?} (bones {:?}, health {:?})",
            state.definition.name,
            hidden.words(),
            cap.pose.bone_names,
            state.health
        );
    }
    dobj.semantic_state.hide_part_bits = hidden;
    dobj.pose_request.hide_part_bits = hidden;
    dobj.pose_revision = dobj.pose_revision.wrapping_add(1);
    dobj.semantic_state.pose_revision = dobj.pose_revision;
    dobj.current_collision = None;
    dobj.materialized_pose_revision = None;
    dobj.materialize();
}
