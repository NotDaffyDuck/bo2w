//! The 2D heads-up display: draws what `hud_t6` works out and the snapshot
//! presents, as bevy UI nodes plus one tessellated GPU list.

mod clock;
mod draw2d;
mod flash;
mod font_overlay;
mod gaps;
mod gpu_list;
mod hud_elems;
mod images;
mod iris;
mod menus;
mod overhead_names;
mod plugin;
mod presentation_scale;
mod rangefinder;
mod reticle;
mod sight;
mod surface;
mod ui_write;

pub use draw2d::{
    Draw2dCmd, Draw2dCmdCensus, Draw2dList, Draw2dOp, Draw2dProvenance, Draw2dQuad, tessellate,
};
pub use gaps::{GapCause, HudGap, HudPresentationGaps};
pub use gpu_list::{HudTessBatch, HudTessGpuFrame, HudTessTechnique, HudTessVertex};
pub use menus::ScriptMenus;
pub use overhead_names::{
    OverheadPosedHead, OverheadPosedPlayerFrame, OverheadPosedPlayerFramePublished,
};
pub use plugin::HudPlugin;
pub use presentation_scale::{
    HorizontalAlign, PlacedRect, PresentationScale, ScaleClass, VIRTUAL_HEIGHT, VIRTUAL_WIDTH,
    VerticalAlign,
};
pub use rangefinder::RangefinderSpots;
pub use surface::Hud2dSurface;
