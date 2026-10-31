//! BO2 2D/3D view-mode switching and begin-view (scratch/hud-facts/set_2d.md). Matrices are row-major,
//! row-vector convention, as BO2 stores them.

pub const VIEW_MODE_UNSET: i32 = 0;
pub const VIEW_MODE_SCENE: i32 = 1;
pub const VIEW_MODE_FLAT: i32 = 2;
pub const GFX_VIEW_MODE_IDENTITY: i32 = 3;

/// GfxSceneDef: time, floatTime, viewOffset xyz.
pub const SCENE_DEF_WORDS: usize = 5;
pub const GFX_SCENE_DEF_TIME: usize = 0;
pub const GFX_SCENE_DEF_FLOAT_TIME: usize = 1;
pub const GFX_SCENE_DEF_VIEW_OFFSET: usize = 2;

/// GfxViewParms: 0x150 bytes.
pub const CAMERA_BLOCK_WORDS: usize = 0x54;
pub const VIEWPARMS_BYTES: usize = CAMERA_BLOCK_WORDS * 4;
pub const CAMERA_BLOCK_EYE: usize = 0x00;
pub const CAMERA_BLOCK_LENS: usize = 0x40;
pub const CAMERA_BLOCK_EYE_LENS: usize = 0x80;
pub const CAMERA_BLOCK_UNPROJECT: usize = 0xC0;
pub const VIEWPARMS_ORIGIN_OFS: usize = 0x100;
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
pub struct VisualViewport {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The parts of BO2's GfxCmdBufSourceState that view switching reads and writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxCmdBufSourceState {
    pub active_mode: i32,
    pub viewport_is_dirty: bool,
    pub gaze_lift: [f32; 4],
    pub viewport_behavior: u8,
    pub canvas_columns: i32,
    pub canvas_rows: i32,
    pub stage_window_rect: VisualViewport,
    /// The active view parms.
    pub eye_parms: [u32; CAMERA_BLOCK_WORDS],
    /// The pending 3D view (BO2 keeps a pointer; R_Set3D needs it set).
    pub view_parms_3d: Option<[u32; CAMERA_BLOCK_WORDS]>,
    pub stage_setup: [u32; SCENE_DEF_WORDS],
    pub skin_origin: [f32; 3],
    pub surface_clock: f32,
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
            active_mode: VIEW_MODE_UNSET,
            viewport_is_dirty: false,
            gaze_lift: [0.0; 4],
            viewport_behavior: GFX_VIEWPORT_FULL,
            canvas_columns: 0,
            canvas_rows: 0,
            stage_window_rect: VisualViewport::default(),
            eye_parms: [0; CAMERA_BLOCK_WORDS],
            view_parms_3d: None,
            stage_setup: [0; SCENE_DEF_WORDS],
            skin_origin: [0.0; 3],
            surface_clock: 0.0,
            destructible_burn_amount: 0.0,
            world_matrix: IDENTITY,
            matrix_versions: [0; 8],
            world_matrix_const_version: 0,
        }
    }
}

/// R_GetViewport: the whole render target for behaviour 1, else the scene viewport.
#[must_use]
pub fn get_viewport(source: &GfxCmdBufSourceState) -> VisualViewport {
    let vp = if source.viewport_behavior == GFX_VIEWPORT_FULL {
        VisualViewport { x: 0, y: 0, width: source.canvas_columns, height: source.canvas_rows }
    } else {
        source.stage_window_rect
    };
    debug_assert!(vp.width > 0 && vp.height > 0, "viewport {vp:?} is empty");
    vp
}

/// R_SetViewportStruct: store the scene viewport and force the next mode switch.
pub fn set_viewport_struct(source: &mut GfxCmdBufSourceState, viewport: VisualViewport) {
    debug_assert!(viewport.width > 0 && viewport.height > 0);
    source.stage_window_rect = viewport;
    source.active_mode = VIEW_MODE_UNSET;
    source.viewport_is_dirty = true;
}

/// The 2D projection: [0] 2/w, [5] -2/h, [12] -1, [13] +1, [15] 1, no half-pixel term. BO2 asserts a
/// positive size; None here.
#[must_use]
pub fn cmd_buf_set_2d_projection(across: i32, tall: i32) -> Option<[f32; 16]> {
    if across <= 0 || tall <= 0 {
        return None;
    }
    let mut m = [0.0f32; 16];
    m[0] = 2.0 / across as f32;
    m[5] = -2.0 / tall as f32;
    m[12] = -1.0;
    m[13] = 1.0;
    m[15] = 1.0;
    Some(m)
}

/// A pixel position in clip space under a 2D projection.
#[must_use]
pub fn clip_from_screen(lens: &[f32; 16], at: [f32; 2]) -> [f32; 2] {
    let [x, y] = at;
    [x * lens[0] + lens[12], y * lens[5] + lens[13]]
}

/// What a 2D shader needs, as two pairs: the (x, y) scale, then the (x, y) bias.
#[must_use]
pub fn set_2d_clip_coeffs(lens: &[f32; 16]) -> [[f32; 2]; 2] {
    [[lens[0], lens[5]], [lens[12], lens[13]]]
}

#[must_use]
pub fn read_eye_matrix(eye_parms: &[u32; CAMERA_BLOCK_WORDS], byte: usize) -> [f32; 16] {
    core::array::from_fn(|i| f32::from_bits(eye_parms[byte / 4 + i]))
}

pub fn write_eye_matrix(eye_parms: &mut [u32; CAMERA_BLOCK_WORDS], byte: usize, m: &[f32; 16]) {
    for (i, v) in m.iter().enumerate() {
        eye_parms[byte / 4 + i] = v.to_bits();
    }
}

#[must_use]
pub fn gfx_viewparms_origin(eye_parms: &[u32; CAMERA_BLOCK_WORDS]) -> [f32; 4] {
    core::array::from_fn(|i| f32::from_bits(eye_parms[VIEWPARMS_ORIGIN_OFS / 4 + i]))
}

pub fn write_eye_origin(eye_parms: &mut [u32; CAMERA_BLOCK_WORDS], spot: [f32; 4]) {
    for (i, v) in spot.iter().enumerate() {
        eye_parms[VIEWPARMS_ORIGIN_OFS / 4 + i] = v.to_bits();
    }
}

/// A scene def from its parts.
#[must_use]
pub fn gfx_scene_def(time: i32, time_float: f32, eye_displacement: [f32; 3]) -> [u32; SCENE_DEF_WORDS] {
    [time as u32, time_float.to_bits(), eye_displacement[0].to_bits(), eye_displacement[1].to_bits(), eye_displacement[2].to_bits()]
}

fn bump_versions(source: &mut GfxCmdBufSourceState) {
    for i in SET_VIEW_MATRIX_VERSIONS {
        source.matrix_versions[i] = source.matrix_versions[i].wrapping_add(1);
    }
    source.world_matrix_const_version = source.matrix_versions[0];
}

/// R_CmdBufSet2D: identity view, 2D projection as projection and view-projection, zero inverse,
/// identity world (no eye offset in 2D).
pub fn cmd_buf_set_2d(source: &mut GfxCmdBufSourceState, viewport: VisualViewport) -> bool {
    let Some(proj) = cmd_buf_set_2d_projection(viewport.width, viewport.height) else {
        return false;
    };
    let vp = &mut source.eye_parms;
    write_eye_matrix(vp, CAMERA_BLOCK_EYE, &IDENTITY);
    write_eye_matrix(vp, CAMERA_BLOCK_LENS, &proj);
    write_eye_matrix(vp, CAMERA_BLOCK_EYE_LENS, &proj);
    write_eye_matrix(vp, CAMERA_BLOCK_UNPROJECT, &[0.0; 16]);
    bump_versions(source);
    source.world_matrix = IDENTITY;
    true
}

/// R_Set2D: no-op when already 2D.
pub fn set_2d(source: &mut GfxCmdBufSourceState) -> bool {
    if source.active_mode == VIEW_MODE_FLAT {
        return false;
    }
    source.active_mode = VIEW_MODE_FLAT;
    source.viewport_is_dirty = true;
    source.gaze_lift = [0.0, 0.0, 0.0, 1.0];
    let vp = get_viewport(source);
    cmd_buf_set_2d(source, vp)
}

/// R_CmdBufSet3D's world matrix: identity with the eye offset taken off the translation.
#[must_use]
pub fn cmd_buf_set_3d_world(gaze_lift: [f32; 4]) -> [f32; 16] {
    let mut w = IDENTITY;
    w[12] -= gaze_lift[0];
    w[13] -= gaze_lift[1];
    w[14] -= gaze_lift[2];
    w
}

/// R_Set3D: no-op when already 3D; needs the pending 3D view. The eye offset is the view origin only
/// when origin.w is not 0.
pub fn set_3d(source: &mut GfxCmdBufSourceState) -> bool {
    if source.active_mode == VIEW_MODE_SCENE {
        return false;
    }
    debug_assert!(source.view_parms_3d.is_some(), "R_Set3D without a 3D view");
    let Some(vp3d) = source.view_parms_3d else {
        return false;
    };
    source.active_mode = VIEW_MODE_SCENE;
    source.eye_parms = vp3d;
    let o = gfx_viewparms_origin(&vp3d);
    let at = if o[3] == 0.0 { [0.0; 3] } else { [o[0], o[1], o[2]] };
    source.gaze_lift = [at[0], at[1], at[2], 1.0];
    bump_versions(source);
    source.world_matrix = cmd_buf_set_3d_world(source.gaze_lift);
    true
}

/// R_BeginView. Returns floatTime for the game-time constant (CONST_GAME_TIME), which the caller
/// builds and writes.
pub fn begin_view(
    source: &mut GfxCmdBufSourceState,
    stage_setup: &[u32; SCENE_DEF_WORDS],
    camera: &[u32; CAMERA_BLOCK_WORDS],
) -> f32 {
    source.stage_setup = *stage_setup;
    source.view_parms_3d = Some(*camera);
    let o = GFX_SCENE_DEF_VIEW_OFFSET;
    source.skin_origin = core::array::from_fn(|i| f32::from_bits(stage_setup[o + i]));
    source.active_mode = VIEW_MODE_UNSET;
    set_3d(source);
    source.surface_clock = 0.0;
    source.destructible_burn_amount = 0.0;
    f32::from_bits(stage_setup[GFX_SCENE_DEF_FLOAT_TIME])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_has_no_half_pixel() {
        let m = cmd_buf_set_2d_projection(640, 480).unwrap();
        assert_eq!(clip_from_screen(&m, [0.0, 0.0]), [-1.0, 1.0]);
        assert_eq!(clip_from_screen(&m, [640.0, 480.0]), [1.0, -1.0]);
        assert_eq!(m[14], 0.0);
        assert_eq!(m[10], 0.0);
        assert_eq!(set_2d_clip_coeffs(&m), [[2.0 / 640.0, -2.0 / 480.0], [-1.0, 1.0]]);
        assert!(cmd_buf_set_2d_projection(0, 480).is_none());
    }

    #[test]
    fn set_2d_writes_parms_and_bumps_versions_once() {
        let mut s = GfxCmdBufSourceState { canvas_columns: 1280, canvas_rows: 720, ..GfxCmdBufSourceState::default() };
        s.world_matrix = [9.0; 16];
        assert!(set_2d(&mut s));
        assert!(!set_2d(&mut s));
        assert_eq!(s.gaze_lift, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.world_matrix, IDENTITY);
        assert_eq!(read_eye_matrix(&s.eye_parms, CAMERA_BLOCK_EYE), IDENTITY);
        assert_eq!(read_eye_matrix(&s.eye_parms, CAMERA_BLOCK_UNPROJECT), [0.0; 16]);
        assert_eq!(s.matrix_versions, [1, 1, 1, 1, 1, 1, 0, 1]);
        assert_eq!(s.world_matrix_const_version, 1);
    }

    #[test]
    fn begin_view_sets_3d_and_zeroes_times() {
        let mut vp = [0u32; CAMERA_BLOCK_WORDS];
        write_eye_origin(&mut vp, [10.0, 20.0, 30.0, 1.0]);
        let mut s = GfxCmdBufSourceState { active_mode: VIEW_MODE_SCENE, surface_clock: 5.0, ..GfxCmdBufSourceState::default() };
        let t = begin_view(&mut s, &gfx_scene_def(1000, 1.0, [1.0, 2.0, 3.0]), &vp);
        assert_eq!(t, 1.0);
        assert_eq!(s.active_mode, VIEW_MODE_SCENE);
        assert_eq!(s.skin_origin, [1.0, 2.0, 3.0]);
        assert_eq!(s.gaze_lift, [10.0, 20.0, 30.0, 1.0]);
        assert_eq!((s.world_matrix[12], s.world_matrix[13], s.world_matrix[14]), (-10.0, -20.0, -30.0));
        assert_eq!(s.surface_clock, 0.0);
        // origin.w == 0: no eye offset.
        write_eye_origin(&mut vp, [10.0, 20.0, 30.0, 0.0]);
        begin_view(&mut s, &gfx_scene_def(0, 0.0, [0.0; 3]), &vp);
        assert_eq!(s.gaze_lift, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.world_matrix, IDENTITY);
    }

    #[test]
    fn viewport_selection() {
        let mut s = GfxCmdBufSourceState { canvas_columns: 800, canvas_rows: 600, ..GfxCmdBufSourceState::default() };
        assert_eq!(get_viewport(&s), VisualViewport { x: 0, y: 0, width: 800, height: 600 });
        s.viewport_behavior = 0;
        set_viewport_struct(&mut s, VisualViewport { x: 5, y: 6, width: 7, height: 8 });
        assert!(s.viewport_is_dirty);
        assert_eq!(get_viewport(&s).width, 7);
    }
}
