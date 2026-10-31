//! Sun-shadow data: the atlas the cascades render into, the volumes casters are
//! gathered from, and the per-frame constants the receivers read.

mod atlas;
mod frame;
mod volume;

pub use self::atlas::{
    SUN_CASTER_TECH, DAYLIGHT_SHADOW_FORCED_PROFILE, SUNLIGHT_CASCADE_TOTAL,
    DaylightShadowAtlasProfile, SunlightShadowRegion,
};
pub use self::frame::{
    DaylightShadowCasterLists, DaylightShadowForcedFrame, SunCascadeSlice, SunCascadeSliceLists,
    DaylightShadowReceiverConstants,
};
pub use self::volume::{SUN_SHADOW_MAX_CLIP_PLANES, SunShadowClipPlanes};
