//! The scene camera as the renderer and the visibility walk read it: joint_transforms,
//! frustum, packed view parameters, and the dvars that bend them.

use bevy::prelude::*;
use hud_t6::set_2d;
use hud_t6::set_2d::{
    CAMERA_BLOCK_WORDS, CAMERA_BLOCK_UNPROJECT, CAMERA_BLOCK_LENS, CAMERA_BLOCK_EYE,
    CAMERA_BLOCK_EYE_LENS, VisualViewport,
};
use hud_t6::view_projection;

use crate::frustum::FrustumShape;
use crate::frustum::view_cone_planes;

/// True when `m` has an inverse worth uploading: its determinant is a finite,
/// non-zero number (normal or subnormal).
fn has_usable_inverse(m: &Mat4) -> bool {
    matches!(m.determinant().classify(), std::num::FpCategory::Normal | std::num::FpCategory::Subnormal)
}

/// The shader-side view parameter block for the live camera. The inverse of the
/// view-projection is left zero when that matrix cannot be inverted.
#[must_use]
pub fn encode_active_view_params(
    view: Mat4,
    lens: Mat4,
    view_lens: Mat4,
    eye_origin: Vec3,
) -> [u32; CAMERA_BLOCK_WORDS] {
    let inverse_block = has_usable_inverse(&view_lens).then(|| (CAMERA_BLOCK_UNPROJECT, view_lens.inverse()));
    let blocks = [(CAMERA_BLOCK_EYE, view), (CAMERA_BLOCK_LENS, lens), (CAMERA_BLOCK_EYE_LENS, view_lens)];
    let mut out = [0u32; CAMERA_BLOCK_WORDS];
    blocks
        .into_iter()
        .chain(inverse_block)
        .for_each(|(at, m)| set_2d::write_eye_matrix(&mut out, at, &m.to_cols_array()));
    let Vec3 { x, y, z } = eye_origin;
    set_2d::write_eye_origin(&mut out, [x, y, z, 1.0]);
    out
}

/// The projection the host renders with: infinite far plane, reversed depth.
#[must_use]
pub fn perspective_clip_matrix(fov_y: f32, ratio: f32, z_near: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fov_y, ratio, z_near)
}

/// Tangent of half of a full field-of-view angle.
fn tan_of_half(full_angle: f32) -> f32 {
    (full_angle / 2.0).tan()
}

/// The camera for this frame, worked out once and read by every pass.
#[derive(Resource, Clone, Debug)]
pub struct StagedStageEye {
    pub primed: bool,
    pub eye: Vec3,
    pub forward: Vec3,
    pub eye_from_map: Mat4,
    pub screen_from_eye: Mat4,
    pub screen_from_map: Mat4,
    pub view_volume_planes: Vec<[f32; 4]>,
    pub view_deg: f32,
    pub wide_by_tall: f32,
    pub near_cut: f32,
    pub far_cut: f32,
    pub target_pixel_width: i32,
    pub target_pixel_height: i32,
    pub inputs: [u32; CAMERA_BLOCK_WORDS],
    pub portal_edge_bevels: Option<dpvs_t6::Bevels>,
    pub weapon_depth_near: f32,
    /// BO2 has no sub-window: the scene viewport is the whole render target.
    pub stage_window_rect: VisualViewport,
}

impl Default for StagedStageEye {
    fn default() -> Self {
        Self::blank(false, 0, 0, view_projection::STOCK_HELD_GUN_FRONT_CLIP)
    }
}

impl StagedStageEye {
    /// A view sized to a `width` x `height` target with nothing worked out yet:
    /// identity joint_transforms, no lens, no culling planes, the viewport covering
    /// the target.
    fn blank(ready: bool, width: i32, height: i32, weapon_depth_near: f32) -> Self {
        let still = Mat4::IDENTITY;
        let unset = 0.0;
        Self {
            primed: ready, eye: Vec3::ZERO, forward: Vec3::NEG_Z,
            eye_from_map: still, screen_from_eye: still, screen_from_map: still,
            view_volume_planes: Vec::new(),
            view_deg: unset, wide_by_tall: unset, near_cut: unset, far_cut: unset,
            target_pixel_width: width,
            target_pixel_height: height,
            inputs: [0; CAMERA_BLOCK_WORDS],
            portal_edge_bevels: None,
            weapon_depth_near,
            stage_window_rect: VisualViewport { x: 0, y: 0, width, height },
        }
    }

    #[must_use]
    pub fn fov_half_tan_y(&self) -> f32 {
        tan_of_half(self.view_deg)
    }
}

/// A float dvar resource holding one `value`, defaulting to `$default`.
macro_rules! float_dvar {
    ($(#[$doc:meta])* $name:ident = $default:expr) => {
        $(#[$doc])*
        #[derive(Resource, Clone, Copy, Debug)]
        pub struct $name {
            pub value: f32,
        }

        impl Default for $name {
            fn default() -> Self {
                Self { value: $default }
            }
        }
    };
}

/// A tri-state toggle dvar resource; `enabled: None` leaves the choice to the map.
macro_rules! map_toggle_dvar {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Resource, Clone, Copy, Debug, Default)]
        pub struct $name {
            pub enabled: Option<bool>,
        }
    };
}

float_dvar!(
    /// `r_znear`: the near plane distance.
    NearClipSetting = view_projection::STOCK_FRONT_CLIP
);
float_dvar!(
    /// `r_znear_depthhack`: the near plane for first-person models.
    GunNearClipSetting = view_projection::STOCK_HELD_GUN_FRONT_CLIP
);
map_toggle_dvar!(
    /// `sm_enable`; `None` leaves the choice to the map.
    ShadowMapsSwitch
);
map_toggle_dvar!(
    /// `sm_sunenable`; `None` leaves the choice to the map.
    SunShadowSwitch
);

/// The camera as it was when the visibility view was locked.
#[derive(Clone, Debug)]
pub struct FrozenVisibilityView {
    pub eye: Vec3,
    pub ahead: Vec3,
    pub view_volume_planes: Vec<[f32; 4]>,
    pub portal_edge_bevels: Option<dpvs_t6::Bevels>,
    pub view_deg: f32,
    pub wide_by_tall: f32,
}

/// `r_lockpvs`: while enabled, the visibility walk keeps using the camera it
/// had when the lock was taken, so the player can fly around what it culled.
#[derive(Resource, Clone, Debug, Default)]
pub struct FrozenVisibilitySet {
    pub enabled: bool,
    /// Take a fresh lock on the next frame.
    pub capture_again: bool,
    pub frozen: Option<FrozenVisibilityView>,
}

impl FrozenVisibilityView {
    /// Copies the parts of `view` the visibility walk reads.
    fn snapshot(view: &StagedStageEye) -> Self {
        let StagedStageEye { eye, forward, ref view_volume_planes, portal_edge_bevels, view_deg: fov, wide_by_tall: aspect, .. } = *view;
        Self { eye, ahead: forward, view_volume_planes: view_volume_planes.clone(), portal_edge_bevels, view_deg: fov, wide_by_tall: aspect }
    }
}

impl FrozenVisibilitySet {
    /// Called once the live camera is known: drops the lock when disabled, and
    /// (re)takes it from `live` when asked or when none is held yet. A pending
    /// capture_again request is always consumed.
    pub fn refresh_from(&mut self, live: &StagedStageEye) {
        let wants_fresh = std::mem::take(&mut self.capture_again) || self.frozen.is_none();
        self.frozen = match (self.enabled, wants_fresh) {
            (false, _) => None,
            (true, true) => Some(FrozenVisibilityView::snapshot(live)),
            (true, false) => self.frozen.take(),
        };
    }

    /// Reads a value from the held lock when locking is on and one exists,
    /// otherwise from the live view.
    fn read<'a, T>(
        &'a self,
        live: &'a StagedStageEye,
        from_live: impl FnOnce(&'a StagedStageEye) -> T,
        from_lock: impl FnOnce(&'a FrozenVisibilityView) -> T,
    ) -> T {
        match &self.frozen {
            Some(lock) if self.enabled => from_lock(lock),
            _ => from_live(live),
        }
    }

    #[must_use]
    pub fn walk_eye(&self, live: &StagedStageEye) -> Vec3 {
        self.read(live, |v| v.eye, |l| l.eye)
    }

    #[must_use]
    pub fn walk_forward(&self, live: &StagedStageEye) -> Vec3 {
        self.read(live, |v| v.forward, |l| l.ahead)
    }

    #[must_use]
    pub fn walk_planes<'a>(&'a self, live: &'a StagedStageEye) -> &'a [[f32; 4]] {
        self.read(live, |v| v.view_volume_planes.as_slice(), |l| l.view_volume_planes.as_slice())
    }

    #[must_use]
    pub fn walk_bevels(&self, live: &StagedStageEye) -> Option<dpvs_t6::Bevels> {
        self.read(live, |v| v.portal_edge_bevels, |l| l.portal_edge_bevels)
    }

    #[must_use]
    pub fn walk_lens(&self, live: &StagedStageEye) -> (f32, f32) {
        self.read(live, |v| (v.view_deg, v.wide_by_tall), |l| (l.view_deg, l.wide_by_tall))
    }
}

/// Older spelling SmEnableDvar still calls; the rename sweep moves it over.
pub use self::ShadowMapsSwitch as SmEnableDvar;
/// Older spelling SmSunEnableDvar still calls; the rename sweep moves it over.
pub use self::SunShadowSwitch as SmSunEnableDvar;

/// The joint_transforms the portal walk clips with: the absolute view times the
/// reversed-depth infinite projection, and that product's inverse (`None` when
/// it is singular).
#[must_use]
pub fn scene_view_projection(
    at: [f32; 3],
    frame: [[f32; 3]; 3],
    horizontal_cone_slope: f32,
    vertical_cone_slope: f32,
    front_clip_depth: f32,
) -> Option<([f32; 16], [f32; 16])> {
    let viewer = view_projection::matrix_for_viewer(at, frame);
    let projection = view_projection::infinite_perspective_matrix(horizontal_cone_slope, vertical_cone_slope, front_clip_depth);
    let combined = view_projection::chain44(&[&viewer, &projection]);
    let mut combined_inverse = [0.0; 16];
    view_projection::invert44(&combined, &mut combined_inverse).then_some((combined, combined_inverse))
}

/// A child transform's world matrix under its parent.
#[must_use]
pub fn child_world_matrix_under_parent(outer: Transform, inner: Transform) -> Mat4 {
    outer.to_matrix() * inner.to_matrix()
}

/// The lens position and its forward, right and up directions in world space.
struct LensFrame {
    spot: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
}

impl LensFrame {
    fn of(world_from_local: &Mat4) -> Self {
        let [forward, right, up] =
            [Vec3::NEG_Z, Vec3::X, Vec3::Y].map(|local| world_from_local.transform_vector3(local));
        Self { spot: world_from_local.transform_point3(Vec3::ZERO), forward, right, up }
    }

    /// BO2's view axis is forward, left, up: the viewer matrix takes clip x from -axis[1].
    fn bo2_axis(&self) -> [[f32; 3]; 3] {
        let Self { forward, right, up, .. } = self;
        [*forward, -*right, *up].map(|axis| axis.to_array())
    }
}

/// How many frustum planes the debug env flags let through (all by default).
fn frustum_plane_budget() -> usize {
    match (env_flag("BO2MP_NO_FRUSTUM"), env_flag("BO2MP_FRUSTUM_NEARFAR")) {
        (true, _) => 0,
        (false, true) => 2,
        (false, false) => usize::MAX,
    }
}

/// Works out the whole camera from the lens's world matrix.
#[must_use]
pub fn prepare_stage_eye(
    lens_to_world: Mat4,
    vertical_fov: f32, width_per_height: f32, front: f32, back: f32,
    target_pixel_width: i32,
    target_pixel_height: i32,
    pvs_front: f32, gun_front: f32,
) -> StagedStageEye {
    let lens = LensFrame::of(&lens_to_world);
    let to_view = lens_to_world.inverse();
    let lens_clip = perspective_clip_matrix(vertical_fov, width_per_height, front);
    let world_clip = lens_clip * to_view;
    let mut view = StagedStageEye {
        eye: lens.spot,
        forward: lens.forward,
        view_deg: vertical_fov,
        wide_by_tall: width_per_height,
        near_cut: front,
        far_cut: back,
        eye_from_map: to_view,
        screen_from_eye: lens_clip,
        screen_from_map: world_clip,
        inputs: encode_active_view_params(to_view, lens_clip, world_clip, lens.spot),
        ..StagedStageEye::blank(true, target_pixel_width, target_pixel_height, gun_front)
    };

    let spec = FrustumShape {
        eye: lens.spot,
        forward: lens.forward,
        side_dir: lens.right,
        up_dir: lens.up,
        vertical_fov_radians: vertical_fov,
        wide_by_tall: width_per_height,
        near_cut: front,
        far_cut: back,
    };
    view.view_volume_planes = view_cone_planes(spec).into_iter().take(frustum_plane_budget()).collect();

    let ty = tan_of_half(vertical_fov);
    view.portal_edge_bevels = scene_view_projection(lens.spot.to_array(), lens.bo2_axis(), ty * width_per_height, ty, pvs_front)
        .map(|(m, m_inv)| dpvs_t6::Bevels::from_row_vector(&m, &m_inv));
    view
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
    fn far_view() -> (StagedStageEye, Vec3, Vec3, Vec3) {
        let eye = Vec3::new(5200.0, -3100.0, 240.0);
        let lens = Transform::from_translation(eye).looking_at(eye + Vec3::new(0.6, 0.8, 0.0), Vec3::Z);
        let view = prepare_stage_eye(
            lens.to_matrix(),
            65f32.to_radians(),
            16.0 / 9.0,
            Z_NEAR,
            0.0,
            1920,
            1080,
            Z_NEAR,
            view_projection::STOCK_HELD_GUN_FRONT_CLIP,
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
        let tan_y = view.fov_half_tan_y();
        let axis = [view.forward.to_array(), (-right).to_array(), up.to_array()];
        let (view_proj, inverse) =
            scene_view_projection(eye.to_array(), axis, tan_y * view.wide_by_tall, tan_y, Z_NEAR).unwrap();
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
        let bevels = view.portal_edge_bevels.expect("bevels");
        let centre = eye + view.forward * 500.0;
        let winding: Vec<[f32; 3]> = (0..12)
            .map(|i| {
                let a = i as f32 * core::f32::consts::TAU / 12.0;
                (centre + right * (100.0 * a.cos()) + up * (100.0 * a.sin())).to_array()
            })
            .collect();
        let mut cut = [[0.0; 4]; 16];
        let from = eye.to_array();
        let n = dpvs_t6::portal_frustum_planes(&winding, from, None, None, Some(&bevels), &mut cut);
        assert_eq!(n, 4);
        let dist = |pl: &[f32; 4], p: Vec3| pl[0] * p.x + pl[1] * p.y + pl[2] * p.z + pl[3];
        let behind = eye + view.forward * 1000.0;
        for pl in &cut[..n] {
            assert!(dist(pl, behind) > 0.0, "centre culled by {pl:?}");
        }
        for off in [right * 600.0, -right * 600.0, up * 600.0, -up * 600.0] {
            let p = behind + off;
            assert!(cut[..n].iter().any(|pl| dist(pl, p) < 0.0), "{off} not culled");
        }
    }

    #[test]
    fn a_prepared_view_is_ready_and_fills_the_render_target() {
        let (view, eye, _, _) = far_view();
        assert!(view.primed);
        assert_eq!(view.eye, eye);
        assert_eq!(view.stage_window_rect, VisualViewport { x: 0, y: 0, width: 1920, height: 1080 });
        assert!(!StagedStageEye::default().primed);
    }

    #[test]
    fn lens_matrix_composes_parent_then_child() {
        let parent = Transform::from_xyz(10.0, 0.0, 0.0);
        let child = Transform::from_xyz(0.0, 5.0, 0.0);
        let world = child_world_matrix_under_parent(parent, child);
        assert_eq!(world.transform_point3(Vec3::ZERO), Vec3::new(10.0, 5.0, 0.0));
    }

    #[test]
    fn a_singular_matrix_leaves_the_inverse_block_zero() {
        let one = Mat4::IDENTITY;
        let live = encode_active_view_params(one, one, Mat4::ZERO, Vec3::ZERO);
        let ok = encode_active_view_params(one, one, one, Vec3::ZERO);
        assert_ne!(live, ok);
        assert_eq!(live[CAMERA_BLOCK_UNPROJECT / 4..CAMERA_BLOCK_UNPROJECT / 4 + 16], [0; 16]);
    }

    #[test]
    fn the_pvs_lock_freezes_the_view_until_recaptured() {
        let (first, ..) = far_view();
        let mut moved = first.clone();
        moved.eye += Vec3::X * 100.0;
        let mut lock = FrozenVisibilitySet::default();

        lock.refresh_from(&first);
        assert_eq!(lock.walk_eye(&moved), moved.eye, "disabled lock follows the live view");

        let on = true;
        lock.enabled = on;
        lock.refresh_from(&first);
        lock.refresh_from(&moved);
        assert_eq!(lock.walk_eye(&moved), first.eye, "held lock ignores the live view");

        lock.capture_again = true;
        lock.refresh_from(&moved);
        assert_eq!(lock.walk_eye(&first), moved.eye);
        assert!(!lock.capture_again);

        lock.enabled = !on;
        lock.refresh_from(&first);
        assert!(lock.frozen.is_none());
    }
}
