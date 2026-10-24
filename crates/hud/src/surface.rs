use bevy::prelude::*;
use hud_t6::scrplace::{AppliedRect, PlacementInputs, ScreenPlacement};

/// PC displays have square pixels.
const SQUARE_PIXELS: f32 = 1.0;

/// The window as the HUD sees it: its pixel size and the Black Ops II screen
/// placement that maps virtual 640x480 coordinates onto it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Hud2dSurface {
    width: f32,
    height: f32,
    place: ScreenPlacement,
}

impl Default for Hud2dSurface {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

impl Hud2dSurface {
    pub fn new(width: f32, height: f32) -> Self {
        let size = [width.max(1.0), height.max(1.0)];
        // TODO(fact): the PC safeArea dvar defaults; the whole window until they are pinned.
        let inputs = PlacementInputs {
            display_size: size,
            safe_area: [1.0, 1.0],
            profile_safe_area: None,
            aspect_display_pixel: SQUARE_PIXELS,
            dual_play_enable: false,
            hud_splitscreen_scale: None,
        };
        Self {
            width,
            height,
            place: ScreenPlacement::setup_float_viewport(size[0], size[1], &inputs),
        }
    }

    /// False until the window has a real size.
    pub fn is_ready(&self) -> bool {
        self.width > 1.0 && self.height > 1.0
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    pub fn apply_rect(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        horz_align: i32,
        vert_align: i32,
    ) -> AppliedRect {
        let virtual_rect = AppliedRect { x, y, w, h };
        self.place
            .apply_rect(virtual_rect, horz_align, vert_align, false)
    }

    pub fn scale_virtual_to_real(&self) -> [f32; 2] {
        self.place.scale_virtual_to_real
    }

    /// The full-window placement (`ScrPlace_SetupFloatViewport`).
    pub fn placement(&self) -> &ScreenPlacement {
        &self.place
    }
}

/// Follows the primary window's size; without a window the surface is empty.
pub(crate) fn update_hud_surface(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut surface: ResMut<Hud2dSurface>,
) {
    *surface = match windows.single() {
        Ok(window) => Hud2dSurface::new(window.width(), window.height()),
        Err(_) => Hud2dSurface::new(0.0, 0.0),
    };
}
