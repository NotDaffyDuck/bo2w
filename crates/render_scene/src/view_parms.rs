//! The scene camera as the renderer and the visibility walk read it: matrices,
//! frustum, packed view parameters, and the dvars that bend them.

use bevy::prelude::*;
use hud_t6::set_2d::{
    GFX_VIEWPARMS_DWORDS, GFX_VIEWPARMS_INV_VP, GFX_VIEWPARMS_PROJECTION, GFX_VIEWPARMS_VIEW,
    GFX_VIEWPARMS_VIEW_PROJECTION, GfxViewport, gfx_viewparms_write_matrix,
    gfx_viewparms_write_origin,
};
use hud_t6::view_projection as vp;

use crate::frustum::{FrustumSpec, perspective_frustum_planes};

/// The shader-side view parameter block for the live camera. The inverse of the
/// view-projection is left zero when that matrix cannot be inverted.
#[must_use]
pub fn pack_live_view_parms(
    view_from_world: Mat4,
    clip_from_view: Mat4,
    clip_from_world: Mat4,
    view_origin: Vec3,
) -> [u32; GFX_VIEWPARMS_DWORDS] {
    let mut parms = [0u32; GFX_VIEWPARMS_DWORDS];
    for (slot, matrix) in [
        (GFX_VIEWPARMS_VIEW, view_from_world),
        (GFX_VIEWPARMS_PROJECTION, clip_from_view),
        (GFX_VIEWPARMS_VIEW_PROJECTION, clip_from_world),
    ] {
        gfx_viewparms_write_matrix(&mut parms, slot, &matrix.to_cols_array());
    }
    let det = clip_from_world.determinant();
    if det != 0.0 && det.is_finite() {
        gfx_viewparms_write_matrix(
            &mut parms,
            GFX_VIEWPARMS_INV_VP,
            &clip_from_world.inverse().to_cols_array(),
        );
    }
    gfx_viewparms_write_origin(&mut parms, view_origin.extend(1.0).to_array());
    parms
}

/// The projection the host renders with: infinite far plane, reversed depth.
#[must_use]
pub fn host_clip_from_view(fov: f32, aspect: f32, near: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fov, aspect, near)
}

/// The camera for this frame, worked out once and read by every pass.
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
            parms: [0; GFX_VIEWPARMS_DWORDS],
            portal_bevels: None,
            depth_hack_near: vp::R_ZNEAR_DEPTHHACK_DEFAULT,
            scene_viewport: GfxViewport::default(),
        }
    }
}

impl PreparedSceneView {
    #[must_use]
    pub fn tan_half_fov_y(&self) -> f32 {
        (self.fov * 0.5).tan()
    }
}

/// `r_znear`: the near plane distance.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RZnearDvar {
    pub value: f32,
}

impl Default for RZnearDvar {
    fn default() -> Self {
        Self { value: vp::R_ZNEAR_DEFAULT }
    }
}

/// `r_znear_depthhack`: the near plane for first-person models.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RZnearDepthhackDvar {
    pub value: f32,
}

impl Default for RZnearDepthhackDvar {
    fn default() -> Self {
        Self { value: vp::R_ZNEAR_DEPTHHACK_DEFAULT }
    }
}

/// `sm_enable`; `None` leaves the choice to the map.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SmEnableDvar {
    pub enabled: Option<bool>,
}

/// `sm_sunenable`; `None` leaves the choice to the map.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SmSunEnableDvar {
    pub enabled: Option<bool>,
}

/// The camera as it was when the visibility view was locked.
#[derive(Clone, Debug)]
pub struct LockPvsView {
    pub eye: Vec3,
    pub forward: Vec3,
    pub frustum_planes: Vec<[f32; 4]>,
    pub portal_bevels: Option<dpvs_t6::Bevels>,
    pub fov: f32,
    pub aspect: f32,
}

/// `r_lockpvs`: while enabled, the visibility walk keeps using the camera it
/// had when the lock was taken, so the player can fly around what it culled.
#[derive(Resource, Clone, Debug, Default)]
pub struct RLockPvs {
    pub enabled: bool,
    /// Take a fresh lock on the next frame.
    pub recapture: bool,
    pub frozen: Option<LockPvsView>,
}

impl RLockPvs {
    /// Called once the live camera is known: drops the lock when disabled, and
    /// (re)takes it from `live` when asked or when none is held yet.
    pub fn apply_after_stamp(&mut self, live: &PreparedSceneView) {
        if !self.enabled {
            self.frozen = None;
            self.recapture = false;
        } else if self.recapture || self.frozen.is_none() {
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

    /// The locked view when locking is on and one is held, else `None`.
    fn locked(&self) -> Option<&LockPvsView> {
        self.frozen.as_ref().filter(|_| self.enabled)
    }

    #[must_use]
    pub fn dpvs_eye(&self, live: &PreparedSceneView) -> Vec3 {
        self.locked().map_or(live.eye, |f| f.eye)
    }

    #[must_use]
    pub fn dpvs_forward(&self, live: &PreparedSceneView) -> Vec3 {
        self.locked().map_or(live.forward, |f| f.forward)
    }

    #[must_use]
    pub fn dpvs_frustum<'a>(&'a self, live: &'a PreparedSceneView) -> &'a [[f32; 4]] {
        self.locked().map_or(&live.frustum_planes, |f| &f.frustum_planes)
    }

    #[must_use]
    pub fn dpvs_portal_bevels(&self, live: &PreparedSceneView) -> Option<dpvs_t6::Bevels> {
        self.locked().map_or(live.portal_bevels, |f| f.portal_bevels)
    }

    #[must_use]
    pub fn dpvs_fov_aspect(&self, live: &PreparedSceneView) -> (f32, f32) {
        self.locked().map_or((live.fov, live.aspect), |f| (f.fov, f.aspect))
    }
}

/// The matrices the portal walk clips with: the absolute view times the
/// reversed-depth infinite projection, and that product's inverse (`None` when
/// it is singular).
#[must_use]
pub fn scene_view_projection(
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    tan_half_x: f32,
    tan_half_y: f32,
    z_near: f32,
) -> Option<([f32; 16], [f32; 16])> {
    let view_proj = vp::matrix_multiply44(
        &vp::matrix_for_viewer(origin, axis),
        &vp::infinite_perspective_matrix(tan_half_x, tan_half_y, z_near),
    );
    vp::matrix_inverse44(&view_proj).map(|inverse| (view_proj, inverse))
}

/// A child transform's world matrix under its parent.
#[must_use]
pub fn lens_world_from_parent_child(parent: Transform, child: Transform) -> Mat4 {
    parent.to_matrix() * child.to_matrix()
}

/// Works out the whole camera from the lens's world matrix.
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
    let forward = world_from_local.transform_vector3(Vec3::NEG_Z);
    let right = world_from_local.transform_vector3(Vec3::X);
    let up = world_from_local.transform_vector3(Vec3::Y);
    let view_from_world = world_from_local.inverse();
    let clip_from_view = host_clip_from_view(fov, aspect, near);
    let clip_from_world = clip_from_view * view_from_world;

    let mut frustum_planes =
        perspective_frustum_planes(FrustumSpec { eye, forward, right, up, fov_y_rad: fov, aspect, near, far })
            .to_vec();
    if env_flag("BO2MP_NO_FRUSTUM") {
        frustum_planes.clear();
    } else if env_flag("BO2MP_FRUSTUM_NEARFAR") {
        frustum_planes.truncate(2);
    }

    let tan_half_y = (fov * 0.5).tan();
    // BO2's view axis is forward, left, up: the viewer matrix takes clip x from -axis[1].
    let axis = [forward.to_array(), (-right).to_array(), up.to_array()];
    let portal_bevels =
        scene_view_projection(eye.to_array(), axis, tan_half_y * aspect, tan_half_y, dpvs_z_near)
            .map(|(view_proj, inverse)| dpvs_t6::Bevels::from_row_vector(&view_proj, &inverse));

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
        parms: pack_live_view_parms(view_from_world, clip_from_view, clip_from_world, eye),
        portal_bevels,
        depth_hack_near,
        scene_viewport: GfxViewport { x: 0, y: 0, width: rt_w, height: rt_h },
    }
}

fn env_flag(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const Z_NEAR: f32 = 4.0;

    /// A view far from the world origin, looking along a diagonal, so a
    /// camera-relative matrix would put every portal in the wrong place.
    /// Returns the view and the lens's eye, right and up.
    fn far_view() -> (PreparedSceneView, Vec3, Vec3, Vec3) {
        let eye = Vec3::new(5200.0, -3100.0, 240.0);
        let lens = Transform::from_translation(eye).looking_at(eye + Vec3::new(0.6, 0.8, 0.0), Vec3::Z);
        let view = prepare_scene_view(
            lens.to_matrix(),
            65f32.to_radians(),
            16.0 / 9.0,
            Z_NEAR,
            0.0,
            1920,
            1080,
            Z_NEAR,
            vp::R_ZNEAR_DEPTHHACK_DEFAULT,
        );
        (view, eye, lens.rotation * Vec3::X, lens.rotation * Vec3::Y)
    }

    /// `p` through a row-vector 4x4 matrix.
    fn project(m: &[f32; 16], p: Vec3) -> [f32; 4] {
        core::array::from_fn(|c| p.x * m[c] + p.y * m[4 + c] + p.z * m[8 + c] + m[12 + c])
    }

    #[test]
    fn scene_matrix_is_absolute_with_reversed_depth() {
        let (view, eye, right, up) = far_view();
        let tan_y = view.tan_half_fov_y();
        let axis = [view.forward.to_array(), (-right).to_array(), up.to_array()];
        let (view_proj, inverse) =
            scene_view_projection(eye.to_array(), axis, tan_y * view.aspect, tan_y, Z_NEAR).unwrap();
        for d in [Z_NEAR, 64.0, 4096.0] {
            let c = project(&view_proj, eye + view.forward * d);
            assert!(c[0].abs() < 1e-3 && c[1].abs() < 1e-3, "{c:?}");
            assert!((c[3] - d).abs() < d * 1e-4, "w {c:?}");
            assert!((c[2] / c[3] - Z_NEAR / d).abs() < 1e-4, "reversed z {c:?}");
        }
        // The screen centre at clip z = 1 unprojects onto the near plane.
        let h: [f32; 4] = core::array::from_fn(|c| inverse[8 + c] + inverse[12 + c]);
        let on_near = Vec3::new(h[0], h[1], h[2]) / h[3];
        assert!(on_near.distance(eye + view.forward * Z_NEAR) < 0.05, "{on_near}");
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
        let n = dpvs_t6::portal_clip_planes(&winding, eye.to_array(), None, None, Some(&bevels), &mut planes);
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

    #[test]
    fn a_prepared_view_is_ready_and_fills_the_render_target() {
        let (view, eye, _, _) = far_view();
        assert!(view.ready);
        assert_eq!(view.eye, eye);
        assert_eq!(view.scene_viewport, GfxViewport { x: 0, y: 0, width: 1920, height: 1080 });
        assert!(!PreparedSceneView::default().ready);
    }

    #[test]
    fn lens_matrix_composes_parent_then_child() {
        let parent = Transform::from_xyz(10.0, 0.0, 0.0);
        let child = Transform::from_xyz(0.0, 5.0, 0.0);
        let world = lens_world_from_parent_child(parent, child);
        assert_eq!(world.transform_point3(Vec3::ZERO), Vec3::new(10.0, 5.0, 0.0));
    }

    #[test]
    fn a_singular_matrix_leaves_the_inverse_block_zero() {
        let live = pack_live_view_parms(Mat4::IDENTITY, Mat4::IDENTITY, Mat4::ZERO, Vec3::ZERO);
        let ok = pack_live_view_parms(Mat4::IDENTITY, Mat4::IDENTITY, Mat4::IDENTITY, Vec3::ZERO);
        assert_ne!(live, ok);
        assert_eq!(live[GFX_VIEWPARMS_INV_VP / 4..GFX_VIEWPARMS_INV_VP / 4 + 16], [0; 16]);
    }

    #[test]
    fn the_pvs_lock_freezes_the_view_until_recaptured() {
        let (first, ..) = far_view();
        let mut moved = first.clone();
        moved.eye += Vec3::X * 100.0;
        let mut lock = RLockPvs::default();

        lock.apply_after_stamp(&first);
        assert_eq!(lock.dpvs_eye(&moved), moved.eye, "disabled lock follows the live view");

        lock.enabled = true;
        lock.apply_after_stamp(&first);
        lock.apply_after_stamp(&moved);
        assert_eq!(lock.dpvs_eye(&moved), first.eye, "held lock ignores the live view");

        lock.recapture = true;
        lock.apply_after_stamp(&moved);
        assert_eq!(lock.dpvs_eye(&first), moved.eye);
        assert!(!lock.recapture);

        lock.enabled = false;
        lock.apply_after_stamp(&first);
        assert!(lock.frozen.is_none());
    }
}
