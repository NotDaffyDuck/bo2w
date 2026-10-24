//! BO2 2D/3D view-mode switching and begin-view (scratch/hud-facts/set_2d.md). Matrices are row-major,
//! row-vector convention, as BO2 stores them.

pub const GFX_VIEW_MODE_NONE: i32 = 0;
pub const GFX_VIEW_MODE_3D: i32 = 1;
pub const GFX_VIEW_MODE_2D: i32 = 2;
pub const GFX_VIEW_MODE_IDENTITY: i32 = 3;

/// GfxSceneDef: time, floatTime, viewOffset xyz.
pub const GFX_SCENE_DEF_DWORDS: usize = 5;
pub const GFX_SCENE_DEF_TIME: usize = 0;
pub const GFX_SCENE_DEF_FLOAT_TIME: usize = 1;
pub const GFX_SCENE_DEF_VIEW_OFFSET: usize = 2;

/// GfxViewParms: 0x150 bytes.
pub const GFX_VIEWPARMS_DWORDS: usize = 0x54;
pub const GFX_VIEWPARMS_SIZE: usize = GFX_VIEWPARMS_DWORDS * 4;
pub const GFX_VIEWPARMS_VIEW: usize = 0x00;
pub const GFX_VIEWPARMS_PROJECTION: usize = 0x40;
pub const GFX_VIEWPARMS_VIEW_PROJECTION: usize = 0x80;
pub const GFX_VIEWPARMS_INV_VP: usize = 0xC0;
pub const GFX_VIEWPARMS_ORIGIN: usize = 0x100;
pub const GFX_VIEWPARMS_AXIS: usize = 0x110;
pub const GFX_VIEWPARMS_DEPTH_HACK_NEAR_CLIP: usize = 0x134;
pub const GFX_VIEWPARMS_Z_NEAR: usize = 0x138;
pub const GFX_VIEWPARMS_Z_FAR: usize = 0x13C;
pub const GFX_VIEWPARMS_BSP_CELL_INDEX: usize = 0x140;

/// Viewport behaviour byte: 1 selects the whole render target.
pub const GFX_VIEWPORT_FULL: u8 = 1;

/// Matrix version slots bumped on every 2D/3D switch (all but 6).
pub const SET_VIEW_MATRIX_VERSIONS: [usize; 7] = [0, 1, 2, 3, 4, 5, 7];
/// Code constant slot that follows the world matrix version.
pub const CONST_WORLD_MATRIX_VERSION: usize = 0xd3;
/// Code constant slot BO2 writes the game-time vec4 into.
pub const CONST_GAME_TIME: usize = 0x19;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxViewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The parts of BO2's GfxCmdBufSourceState that view switching reads and writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxCmdBufSourceState {
    pub view_mode: i32,
    pub viewport_is_dirty: bool,
    pub eye_offset: [f32; 4],
    pub viewport_behavior: u8,
    pub render_target_width: i32,
    pub render_target_height: i32,
    pub scene_viewport: GfxViewport,
    /// The active view parms.
    pub view_parms: [u32; GFX_VIEWPARMS_DWORDS],
    /// The pending 3D view (BO2 keeps a pointer; R_Set3D needs it set).
    pub view_parms_3d: Option<[u32; GFX_VIEWPARMS_DWORDS]>,
    pub scene_def: [u32; GFX_SCENE_DEF_DWORDS],
    pub skinned_placement_origin: [f32; 3],
    pub material_time: f32,
    pub destructible_burn_amount: f32,
    /// Matrix slot 0.
    pub world_matrix: [f32; 16],
    pub matrix_versions: [u16; 8],
    /// constVersions[0xd3].
    pub world_matrix_const_version: u16,
}

impl Default for GfxCmdBufSourceState {
    fn default() -> Self {
        Self {
            view_mode: GFX_VIEW_MODE_NONE,
            viewport_is_dirty: false,
            eye_offset: [0.0; 4],
            viewport_behavior: GFX_VIEWPORT_FULL,
            render_target_width: 0,
            render_target_height: 0,
            scene_viewport: GfxViewport::default(),
            view_parms: [0; GFX_VIEWPARMS_DWORDS],
            view_parms_3d: None,
            scene_def: [0; GFX_SCENE_DEF_DWORDS],
            skinned_placement_origin: [0.0; 3],
            material_time: 0.0,
            destructible_burn_amount: 0.0,
            world_matrix: IDENTITY,
            matrix_versions: [0; 8],
            world_matrix_const_version: 0,
        }
    }
}

/// R_GetViewport: the whole render target for behaviour 1, else the scene viewport.
#[must_use]
pub fn get_viewport(source: &GfxCmdBufSourceState) -> GfxViewport {
    let vp = if source.viewport_behavior == GFX_VIEWPORT_FULL {
        GfxViewport { x: 0, y: 0, width: source.render_target_width, height: source.render_target_height }
    } else {
        source.scene_viewport
    };
    debug_assert!(vp.width > 0 && vp.height > 0, "viewport {vp:?} is empty");
    vp
}

/// R_SetViewportStruct: store the scene viewport and force the next mode switch.
pub fn set_viewport_struct(source: &mut GfxCmdBufSourceState, viewport: GfxViewport) {
    debug_assert!(viewport.width > 0 && viewport.height > 0);
    source.scene_viewport = viewport;
    source.view_mode = GFX_VIEW_MODE_NONE;
    source.viewport_is_dirty = true;
}

/// The 2D projection: [0] 2/w, [5] -2/h, [12] -1, [13] +1, [15] 1, no half-pixel term. BO2 asserts a
/// positive size; None here.
#[must_use]
pub fn cmd_buf_set_2d_projection(width: i32, height: i32) -> Option<[f32; 16]> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let mut m = [0.0f32; 16];
    m[0] = 2.0 / width as f32;
    m[5] = -2.0 / height as f32;
    m[12] = -1.0;
    m[13] = 1.0;
    m[15] = 1.0;
    Some(m)
}

/// Pixel to clip space with a 2D projection.
#[must_use]
pub fn set_2d_clip_xy(x: f32, y: f32, projection: &[f32; 16]) -> (f32, f32) {
    (x * projection[0] + projection[12], y * projection[5] + projection[13])
}

/// The four elements a 2D shader needs: x scale, y scale, x bias, y bias.
#[must_use]
pub fn set_2d_clip_coeffs(projection: &[f32; 16]) -> [f32; 4] {
    [projection[0], projection[5], projection[12], projection[13]]
}

#[must_use]
pub fn gfx_viewparms_read_matrix(view_parms: &[u32; GFX_VIEWPARMS_DWORDS], byte: usize) -> [f32; 16] {
    core::array::from_fn(|i| f32::from_bits(view_parms[byte / 4 + i]))
}

pub fn gfx_viewparms_write_matrix(view_parms: &mut [u32; GFX_VIEWPARMS_DWORDS], byte: usize, m: &[f32; 16]) {
    for (i, v) in m.iter().enumerate() {
        view_parms[byte / 4 + i] = v.to_bits();
    }
}

#[must_use]
pub fn gfx_viewparms_origin(view_parms: &[u32; GFX_VIEWPARMS_DWORDS]) -> [f32; 4] {
    core::array::from_fn(|i| f32::from_bits(view_parms[GFX_VIEWPARMS_ORIGIN / 4 + i]))
}

pub fn gfx_viewparms_write_origin(view_parms: &mut [u32; GFX_VIEWPARMS_DWORDS], origin: [f32; 4]) {
    for (i, v) in origin.iter().enumerate() {
        view_parms[GFX_VIEWPARMS_ORIGIN / 4 + i] = v.to_bits();
    }
}

/// A scene def from its parts.
#[must_use]
pub fn gfx_scene_def(time: i32, float_time: f32, view_offset: [f32; 3]) -> [u32; GFX_SCENE_DEF_DWORDS] {
    [time as u32, float_time.to_bits(), view_offset[0].to_bits(), view_offset[1].to_bits(), view_offset[2].to_bits()]
}

fn bump_versions(source: &mut GfxCmdBufSourceState) {
    for i in SET_VIEW_MATRIX_VERSIONS {
        source.matrix_versions[i] = source.matrix_versions[i].wrapping_add(1);
    }
    source.world_matrix_const_version = source.matrix_versions[0];
}

/// R_CmdBufSet2D: identity view, 2D projection as projection and view-projection, zero inverse,
/// identity world (no eye offset in 2D).
pub fn cmd_buf_set_2d(source: &mut GfxCmdBufSourceState, viewport: GfxViewport) -> bool {
    let Some(proj) = cmd_buf_set_2d_projection(viewport.width, viewport.height) else {
        return false;
    };
    let vp = &mut source.view_parms;
    gfx_viewparms_write_matrix(vp, GFX_VIEWPARMS_VIEW, &IDENTITY);
    gfx_viewparms_write_matrix(vp, GFX_VIEWPARMS_PROJECTION, &proj);
    gfx_viewparms_write_matrix(vp, GFX_VIEWPARMS_VIEW_PROJECTION, &proj);
    gfx_viewparms_write_matrix(vp, GFX_VIEWPARMS_INV_VP, &[0.0; 16]);
    bump_versions(source);
    source.world_matrix = IDENTITY;
    true
}

/// R_Set2D: no-op when already 2D.
pub fn set_2d(source: &mut GfxCmdBufSourceState) -> bool {
    if source.view_mode == GFX_VIEW_MODE_2D {
        return false;
    }
    source.view_mode = GFX_VIEW_MODE_2D;
    source.viewport_is_dirty = true;
    source.eye_offset = [0.0, 0.0, 0.0, 1.0];
    let vp = get_viewport(source);
    cmd_buf_set_2d(source, vp)
}

/// R_CmdBufSet3D's world matrix: identity with the eye offset taken off the translation.
#[must_use]
pub fn cmd_buf_set_3d_world(eye_offset: [f32; 4]) -> [f32; 16] {
    let mut w = IDENTITY;
    w[12] -= eye_offset[0];
    w[13] -= eye_offset[1];
    w[14] -= eye_offset[2];
    w
}

/// R_Set3D: no-op when already 3D; needs the pending 3D view. The eye offset is the view origin only
/// when origin.w is not 0.
pub fn set_3d(source: &mut GfxCmdBufSourceState) -> bool {
    if source.view_mode == GFX_VIEW_MODE_3D {
        return false;
    }
    debug_assert!(source.view_parms_3d.is_some(), "R_Set3D without a 3D view");
    let Some(vp3d) = source.view_parms_3d else {
        return false;
    };
    source.view_mode = GFX_VIEW_MODE_3D;
    source.view_parms = vp3d;
    let o = gfx_viewparms_origin(&vp3d);
    source.eye_offset = if o[3] == 0.0 { [0.0, 0.0, 0.0, 1.0] } else { [o[0], o[1], o[2], 1.0] };
    bump_versions(source);
    source.world_matrix = cmd_buf_set_3d_world(source.eye_offset);
    true
}

/// R_BeginView. Returns floatTime for the game-time constant (CONST_GAME_TIME), which the caller
/// builds and writes.
pub fn begin_view(
    source: &mut GfxCmdBufSourceState,
    scene_def: &[u32; GFX_SCENE_DEF_DWORDS],
    view_parms_3d: &[u32; GFX_VIEWPARMS_DWORDS],
) -> f32 {
    source.scene_def = *scene_def;
    source.view_parms_3d = Some(*view_parms_3d);
    let o = GFX_SCENE_DEF_VIEW_OFFSET;
    source.skinned_placement_origin = core::array::from_fn(|i| f32::from_bits(scene_def[o + i]));
    source.view_mode = GFX_VIEW_MODE_NONE;
    set_3d(source);
    source.material_time = 0.0;
    source.destructible_burn_amount = 0.0;
    f32::from_bits(scene_def[GFX_SCENE_DEF_FLOAT_TIME])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_has_no_half_pixel() {
        let m = cmd_buf_set_2d_projection(640, 480).unwrap();
        assert_eq!(set_2d_clip_xy(0.0, 0.0, &m), (-1.0, 1.0));
        assert_eq!(set_2d_clip_xy(640.0, 480.0, &m), (1.0, -1.0));
        assert_eq!(m[14], 0.0);
        assert_eq!(m[10], 0.0);
        assert_eq!(set_2d_clip_coeffs(&m), [2.0 / 640.0, -2.0 / 480.0, -1.0, 1.0]);
        assert!(cmd_buf_set_2d_projection(0, 480).is_none());
    }

    #[test]
    fn set_2d_writes_parms_and_bumps_versions_once() {
        let mut s = GfxCmdBufSourceState { render_target_width: 1280, render_target_height: 720, ..Default::default() };
        s.world_matrix = [9.0; 16];
        assert!(set_2d(&mut s));
        assert!(!set_2d(&mut s));
        assert_eq!(s.eye_offset, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.world_matrix, IDENTITY);
        assert_eq!(gfx_viewparms_read_matrix(&s.view_parms, GFX_VIEWPARMS_VIEW), IDENTITY);
        assert_eq!(gfx_viewparms_read_matrix(&s.view_parms, GFX_VIEWPARMS_INV_VP), [0.0; 16]);
        assert_eq!(s.matrix_versions, [1, 1, 1, 1, 1, 1, 0, 1]);
        assert_eq!(s.world_matrix_const_version, 1);
    }

    #[test]
    fn begin_view_sets_3d_and_zeroes_times() {
        let mut vp = [0u32; GFX_VIEWPARMS_DWORDS];
        gfx_viewparms_write_origin(&mut vp, [10.0, 20.0, 30.0, 1.0]);
        let mut s = GfxCmdBufSourceState { view_mode: GFX_VIEW_MODE_3D, material_time: 5.0, ..Default::default() };
        let t = begin_view(&mut s, &gfx_scene_def(1000, 1.0, [1.0, 2.0, 3.0]), &vp);
        assert_eq!(t, 1.0);
        assert_eq!(s.view_mode, GFX_VIEW_MODE_3D);
        assert_eq!(s.skinned_placement_origin, [1.0, 2.0, 3.0]);
        assert_eq!(s.eye_offset, [10.0, 20.0, 30.0, 1.0]);
        assert_eq!((s.world_matrix[12], s.world_matrix[13], s.world_matrix[14]), (-10.0, -20.0, -30.0));
        assert_eq!(s.material_time, 0.0);
        // origin.w == 0: no eye offset.
        gfx_viewparms_write_origin(&mut vp, [10.0, 20.0, 30.0, 0.0]);
        begin_view(&mut s, &gfx_scene_def(0, 0.0, [0.0; 3]), &vp);
        assert_eq!(s.eye_offset, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.world_matrix, IDENTITY);
    }

    #[test]
    fn viewport_selection() {
        let mut s = GfxCmdBufSourceState { render_target_width: 800, render_target_height: 600, ..Default::default() };
        assert_eq!(get_viewport(&s), GfxViewport { x: 0, y: 0, width: 800, height: 600 });
        s.viewport_behavior = 0;
        set_viewport_struct(&mut s, GfxViewport { x: 5, y: 6, width: 7, height: 8 });
        assert!(s.viewport_is_dirty);
        assert_eq!(get_viewport(&s).width, 7);
    }
}
