//! BO2 scene view and projection setup (scratch/hud-facts/view_projection.md): near-clip choice,
//! absolute view matrix, exact reversed-depth projections and the per-draw depth-hack and
//! camera-relative derivation. Matrices are row-major, row vectors (position times matrix).

use crate::set_2d::{
    GFX_VIEWPARMS_AXIS, GFX_VIEWPARMS_DEPTH_HACK_NEAR_CLIP, GFX_VIEWPARMS_DWORDS, GFX_VIEWPARMS_INV_VP,
    GFX_VIEWPARMS_PROJECTION, GFX_VIEWPARMS_VIEW, GFX_VIEWPARMS_VIEW_PROJECTION, GFX_VIEWPARMS_Z_NEAR,
    gfx_viewparms_write_matrix, gfx_viewparms_write_origin,
};

/// r_znear: default 4, range 1..10000.
pub const R_ZNEAR_DEFAULT: f32 = 4.0;
pub const R_ZNEAR_MIN: f32 = 1.0;
pub const R_ZNEAR_MAX: f32 = 10000.0;
/// r_znear_depthhack ("Viewmodel near clip plane"): default 0.1, range 0.001..16, use-time floor 0.1.
pub const R_ZNEAR_DEPTHHACK_DEFAULT: f32 = 0.1;
pub const R_ZNEAR_DEPTHHACK_MIN: f32 = 0.001;
pub const R_ZNEAR_DEPTHHACK_MAX: f32 = 16.0;
/// r_zfar: 0 is off. Read by visibility, not by the (infinite) scene projection.
pub const R_ZFAR_DEFAULT: f32 = 0.0;
/// depthHackFlags value that swaps in the depth-hack near clip.
pub const DEPTH_HACK_FLAG: u32 = 0x4000_0000;

/// R_GetDefaultNearClip: r_znear clamped to [1, 4].
#[must_use]
pub fn default_near_clip(r_znear: f32) -> f32 {
    r_znear.clamp(1.0, 4.0)
}

/// R_GetDefaultNearClip_DepthHack: r_znear_depthhack, never below 0.1.
#[must_use]
pub fn depth_hack_near_clip(r_znear_depthhack: f32) -> f32 {
    r_znear_depthhack.max(0.1)
}

/// Scene zNear: the refdef value when positive, else the default near clip; never below 1.
#[must_use]
pub fn znear_from_refdef(refdef_z_near: f32, r_znear: f32) -> f32 {
    let z = if refdef_z_near <= 0.0 { default_near_clip(r_znear) } else { refdef_z_near };
    if z <= 1.0 { 1.0 } else { z }
}

/// InfinitePerspectiveMatrix: reversed depth, z_ndc = zNear / viewZ.
#[must_use]
pub fn infinite_perspective_matrix(tan_half_x: f32, tan_half_y: f32, z_near: f32) -> [f32; 16] {
    debug_assert!(z_near > 0.0);
    let mut m = [0.0f32; 16];
    m[0] = 1.0 / tan_half_x;
    m[5] = 1.0 / tan_half_y;
    m[11] = 1.0;
    m[14] = z_near;
    m
}

/// FinitePerspectiveMatrix: reversed depth, 1 at the near plane and 0 at the far plane.
#[must_use]
pub fn finite_perspective_matrix(tan_half_x: f32, tan_half_y: f32, z_near: f32, z_far: f32) -> [f32; 16] {
    debug_assert!(z_near > 0.0 && z_far > z_near);
    let range = z_far - z_near;
    let mut m = [0.0f32; 16];
    m[0] = 1.0 / tan_half_x;
    m[5] = 1.0 / tan_half_y;
    m[10] = -(z_near / range);
    m[11] = 1.0;
    m[14] = z_near * z_far / range;
    m
}

/// MatrixForViewer: absolute view matrix (rotation plus the origin's translation row).
#[must_use]
pub fn matrix_for_viewer(origin: [f32; 3], axis: [[f32; 3]; 3]) -> [f32; 16] {
    let mut m = [0.0f32; 16];
    for r in 0..3 {
        m[r * 4] = -axis[1][r];
        m[r * 4 + 1] = axis[2][r];
        m[r * 4 + 2] = axis[0][r];
    }
    for c in 0..3 {
        m[12 + c] = -(origin[0] * m[c] + origin[1] * m[4 + c] + origin[2] * m[8 + c]);
    }
    m[15] = 1.0;
    m
}

/// MatrixMultiply44: a then b.
#[must_use]
pub fn matrix_multiply44(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    core::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum()
    })
}

/// MatrixInverse44: general 4x4 inverse; None when singular.
#[must_use]
pub fn matrix_inverse44(m: &[f32; 16]) -> Option<[f32; 16]> {
    let a = m.map(f64::from);
    let s0 = a[0] * a[5] - a[4] * a[1];
    let s1 = a[0] * a[6] - a[4] * a[2];
    let s2 = a[0] * a[7] - a[4] * a[3];
    let s3 = a[1] * a[6] - a[5] * a[2];
    let s4 = a[1] * a[7] - a[5] * a[3];
    let s5 = a[2] * a[7] - a[6] * a[3];
    let c5 = a[10] * a[15] - a[14] * a[11];
    let c4 = a[9] * a[15] - a[13] * a[11];
    let c3 = a[9] * a[14] - a[13] * a[10];
    let c2 = a[8] * a[15] - a[12] * a[11];
    let c1 = a[8] * a[14] - a[12] * a[10];
    let c0 = a[8] * a[13] - a[12] * a[9];
    let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let d = 1.0 / det;
    let r = [
        (a[5] * c5 - a[6] * c4 + a[7] * c3) * d,
        (-a[1] * c5 + a[2] * c4 - a[3] * c3) * d,
        (a[13] * s5 - a[14] * s4 + a[15] * s3) * d,
        (-a[9] * s5 + a[10] * s4 - a[11] * s3) * d,
        (-a[4] * c5 + a[6] * c2 - a[7] * c1) * d,
        (a[0] * c5 - a[2] * c2 + a[3] * c1) * d,
        (-a[12] * s5 + a[14] * s2 - a[15] * s1) * d,
        (a[8] * s5 - a[10] * s2 + a[11] * s1) * d,
        (a[4] * c4 - a[5] * c2 + a[7] * c0) * d,
        (-a[0] * c4 + a[1] * c2 - a[3] * c0) * d,
        (a[12] * s4 - a[13] * s2 + a[15] * s0) * d,
        (-a[8] * s4 + a[9] * s2 - a[11] * s0) * d,
        (-a[4] * c3 + a[5] * c1 - a[6] * c0) * d,
        (a[0] * c3 - a[1] * c1 + a[2] * c0) * d,
        (-a[12] * s3 + a[13] * s1 - a[14] * s0) * d,
        (a[8] * s3 - a[9] * s1 + a[10] * s0) * d,
    ];
    Some(r.map(|v| v as f32))
}

/// The refdef fields scene setup reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRefdef {
    pub vieworg: [f32; 3],
    pub viewaxis: [[f32; 3]; 3],
    pub tan_half_fov_x: f32,
    pub tan_half_fov_y: f32,
    pub z_near: f32,
}

/// R_SetViewParmsForScene: zeroed parms, origin (w 1) and axis, absolute view, infinite projection,
/// depth-hack near clip, view x projection and its plain inverse.
#[must_use]
pub fn set_view_parms_for_scene(
    refdef: &SceneRefdef,
    r_znear: f32,
    r_znear_depthhack: f32,
) -> [u32; GFX_VIEWPARMS_DWORDS] {
    let mut p = [0u32; GFX_VIEWPARMS_DWORDS];
    let o = refdef.vieworg;
    gfx_viewparms_write_origin(&mut p, [o[0], o[1], o[2], 1.0]);
    for (i, row) in refdef.viewaxis.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            p[GFX_VIEWPARMS_AXIS / 4 + i * 3 + j] = v.to_bits();
        }
    }
    let view = matrix_for_viewer(o, refdef.viewaxis);
    gfx_viewparms_write_matrix(&mut p, GFX_VIEWPARMS_VIEW, &view);
    let z_near = znear_from_refdef(refdef.z_near, r_znear);
    p[GFX_VIEWPARMS_Z_NEAR / 4] = z_near.to_bits();
    let proj = infinite_perspective_matrix(refdef.tan_half_fov_x, refdef.tan_half_fov_y, z_near);
    gfx_viewparms_write_matrix(&mut p, GFX_VIEWPARMS_PROJECTION, &proj);
    p[GFX_VIEWPARMS_DEPTH_HACK_NEAR_CLIP / 4] = depth_hack_near_clip(r_znear_depthhack).to_bits();
    setup_view_projection_matrices(&mut p, &view, &proj);
    p
}

/// R_SetupViewProjectionMatrices: view x projection and its inverse (zero when singular).
pub fn setup_view_projection_matrices(p: &mut [u32; GFX_VIEWPARMS_DWORDS], view: &[f32; 16], proj: &[f32; 16]) {
    let vp = matrix_multiply44(view, proj);
    gfx_viewparms_write_matrix(p, GFX_VIEWPARMS_VIEW_PROJECTION, &vp);
    let inv = matrix_inverse44(&vp).unwrap_or([0.0; 16]);
    gfx_viewparms_write_matrix(p, GFX_VIEWPARMS_INV_VP, &inv);
}

/// R_DeriveProjectionMatrix: with the depth-hack flag, element 14 becomes the depth-hack near clip;
/// a w column other than 0 or 1 in element 11 divides the whole matrix.
#[must_use]
pub fn derive_projection_matrix(projection: &[f32; 16], depth_hack_flags: u32, depth_hack_near_clip: f32) -> [f32; 16] {
    let mut m = *projection;
    if depth_hack_flags == DEPTH_HACK_FLAG {
        m[14] = depth_hack_near_clip;
    }
    let w = m[11];
    if w != 0.0 && w != 1.0 {
        m = m.map(|v| v / w);
    }
    m
}

/// R_DeriveViewProjectionMatrix: view x depth-hacked projection when the flag is set, else the stored
/// view-projection; then the translation row becomes the eye offset pushed through the matrix, so
/// draws in camera-relative positions land where the absolute ones would.
#[must_use]
pub fn derive_view_projection_matrix(
    view: &[f32; 16],
    view_projection: &[f32; 16],
    derived_projection: &[f32; 16],
    depth_hack_flags: u32,
    eye_offset: [f32; 3],
) -> [f32; 16] {
    let mut m = if depth_hack_flags == DEPTH_HACK_FLAG {
        matrix_multiply44(view, derived_projection)
    } else {
        *view_projection
    };
    let t: [f32; 4] =
        core::array::from_fn(|c| eye_offset[0] * m[c] + eye_offset[1] * m[4 + c] + eye_offset[2] * m[8 + c] + m[12 + c]);
    m[12..16].copy_from_slice(&t);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set_2d::gfx_viewparms_read_matrix;

    const AXIS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn xform(p: [f32; 4], m: &[f32; 16]) -> [f32; 4] {
        core::array::from_fn(|c| (0..4).map(|k| p[k] * m[k * 4 + c]).sum())
    }

    #[test]
    fn near_clip_rules() {
        assert_eq!(default_near_clip(8.0), 4.0);
        assert_eq!(default_near_clip(0.5), 1.0);
        assert_eq!(default_near_clip(2.5), 2.5);
        assert_eq!(znear_from_refdef(0.0, 4.0), 4.0);
        assert_eq!(znear_from_refdef(0.5, 4.0), 1.0);
        assert_eq!(znear_from_refdef(6.0, 4.0), 6.0);
        assert_eq!(depth_hack_near_clip(0.01), 0.1);
        assert_eq!(depth_hack_near_clip(0.5), 0.5);
    }

    #[test]
    fn reversed_depth() {
        let m = infinite_perspective_matrix(1.0, 1.0, 4.0);
        // A point 4 units ahead (view z = 4) is depth 1; farther tends to 0.
        let near = xform([0.0, 0.0, 4.0, 1.0], &m);
        assert_eq!(near[2] / near[3], 1.0);
        let far = xform([0.0, 0.0, 400.0, 1.0], &m);
        assert!((far[2] / far[3] - 0.01).abs() < 1e-6);
        let f = finite_perspective_matrix(1.0, 1.0, 4.0, 100.0);
        let a = xform([0.0, 0.0, 4.0, 1.0], &f);
        let b = xform([0.0, 0.0, 100.0, 1.0], &f);
        assert!((a[2] / a[3] - 1.0).abs() < 1e-6);
        assert!((b[2] / b[3]).abs() < 1e-6);
    }

    #[test]
    fn view_matrix_is_absolute() {
        let v = matrix_for_viewer([10.0, 20.0, 30.0], AXIS);
        // The eye maps to the view-space origin; a point ahead along forward (x) lands on view z.
        assert_eq!(xform([10.0, 20.0, 30.0, 1.0], &v), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(xform([15.0, 20.0, 30.0, 1.0], &v), [0.0, 0.0, 5.0, 1.0]);
    }

    #[test]
    fn scene_parms_and_inverse() {
        let r = SceneRefdef {
            vieworg: [100.0, -50.0, 64.0],
            viewaxis: AXIS,
            tan_half_fov_x: 1.2,
            tan_half_fov_y: 0.75,
            z_near: 0.0,
        };
        let p = set_view_parms_for_scene(&r, 4.0, 0.1);
        assert_eq!(f32::from_bits(p[GFX_VIEWPARMS_Z_NEAR / 4]), 4.0);
        assert_eq!(f32::from_bits(p[GFX_VIEWPARMS_DEPTH_HACK_NEAR_CLIP / 4]), 0.1);
        let vp = gfx_viewparms_read_matrix(&p, GFX_VIEWPARMS_VIEW_PROJECTION);
        let inv = gfx_viewparms_read_matrix(&p, GFX_VIEWPARMS_INV_VP);
        let id = matrix_multiply44(&vp, &inv);
        for (i, v) in id.iter().enumerate() {
            let want = if i % 5 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-4, "element {i} = {v}");
        }
    }

    #[test]
    fn derive_depth_hack_and_camera_relative() {
        let proj = infinite_perspective_matrix(1.0, 1.0, 4.0);
        assert_eq!(derive_projection_matrix(&proj, 0, 0.1)[14], 4.0);
        let dh = derive_projection_matrix(&proj, DEPTH_HACK_FLAG, 0.1);
        assert_eq!(dh[14], 0.1);
        let eye = [100.0, -50.0, 64.0];
        let view = matrix_for_viewer(eye, AXIS);
        let vp = matrix_multiply44(&view, &proj);
        let rel = derive_view_projection_matrix(&view, &vp, &proj, 0, eye);
        // A world point drawn camera-relative lands where the absolute one does.
        let world = [140.0, -45.0, 70.0];
        let a = xform([world[0], world[1], world[2], 1.0], &vp);
        let b = xform([world[0] - eye[0], world[1] - eye[1], world[2] - eye[2], 1.0], &rel);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-3);
        }
        let mut odd = proj;
        odd[11] = 2.0;
        assert_eq!(derive_projection_matrix(&odd, 0, 0.1)[11], 1.0);
    }
}
