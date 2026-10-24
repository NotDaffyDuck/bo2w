//! BO2's scene DObjs and models in the per-cell walk (R_AddCellSceneEntSurfacesInFrustum):
//! the GfxSceneDObj cull state, the child planes a cell tests, the sphere pre-test and the
//! entVisData bytes. The box tests are [`crate::bounds_culled`] (p-vertex, dot + d <= 0).

use crate::aabb::Bounds;

/// gfxCfg.entCount: ents per client.
pub const GFX_CFG_ENT_COUNT: u32 = 0x800;

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

/// DObjAnimMat: one posed bone (quat, trans, transWeight).
#[derive(Clone, Copy, Debug)]
pub struct DObjAnimMat {
    pub quat: [f32; 4],
    pub trans: [f32; 3],
    pub trans_weight: f32,
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
    frustum_plane_count: u8,
    plane_count: u8,
) -> &[[f32; 4]] {
    let begin = usize::from(frustum_plane_count);
    let end = usize::from(plane_count).min(planes.len());
    planes.get(begin..end).unwrap_or(&[])
}

/// The sphere test: dot(plane, origin) + d + radius <= 0 culls.
#[must_use]
pub fn scene_ent_sphere_culled(origin: [f32; 3], radius: f32, planes: &[[f32; 4]]) -> bool {
    planes.iter().any(|&plane| {
        plane[0] * origin[0] + plane[1] * origin[1] + plane[2] * origin[2] + plane[3] + radius
            <= 0.0
    })
}

/// A new sceneDObj slot's bounds: the cube origin +/- radius.
#[must_use]
pub fn scene_dobj_initial_bounds(origin: [f32; 3], radius: f32) -> Bounds {
    Bounds::from_mid_half(origin, [radius; 3])
}

/// entVisData[view][entnum] = 1.
pub fn mark_scene_ent_visible(table: &mut [u8], entnum: u32) {
    if let Some(slot) = table.get_mut(entnum as usize) {
        *slot = 1;
    }
}

#[must_use]
pub fn scene_ent_is_visible(table: &[u8], entnum: u32) -> bool {
    table.get(entnum as usize).is_some_and(|&b| b != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounds_culled;

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
        let cube = scene_dobj_initial_bounds([-3.0, 0.0, 0.0], 3.5);
        assert!(!bounds_culled(cube, &planes));
        assert!(bounds_culled(scene_dobj_initial_bounds([-3.0, 0.0, 0.0], 3.0), &planes));
    }

    #[test]
    fn vis_bytes() {
        let mut table = [0u8; 4];
        mark_scene_ent_visible(&mut table, 2);
        mark_scene_ent_visible(&mut table, 9);
        assert!(scene_ent_is_visible(&table, 2) && !scene_ent_is_visible(&table, 1));
        assert!(!scene_ent_is_visible(&table, 9));
    }
}
