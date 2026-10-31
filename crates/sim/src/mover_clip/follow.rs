use crate::mover_clip::{Placement, is_hidden, movers_by_id};
use crate::shot_trace::EntityHitAbilities;
use crate::ent_slots::MoverSlot;

/// Brings every row owned by one of `movers` to that mover's visibility, solidity and pose at `clock_stamp_ms`. A row that
/// has not followed before only remembers the pose; one whose mover has moved since moves its model and brushes too.
pub(crate) fn follow_movers(rows: &mut [EntityHitAbilities], movers: &[MoverSlot], clock_stamp_ms: i32) {
    let by_id = movers_by_id(movers);
    for row in rows.iter_mut() {
        let Some(&mover) = row.owner.scripted_model().and_then(|id| by_id.get(&id)) else {
            continue;
        };
        row.unseen = is_hidden(mover);
        row.solid = !mover.nonsolid;

        let now = Placement::of(mover, clock_stamp_ms);
        let before = row.followed_pose.replace((now.at, now.turn));
        if before.is_some_and(|(at, turn)| Placement { at, turn } != now) {
            carry_to(row, now);
        }
    }
}

/// Moves a row's model and brushes to `pose`.
fn carry_to(row: &mut EntityHitAbilities, pose: Placement) {
    if let Some(dobj) = &mut row.dobj {
        dobj.set_world_pose(pose.at, pose.turn);
    }
    for brush in &mut row.attached_solids {
        (brush.origin, brush.turn) = (pose.at, pose.turn);
    }
}
