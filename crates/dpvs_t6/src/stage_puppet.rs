//! BO2's scene DObjs and models in the per-cell walk (R_AddCellSceneEntSurfacesInFrustum):
//! the GfxSceneDObj cull state, the child planes a cell tests, the sphere pre-test and the
//! entVisData bytes. The box tests are [`crate::box_outside_planes`] (p-vertex, dot + d <= 0).

use crate::box_tree::Bounds;

/// gfxCfg.entCount: ents per client.
pub const VISUAL_CFG_ACTOR_TALLY: u32 = 0x800;

/// GfxSceneDObj.cull.state: nothing computed yet.
pub const SCENE_DOBJ_CULL_IDLE: u32 = 0;
/// One thread is building the bounds (taken with 0 to 1).
pub const SCENE_DOBJ_CULL_BOUNDING: u32 = 1;
/// Bounds built.
pub const SCENE_DOBJ_CULL_BOUNDED: u32 = 2;
/// Skin pending.
pub const SCENE_DOBJ_CULL_SKIN_PENDING: u32 = 3;
/// Done: the state is surfaceCount + 4, so exactly 4 means nothing to skin.
pub const SCENE_DOBJ_CULL_DONE: u32 = 4;

/// DObjAnimMat: one posed bone (quat, shift_offset, transWeight).
#[derive(Clone, Copy, Debug)]
pub struct BonePose {
    pub turn: [f32; 4],
    pub shift_offset: [f32; 3],
    pub move_blend_amount: f32,
}

/// R_UpdateSceneEntBounds' claim: only an idle DObj moves to bounding.
#[must_use]
pub fn scene_dobj_cull_begin(state: &mut u32) -> bool {
    if *state == SCENE_DOBJ_CULL_IDLE {
        *state = SCENE_DOBJ_CULL_BOUNDING;
        true
    } else {
        false
    }
}

/// cull.state < 2: the cell walk tests the sphere, not the box.
#[must_use]
pub fn scene_dobj_bounds_pending(state: u32) -> bool {
    state < SCENE_DOBJ_CULL_BOUNDED
}

/// The child planes of a cell: frustumPlaneCount up to planeCount. The frustum's own planes
/// are not tested again, so equal counts test nothing.
#[must_use]
pub fn scene_ent_child_planes(
    planes: &[[f32; 4]],
    view_wall_total: u8,
    plane_tally: u8,
) -> &[[f32; 4]] {
    let first_ix = usize::from(view_wall_total);
    let end = usize::from(plane_tally).min(planes.len());
    planes.get(first_ix..end).unwrap_or(&[])
}

/// The sphere test: dot(plane, origin) + d + radius <= 0 culls.
#[must_use]
pub fn scene_ent_sphere_culled(origin: [f32; 3], rad: f32, cut_planes: &[[f32; 4]]) -> bool {
    cut_planes.iter().any(|&cut_plane| {
        cut_plane[0] * origin[0] + cut_plane[1] * origin[1] + cut_plane[2] * origin[2] + cut_plane[3] + rad
            <= 0.0
    })
}

/// A new sceneDObj slot's bounds: the cube origin +/- radius.
#[must_use]
pub fn animated_object_starting_box(origin: [f32; 3], reach: f32) -> Bounds {
    Bounds::from_center_extent(origin, [reach; 3])
}

/// entVisData[view][entnum] = 1.
pub fn stain_stage_actor_visible(table: &mut [u8], ent_ix: u32) {
    if let Some(slot) = table.get_mut(ent_ix as usize) {
        *slot = 1;
    }
}

#[must_use]
pub fn entity_flagged_visible(table: &[u8], actor_ix: u32) -> bool {
    table.get(actor_ix as usize).is_some_and(|&b| b != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_outside_planes;

    #[test]
    fn cull_state_claim() {
        let mut state = SCENE_DOBJ_CULL_IDLE;
        assert!(scene_dobj_bounds_pending(state));
        assert!(scene_dobj_cull_begin(&mut state));
        assert_eq!(state, SCENE_DOBJ_CULL_BOUNDING);
        assert!(!scene_dobj_cull_begin(&mut state));
        assert!(!scene_dobj_bounds_pending(SCENE_DOBJ_CULL_BOUNDED));
    }

    #[test]
    fn child_planes_skip_the_frustum() {
        let planes = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        assert_eq!(scene_ent_child_planes(&planes, 1, 3), &planes[1..]);
        assert!(scene_ent_child_planes(&planes, 3, 3).is_empty());
        assert!(scene_ent_child_planes(&planes, 2, 9).len() == 1);
    }

    #[test]
    fn sphere_then_box() {
        // x >= 0 kept.
        let planes = [[1.0, 0.0, 0.0, 0.0]];
        assert!(scene_ent_sphere_culled([-3.0, 0.0, 0.0], 3.0, &planes));
        assert!(!scene_ent_sphere_culled([-3.0, 0.0, 0.0], 3.5, &planes));
        let cube = animated_object_starting_box([-3.0, 0.0, 0.0], 3.5);
        assert!(!box_outside_planes(cube, &planes));
        assert!(box_outside_planes(animated_object_starting_box([-3.0, 0.0, 0.0], 3.0), &planes));
    }

    #[test]
    fn vis_bytes() {
        let mut table = [0u8; 4];
        stain_stage_actor_visible(&mut table, 2);
        stain_stage_actor_visible(&mut table, 9);
        assert!(entity_flagged_visible(&table, 2) && !entity_flagged_visible(&table, 1));
        assert!(!entity_flagged_visible(&table, 9));
    }
}
