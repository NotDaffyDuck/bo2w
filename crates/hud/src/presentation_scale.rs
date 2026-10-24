//! Placing 640x480-authored HUD rectangles on a window of any size.

/// The authored canvas the HUD coordinates are written against.
pub const VIRTUAL_HEIGHT: f32 = 480.0;
pub const VIRTUAL_WIDTH: f32 = 640.0;

// Chrome is drawn at half the projection scale, and the lower strip of it a
// little larger.
const CHROME_SHARE: f32 = 0.5;
const LOWER_CHROME_GAIN: f32 = 1.4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HorizontalAlign {
    SubLeft = 0,
    Left = 1,
    Center = 2,
    Right = 3,
    Fullscreen = 4,
    NoScale = 5,
    To640 = 6,
    CenterSafeArea = 7,
    UserLeft = 8,
    UserCenter = 9,
    UserRight = 10,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerticalAlign {
    SubTop = 0,
    Top = 1,
    Center = 2,
    Bottom = 3,
    Fullscreen = 4,
    NoScale = 5,
    To480 = 6,
    CenterSafeArea = 7,
    UserTop = 8,
    UserCenter = 9,
    UserBottom = 10,
}

/// Which scale an element follows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScaleClass {
    Chrome,
    LowerChrome,
    ProjectionBound,
    ViewportFill,
}

/// The window the HUD is placed on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PresentationScale {
    width: f32,
    height: f32,
}

/// A rectangle in window pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PlacedRect {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl PresentationScale {
    pub fn from_window(width: f32, height: f32) -> Self {
        Self {
            width: width.max(1.0),
            height: height.max(1.0),
        }
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Window pixels per authored unit for the class.
    pub fn factor(&self, class: ScaleClass) -> f32 {
        let per_unit = self.height / VIRTUAL_HEIGHT;
        match class {
            ScaleClass::Chrome => per_unit * CHROME_SHARE,
            ScaleClass::LowerChrome => per_unit * CHROME_SHARE * LOWER_CHROME_GAIN,
            ScaleClass::ProjectionBound | ScaleClass::ViewportFill => per_unit,
        }
    }

    /// The largest 4:3 rectangle that fits in the window, as (width, height).
    fn four_by_three(&self) -> (f32, f32) {
        let wide = (self.height * (VIRTUAL_WIDTH / VIRTUAL_HEIGHT)).min(self.width);
        let tall = (self.width * (VIRTUAL_HEIGHT / VIRTUAL_WIDTH)).min(self.height);
        (wide, tall)
    }

    /// Window x of the origin the alignment counts from.
    pub fn anchor_x(&self, align: HorizontalAlign) -> f32 {
        use HorizontalAlign::*;
        match align {
            SubLeft => (self.width - self.four_by_three().0) * 0.5,
            Center | CenterSafeArea | UserCenter => self.width * 0.5,
            Right | UserRight => self.width,
            Left | Fullscreen | NoScale | UserLeft | To640 => 0.0,
        }
    }

    /// Window y of the origin the alignment counts from.
    pub fn anchor_y(&self, align: VerticalAlign) -> f32 {
        use VerticalAlign::*;
        match align {
            SubTop => (self.height - self.four_by_three().1) * 0.5,
            Center | CenterSafeArea | UserCenter => self.height * 0.5,
            Bottom | UserBottom => self.height,
            Top | Fullscreen | NoScale | UserTop | To480 => 0.0,
        }
    }

    pub fn place(
        &self,
        class: ScaleClass,
        h_align: HorizontalAlign,
        v_align: VerticalAlign,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> PlacedRect {
        if class == ScaleClass::ViewportFill {
            return PlacedRect {
                left: 0.0,
                top: 0.0,
                width: self.width,
                height: self.height,
            };
        }
        let scale = self.factor(class);
        PlacedRect {
            left: self.anchor_x(h_align) + x * scale,
            top: self.anchor_y(v_align) + y * scale,
            width: width * scale,
            height: height * scale,
        }
    }
}
