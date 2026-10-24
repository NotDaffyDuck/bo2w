//! Moving the prediction world's collision rows with the script movers that own them.

use crate::bullet_collision::{EntityCollisionCapabilities, LinkedBrushCollisionBrush};
use crate::gentity::{ScriptMoverGentity, apos_from_entity_state, pos_from_entity_state};
use crate::identities::ScriptModelId;
use entity_t6::{evaluate_trajectory, missile::EF_NODRAW, script_mover::SOLID_BMODEL};
use std::collections::BTreeMap;

/// Where a mover is at `time_ms`: its origin and its angles.
fn pose_at(mover: &ScriptMoverGentity, time_ms: i32) -> ([f32; 3], [f32; 3]) {
    (
        evaluate_trajectory(&pos_from_entity_state(&mover.state), time_ms),
        evaluate_trajectory(&apos_from_entity_state(&mover.state), time_ms),
    )
}

/// Gives the prediction world a collision row for every brush mover it has none for (a door or debris pile whose level
/// started after prediction armed): the brush model the mover names, at the mover's pose. Rows made this way are
/// marked, and go when their mover leaves the snapshot; `follow_movers` keeps their pose and solidity afterwards.
pub(crate) fn adopt_brush_movers(capabilities: &mut Vec<EntityCollisionCapabilities>, movers: &[ScriptMoverGentity], time_ms: i32) {
    let brush_movers: BTreeMap<ScriptModelId, &ScriptMoverGentity> = movers
        .iter()
        .filter(|mover| mover.state.solid == SOLID_BMODEL && mover.state.index >= 0)
        .map(|mover| (mover.id, mover))
        .collect();
    let rows_before = capabilities.len();
    capabilities.retain(|row| {
        !row.from_snapshot || row.owner.script_model().is_some_and(|id| brush_movers.contains_key(&id))
    });
    let dropped = rows_before - capabilities.len();
    let mut made = 0;
    for (id, mover) in brush_movers {
        let owner = crate::AuthorityModelOwner::ScriptModel(id);
        if capabilities.binary_search_by_key(&owner, |row| row.owner).is_ok() {
            continue;
        }
        let (origin, angles) = pose_at(mover, time_ms);
        let brush = LinkedBrushCollisionBrush { cmodel_handle: mover.state.index as u32, origin, angles };
        let mut row = EntityCollisionCapabilities::current_tick(owner, None, vec![brush]);
        row.from_snapshot = true;
        row.hidden = mover.state.lerp.e_flags & EF_NODRAW != 0;
        row.solid = !mover.nonsolid;
        row.followed_pose = Some((origin, angles));
        capabilities.push(row);
        made += 1;
    }
    if made > 0 {
        capabilities.sort_by_key(|row| row.owner);
    }
    static LOG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if (made > 0 || dropped > 0) && *LOG.get_or_init(|| std::env::var_os("BO2MP_PRED_LOG").is_some()) {
        diag::info!(
            Sim,
            "prediction brush rows: {made} made, {dropped} dropped, {} in all",
            capabilities.iter().filter(|row| row.from_snapshot).count()
        );
    }
}

/// Brings each row owned by a mover to the mover's visibility, solidity and pose at `time_ms`. A row that has not
/// followed yet only remembers the pose; one whose mover has moved since moves its model and brushes there too.
pub(crate) fn follow_movers(capabilities: &mut [EntityCollisionCapabilities], movers: &[ScriptMoverGentity], time_ms: i32) {
    let by_id: BTreeMap<ScriptModelId, &ScriptMoverGentity> = movers.iter().map(|mover| (mover.id, mover)).collect();
    for row in capabilities {
        let Some(mover) = row.owner.script_model().and_then(|id| by_id.get(&id)) else {
            continue;
        };
        row.hidden = mover.state.lerp.e_flags & EF_NODRAW != 0;
        row.solid = !mover.nonsolid;
        let pose = pose_at(mover, time_ms);
        let moved = row.followed_pose.replace(pose).is_some_and(|followed| followed != pose);
        if !moved {
            continue;
        }
        if let Some(dobj) = row.dobj.as_mut() {
            dobj.set_world_pose(pose.0, pose.1);
        }
        for brush in &mut row.linked_brushes {
            brush.origin = pose.0;
            brush.angles = pose.1;
        }
    }
}
