//! Putting rectangles authored on a 640x480 canvas onto a window of any size.

/// The canvas the authored HUD coordinates are measured on.
pub const CANVAS_HEIGHT: f32 = 480.0;
pub const CANVAS_WIDTH: f32 = 640.0;

/// Chrome is drawn at half the projection scale; its lower band gets a
/// further boost on top of that.
const CHROME_SHARE: f32 = 0.5;
const LOWER_CHROME_GAIN: f32 = 1.4;

/// Where along the window's width an authored x coordinate starts counting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorX {
    /// The left edge of the centred 4:3 area.
    InsetLeft,
    Left,
    Center,
    Right,
    Screen,
    Unscaled,
    Authored640,
    SafeCenter,
    ViewLeft,
    ViewCenter,
    ViewRight,
}

/// Where down the window's height an authored y coordinate starts counting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnchorY {
    /// The top edge of the centred 4:3 area.
    InsetTop,
    Top,
    Center,
    Bottom,
    Screen,
    Unscaled,
    Authored480,
    SafeCenter,
    ViewTop,
    ViewCenter,
    ViewBottom,
}

/// Which scale an element follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnitClass {
    Chrome,
    LowChrome,
    Projection,
    FillWindow,
}

/// The window the HUD is laid out on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowScale {
    width: f32,
    height: f32,
}

/// A rectangle in window pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl WindowScale {
    /// A window size, never smaller than one pixel each way.
    pub fn for_window(win_w: f32, win_h: f32) -> Self {
        let at_least_one = |side: f32| side.max(1.0);
        Self {
            width: at_least_one(win_w),
            height: at_least_one(win_h),
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Window pixels for one authored unit, by class.
    pub fn unit_px(&self, class: UnitClass) -> f32 {
        let full = self.height / CANVAS_HEIGHT;
        match class {
            UnitClass::Chrome => full * CHROME_SHARE,
            UnitClass::LowChrome => full * CHROME_SHARE * LOWER_CHROME_GAIN,
            UnitClass::Projection | UnitClass::FillWindow => full,
        }
    }

    /// Size of the largest centred 4:3 area that fits in the window.
    fn inset_area(&self) -> (f32, f32) {
        let by_height = (self.height * (CANVAS_WIDTH / CANVAS_HEIGHT)).min(self.width);
        let by_width = (self.width * (CANVAS_HEIGHT / CANVAS_WIDTH)).min(self.height);
        (by_height, by_width)
    }

    /// Window x that the anchor counts from.
    pub fn origin_x(&self, anchor: AnchorX) -> f32 {
        match anchor {
            AnchorX::InsetLeft => (self.width - self.inset_area().0) / 2.0,
            AnchorX::Center | AnchorX::SafeCenter | AnchorX::ViewCenter => self.width / 2.0,
            AnchorX::Right | AnchorX::ViewRight => self.width,
            AnchorX::Left
            | AnchorX::Screen
            | AnchorX::Unscaled
            | AnchorX::ViewLeft
            | AnchorX::Authored640 => 0.0,
        }
    }

    /// Window y that the anchor counts from.
    pub fn origin_y(&self, anchor: AnchorY) -> f32 {
        match anchor {
            AnchorY::InsetTop => (self.height - self.inset_area().1) / 2.0,
            AnchorY::Center | AnchorY::SafeCenter | AnchorY::ViewCenter => self.height / 2.0,
            AnchorY::Bottom | AnchorY::ViewBottom => self.height,
            AnchorY::Top
            | AnchorY::Screen
            | AnchorY::Unscaled
            | AnchorY::ViewTop
            | AnchorY::Authored480 => 0.0,
        }
    }

    /// Where an authored rectangle lands on the window.
    pub fn place_rect(
        &self,
        class: UnitClass,
        across: AnchorX,
        down: AnchorY,
        authored: PxRect,
    ) -> PxRect {
        if class == UnitClass::FillWindow {
            return PxRect {
                x: 0.0,
                y: 0.0,
                w: self.width,
                h: self.height,
            };
        }
        let unit = self.unit_px(class);
        PxRect {
            x: self.origin_x(across) + authored.x * unit,
            y: self.origin_y(down) + authored.y * unit,
            w: authored.w * unit,
            h: authored.h * unit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: (f32, f32) = (1920.0, 1080.0);
    const SQUARE: (f32, f32) = (600.0, 600.0);

    fn scale((w, h): (f32, f32)) -> WindowScale {
        WindowScale::for_window(w, h)
    }

    #[test]
    fn degenerate_windows_are_one_pixel() {
        let s = WindowScale::for_window(0.0, -5.0);
        assert_eq!((s.width(), s.height()), (1.0, 1.0));
    }

    #[test]
    fn unit_px_follows_window_height() {
        let s = scale(WIDE);
        assert_eq!(s.unit_px(UnitClass::Projection), 2.25);
        assert_eq!(s.unit_px(UnitClass::FillWindow), 2.25);
        assert_eq!(s.unit_px(UnitClass::Chrome), 1.125);
        let low = s.unit_px(UnitClass::LowChrome);
        assert!((low - 1.125 * 1.4).abs() < 1e-6);
    }

    #[test]
    fn inset_origin_centres_the_four_by_three_area() {
        let s = scale(WIDE);
        assert_eq!(s.origin_x(AnchorX::InsetLeft), (1920.0 - 1440.0) / 2.0);
        assert_eq!(s.origin_y(AnchorY::InsetTop), 0.0);
        let tall = scale(SQUARE);
        assert_eq!(tall.origin_x(AnchorX::InsetLeft), 0.0);
        assert_eq!(tall.origin_y(AnchorY::InsetTop), (600.0 - 450.0) / 2.0);
    }

    #[test]
    fn anchors_pick_their_edge() {
        let s = scale(WIDE);
        assert_eq!(s.origin_x(AnchorX::Left), 0.0);
        assert_eq!(s.origin_x(AnchorX::Center), 960.0);
        assert_eq!(s.origin_x(AnchorX::ViewRight), 1920.0);
        assert_eq!(s.origin_y(AnchorY::Top), 0.0);
        assert_eq!(s.origin_y(AnchorY::SafeCenter), 540.0);
        assert_eq!(s.origin_y(AnchorY::Bottom), 1080.0);
    }

    #[test]
    fn fill_window_ignores_the_authored_rect() {
        let s = scale(WIDE);
        let authored = PxRect { x: 5.0, y: 6.0, w: 7.0, h: 8.0 };
        let placed = s.place_rect(UnitClass::FillWindow, AnchorX::Right, AnchorY::Bottom, authored);
        assert_eq!(placed, PxRect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
    }

    #[test]
    fn placing_scales_and_offsets_from_the_anchor() {
        let s = scale(WIDE);
        let authored = PxRect { x: 10.0, y: -4.0, w: 20.0, h: 8.0 };
        let placed = s.place_rect(UnitClass::Projection, AnchorX::Center, AnchorY::Bottom, authored);
        assert_eq!(placed, PxRect { x: 960.0 + 22.5, y: 1080.0 - 9.0, w: 45.0, h: 18.0 });
    }
}
