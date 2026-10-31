//! The 2D heads-up display: draws what `hud_t6` works out and the snapshot
//! presents, as bevy UI nodes plus one tessellated GPU list.

mod clock;
mod draw2d;
mod flash;
mod glyph_layer;
mod gaps;
mod draw_batch;
mod hud_elems;
mod images;
mod iris;
mod menus;
mod name_tags;
mod plugin;
mod screen_fit;
mod rangefinder;
mod aim_mark;
mod sight;
mod surface;
mod ui_emit;

pub use draw2d::FlatPaintEntry;
pub use draw2d::FlatPaintTally;
pub use draw2d::FlatPaintQueue;
pub use draw2d::FlatPaintAction;
pub use draw2d::FlatPaintOrigin;
pub use draw2d::FlatPaintRect;
pub use draw2d::expand_paint_list;
pub use gaps::GapReason;
pub use gaps::HudShortage;
pub use gaps::HudGapBook;
pub use draw_batch::HudStitchBatch;
pub use draw_batch::OverlayMeshBatch;
pub use draw_batch::HudStitchMethod;
pub use draw_batch::HudGpuVertex;
pub use menus::ScriptMenus;
pub use name_tags::{
    NameplateSkullPose, NameplateBodyPose, NameplateBodyPoseShared,
};
pub use plugin::OverlayModule;
pub use screen_fit::{
    AnchorX, PxRect, WindowScale, UnitClass, CANVAS_HEIGHT, CANVAS_WIDTH,
    AnchorY,
};
pub use rangefinder::RangefinderSpots;
pub use surface::FlatOverlayCanvas;
