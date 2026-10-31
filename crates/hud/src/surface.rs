use bevy::prelude::*;
use hud_t6::scrplace::{PlacementInputs, DisplayAnchoring};
use bevy::window::PrimaryWindow;

/// The window as the HUD sees it: its pixel size and the screen placement
/// that maps virtual 640x480 coordinates onto it.
#[derive(Resource, Clone, Copy, Debug)]
pub struct FlatOverlayCanvas {
    window: [f32; 2],
    placement: DisplayAnchoring,
}

impl Default for FlatOverlayCanvas {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

/// The placement inputs for a PC window: square pixels, no splitscreen, and
/// the whole window as the safe area until the real dvar defaults are pinned.
fn pc_inputs(size: [f32; 2]) -> PlacementInputs {
    PlacementInputs {
        display_size: size,
        safe_area: [1.0, 1.0],
        profile_safe_area: None,
        aspect_display_pixel: 1.0,
        dual_play_enable: false,
        hud_splitscreen_scale: None,
    }
}

impl FlatOverlayCanvas {
    pub fn new(width: f32, height: f32) -> Self {
        // The placement never sees a window smaller than a pixel.
        let size = [width.max(1.0), height.max(1.0)];
        Self {
            window: [width, height],
            placement: DisplayAnchoring::setup_float_viewport(size[0], size[1], &pc_inputs(size)),
        }
    }

    /// False until the window is more than a pixel each way.
    pub fn has_size(&self) -> bool {
        self.window.iter().all(|&side| side > 1.0)
    }

    pub fn width(&self) -> f32 {
        self.window[0]
    }

    pub fn height(&self) -> f32 {
        self.window[1]
    }

    /// The full-window placement.
    pub fn placement(&self) -> &DisplayAnchoring {
        &self.placement
    }
}

/// Follows the primary window's size; without a window the surface is empty.
pub(crate) fn refresh_hud_surface(
    primary: Query<&Window, With<PrimaryWindow>>,
    mut surface: ResMut<FlatOverlayCanvas>,
) {
    let (width, height) = primary
        .single()
        .map_or((0.0, 0.0), |window| (window.width(), window.height()));
    *surface = FlatOverlayCanvas::new(width, height);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_window_is_not_ready() {
        let empty = FlatOverlayCanvas::default();
        assert!(!empty.has_size());
        assert_eq!((empty.width(), empty.height()), (0.0, 0.0));
    }

    #[test]
    fn one_pixel_is_still_not_ready() {
        assert!(!FlatOverlayCanvas::new(1.0, 720.0).has_size());
        assert!(!FlatOverlayCanvas::new(1280.0, 1.0).has_size());
        assert!(FlatOverlayCanvas::new(2.0, 2.0).has_size());
    }

    #[test]
    fn size_is_reported_as_given() {
        let surface = FlatOverlayCanvas::new(1280.0, 720.0);
        assert_eq!((surface.width(), surface.height()), (1280.0, 720.0));
    }

    #[test]
    fn placement_scales_the_virtual_canvas_by_height() {
        let scale = FlatOverlayCanvas::new(1280.0, 720.0).placement().layout_to_pixels;
        assert!((scale[1] - 720.0 / 480.0).abs() < 1e-4, "{scale:?}");
        let bigger = FlatOverlayCanvas::new(2560.0, 1440.0).placement().layout_to_pixels;
        assert!((bigger[1] - 2.0 * scale[1]).abs() < 1e-4);
    }

    #[test]
    fn a_zero_window_still_has_a_finite_placement() {
        let scale = FlatOverlayCanvas::new(0.0, 0.0).placement().layout_to_pixels;
        assert!(scale.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn surface_follows_the_window_size() {
        let mut app = App::new();
        app.init_resource::<FlatOverlayCanvas>()
            .add_systems(bevy::app::Update, refresh_hud_surface);
        app.update();
        assert!(!app.world().resource::<FlatOverlayCanvas>().has_size());
        app.world_mut().spawn((
            Window {
                resolution: (800, 600).into(),
                ..default()
            },
            bevy::window::PrimaryWindow,
        ));
        app.update();
        let surface = app.world().resource::<FlatOverlayCanvas>();
        assert!(surface.has_size());
        assert_eq!((surface.width(), surface.height()), (800.0, 600.0));
    }
}
