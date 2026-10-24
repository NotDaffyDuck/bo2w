use bevy::prelude::*;

use hud_t6::set_2d::{GFX_VIEWPARMS_DWORDS, GfxViewport};

use crate::frustum::{FrustumSpec, perspective_frustum_planes};

#[must_use]
pub fn pack_live_view_parms(
    view_from_world: Mat4,
    clip_from_view: Mat4,
    clip_from_world: Mat4,
    view_origin: Vec3,
) -> [u32; GFX_VIEWPARMS_DWORDS] {
    use hud_t6::set_2d::{
        GFX_VIEWPARMS_INV_VP, GFX_VIEWPARMS_PROJECTION, GFX_VIEWPARMS_VIEW,
        GFX_VIEWPARMS_VIEW_PROJECTION, gfx_viewparms_write_matrix, gfx_viewparms_write_origin,
    };

    let mut parms = [0u32; GFX_VIEWPARMS_DWORDS];
    gfx_viewparms_write_matrix(&mut parms, GFX_VIEWPARMS_VIEW, &view_from_world.to_cols_array());
    gfx_viewparms_write_matrix(&mut parms, GFX_VIEWPARMS_PROJECTION, &clip_from_view.to_cols_array());
    gfx_viewparms_write_matrix(
        &mut parms,
        GFX_VIEWPARMS_VIEW_PROJECTION,
        &clip_from_world.to_cols_array(),
    );
    let det = clip_from_world.determinant();
    if det != 0.0 && det.is_finite() {
        let inv = clip_from_world.inverse();
        gfx_viewparms_write_matrix(&mut parms, GFX_VIEWPARMS_INV_VP, &inv.to_cols_array());
    }
    let o = view_origin;
    gfx_viewparms_write_origin(&mut parms, [o.x, o.y, o.z, 1.0]);
    parms
}

#[must_use]
pub fn host_clip_from_view(fov: f32, aspect: f32, near: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fov, aspect, near)
}

#[derive(Resource, Clone, Debug)]
pub struct PreparedSceneView {
    pub ready: bool,
    pub eye: Vec3,

    pub forward: Vec3,
    pub view_from_world: Mat4,
    pub clip_from_view: Mat4,
    pub clip_from_world: Mat4,
    pub frustum_planes: Vec<[f32; 4]>,
    pub fov: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
    pub rt_w: i32,
    pub rt_h: i32,
    pub parms: [u32; GFX_VIEWPARMS_DWORDS],

    pub portal_bevels: Option<dpvs_t6::Bevels>,

    pub depth_hack_near: f32,

    /// BO2 has no sub-window: the scene viewport is the whole render target.
    pub scene_viewport: GfxViewport,
}

impl PreparedSceneView {
    #[must_use]
    pub fn tan_half_fov_y(&self) -> f32 {
        (self.fov * 0.5).tan()
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct RZnearDvar {
    pub value: f32,
}

impl Default for RZnearDvar {
    fn default() -> Self {
        Self {
            value: hud_t6::view_projection::R_ZNEAR_DEFAULT,
        }
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct RZnearDepthhackDvar {
    pub value: f32,
}

impl Default for RZnearDepthhackDvar {
    fn default() -> Self {
        Self {
            value: hud_t6::view_projection::R_ZNEAR_DEPTHHACK_DEFAULT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LockPvsView {
    pub eye: Vec3,
    pub forward: Vec3,
    pub frustum_planes: Vec<[f32; 4]>,
    pub portal_bevels: Option<dpvs_t6::Bevels>,
    pub fov: f32,
    pub aspect: f32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct RLockPvs {
    pub enabled: bool,

    pub recapture: bool,
    pub frozen: Option<LockPvsView>,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SmEnableDvar {
    pub enabled: Option<bool>,
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SmSunEnableDvar {
    pub enabled: Option<bool>,
}

impl RLockPvs {
    pub fn apply_after_stamp(&mut self, live: &PreparedSceneView) {
        if !self.enabled {
            self.frozen = None;
            self.recapture = false;
            return;
        }
        if self.recapture || self.frozen.is_none() {
            self.frozen = Some(LockPvsView {
                eye: live.eye,
                forward: live.forward,
                frustum_planes: live.frustum_planes.clone(),
                portal_bevels: live.portal_bevels,
                fov: live.fov,
                aspect: live.aspect,
            });
            self.recapture = false;
        }
    }

    #[must_use]
    pub fn dpvs_eye(&self, live: &PreparedSceneView) -> Vec3 {
        match (self.enabled, &self.frozen) {
            (true, Some(f)) => f.eye,
            _ => live.eye,
        }
    }

    #[must_use]
    pub fn dpvs_forward(&self, live: &PreparedSceneView) -> Vec3 {
        match (self.enabled, &self.frozen) {
            (true, Some(f)) => f.forward,
            _ => live.forward,
        }
    }

    #[must_use]
    pub fn dpvs_frustum<'a>(&'a self, live: &'a PreparedSceneView) -> &'a [[f32; 4]] {
        match (self.enabled, &self.frozen) {
            (true, Some(f)) => f.frustum_planes.as_slice(),
            _ => live.frustum_planes.as_slice(),
        }
    }

    #[must_use]
    pub fn dpvs_portal_bevels(&self, live: &PreparedSceneView) -> Option<dpvs_t6::Bevels> {
        match (self.enabled, &self.frozen) {
            (true, Some(f)) => f.portal_bevels,
            _ => live.portal_bevels,
        }
    }

    #[must_use]
    pub fn dpvs_fov_aspect(&self, live: &PreparedSceneView) -> (f32, f32) {
        match (self.enabled, &self.frozen) {
            (true, Some(f)) => (f.fov, f.aspect),
            _ => (live.fov, live.aspect),
        }
    }
}

impl Default for PreparedSceneView {
    fn default() -> Self {
        Self {
            ready: false,
            eye: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            view_from_world: Mat4::IDENTITY,
            clip_from_view: Mat4::IDENTITY,
            clip_from_world: Mat4::IDENTITY,
            frustum_planes: Vec::new(),
            fov: 0.0,
            aspect: 0.0,
            near: 0.0,
            far: 0.0,
            rt_w: 0,
            rt_h: 0,
            parms: [0u32; GFX_VIEWPARMS_DWORDS],
            portal_bevels: None,
            depth_hack_near: hud_t6::view_projection::R_ZNEAR_DEPTHHACK_DEFAULT,
            scene_viewport: GfxViewport::default(),
        }
    }
}

/// BO2 R_SetViewParmsForScene's DPVS matrices: the absolute view times the reversed-depth
/// infinite projection, and its plain inverse; None when singular.
#[must_use]
pub fn scene_view_projection(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    tan_half_x: f32,
    tan_half_y: f32,
    z_near: f32,
) -> Option<([f32; 16], [f32; 16])> {
    use hud_t6::view_projection as vp;
    let view_proj = vp::matrix_multiply44(
        &vp::matrix_for_viewer(origin, axis),
        &vp::infinite_perspective_matrix(tan_half_x, tan_half_y, z_near),
    );
    vp::matrix_inverse44(&view_proj).map(|inv| (view_proj, inv))
}

#[must_use]
pub fn lens_world_from_parent_child(parent: Transform, child: Transform) -> Mat4 {
    parent.to_matrix() * child.to_matrix()
}

#[must_use]
pub fn prepare_scene_view(
    world_from_local: Mat4,
    fov: f32,
    aspect: f32,
    near: f32,
    far: f32,
    rt_w: i32,
    rt_h: i32,
    dpvs_z_near: f32,
    depth_hack_near: f32,
) -> PreparedSceneView {
    let eye = world_from_local.transform_point3(Vec3::ZERO);
    let view_from_world = world_from_local.inverse();
    let clip_from_view = host_clip_from_view(fov, aspect, near);
    let clip_from_world = clip_from_view * view_from_world;
    let forward = world_from_local.transform_vector3(Vec3::NEG_Z);
    let right = world_from_local.transform_vector3(Vec3::X);
    let up = world_from_local.transform_vector3(Vec3::Y);
    let mut frustum_planes = perspective_frustum_planes(FrustumSpec {
        eye,
        forward,
        right,
        up,
        fov_y_rad: fov,
        aspect,
        near,
        far,
    })
    .to_vec();
    if no_frustum_env() {
        frustum_planes.clear();
    } else if frustum_nearfar_env() {
        frustum_planes.truncate(2);
    }
    let parms = pack_live_view_parms(view_from_world, clip_from_view, clip_from_world, eye);
    let tan_half_y = (fov * 0.5).tan();
    let tan_half_x = tan_half_y * aspect;
    // BO2's view axis is forward, LEFT, up (MatrixForViewer takes clip x from -axis[1]).
    let axis = [
        [forward.x, forward.y, forward.z],
        [-right.x, -right.y, -right.z],
        [up.x, up.y, up.z],
    ];
    let portal_bevels =
        scene_view_projection(eye.to_array(), axis, tan_half_x, tan_half_y, dpvs_z_near)
            .map(|(vp, inv)| dpvs_t6::Bevels::from_row_vector(&vp, &inv));
    PreparedSceneView {
        ready: true,
        eye,
        forward,
        view_from_world,
        clip_from_view,
        clip_from_world,
        frustum_planes,
        fov,
        aspect,
        near,
        far,
        rt_w,
        rt_h,
        parms,
        portal_bevels,
        depth_hack_near,
        scene_viewport: GfxViewport { x: 0, y: 0, width: rt_w, height: rt_h },
    }
}

fn no_frustum_env() -> bool {
    matches!(
        std::env::var("BO2MP_NO_FRUSTUM").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn frustum_nearfar_env() -> bool {
    matches!(
        std::env::var("BO2MP_FRUSTUM_NEARFAR").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZN: f32 = 4.0;

    // Far from the world origin, looking along a diagonal, so a camera-relative matrix
    // would put every portal in the wrong place.
    fn far_view() -> (PreparedSceneView, Vec3, Vec3, Vec3) {
        let eye = Vec3::new(5200.0, -3100.0, 240.0);
        let lens = Transform::from_translation(eye)
            .looking_at(eye + Vec3::new(0.6, 0.8, 0.0), Vec3::Z);
        let view = prepare_scene_view(
            lens.to_matrix(),
            65f32.to_radians(),
            16.0 / 9.0,
            ZN,
            0.0,
            1920,
            1080,
            ZN,
            hud_t6::view_projection::R_ZNEAR_DEPTHHACK_DEFAULT,
        );
        let right = lens.rotation * Vec3::X;
        let up = lens.rotation * Vec3::Y;
        (view, eye, right, up)
    }

    fn project(vp: &[f32; 16], p: Vec3) -> [f32; 4] {
        core::array::from_fn(|c| p.x * vp[c] + p.y * vp[4 + c] + p.z * vp[8 + c] + vp[12 + c])
    }

    #[test]
    fn scene_matrix_is_absolute_with_reversed_depth() {
        let (view, eye, right, up) = far_view();
        let tan_y = (view.fov * 0.5).tan();
        let fwd = view.forward;
        let axis = [fwd.to_array(), (-right).to_array(), up.to_array()];
        let (vp, inv) =
            scene_view_projection(eye.to_array(), axis, tan_y * view.aspect, tan_y, ZN).unwrap();
        for d in [ZN, 64.0, 4096.0] {
            let c = project(&vp, eye + fwd * d);
            assert!(c[0].abs() < 1e-3 && c[1].abs() < 1e-3, "{c:?}");
            assert!((c[3] - d).abs() < d * 1e-4, "w {c:?}");
            assert!((c[2] / c[3] - ZN / d).abs() < 1e-4, "reversed z {c:?}");
        }
        // The screen centre at clip z 1 unprojects onto the near plane.
        let h: [f32; 4] = core::array::from_fn(|c| inv[8 + c] + inv[12 + c]);
        let near_mid = Vec3::new(h[0], h[1], h[2]) / h[3];
        let want = eye + fwd * ZN;
        assert!(near_mid.distance(want) < 0.05, "{near_mid} vs {want}");
    }

    #[test]
    fn bevel_planes_keep_the_view_through_a_far_portal() {
        let (view, eye, right, up) = far_view();
        let bevels = view.portal_bevels.expect("bevels");
        let centre = eye + view.forward * 500.0;
        let winding: Vec<[f32; 3]> = (0..12)
            .map(|i| {
                let a = i as f32 * core::f32::consts::TAU / 12.0;
                (centre + right * (100.0 * a.cos()) + up * (100.0 * a.sin())).to_array()
            })
            .collect();
        let mut planes = [[0.0f32; 4]; 16];
        let n = dpvs_t6::portal_clip_planes(
            &winding,
            eye.to_array(),
            None,
            None,
            Some(&bevels),
            &mut planes,
        );
        assert_eq!(n, 4);
        let dist = |pl: &[f32; 4], p: Vec3| pl[0] * p.x + pl[1] * p.y + pl[2] * p.z + pl[3];
        let behind = eye + view.forward * 1000.0;
        for pl in &planes[..n] {
            assert!(dist(pl, behind) > 0.0, "centre culled by {pl:?}");
        }
        for off in [right * 600.0, -right * 600.0, up * 600.0, -up * 600.0] {
            let p = behind + off;
            assert!(planes[..n].iter().any(|pl| dist(pl, p) < 0.0), "{off} not culled");
        }
    }
}
