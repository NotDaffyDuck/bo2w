//! Sun-shadow data: the atlas the cascades render into, the volumes casters are
//! gathered from, and the per-frame constants the receivers read.

mod atlas;
mod frame;
mod volume;

pub use self::atlas::{
    SUN_SHADOW_CASTER_TECH, SUN_SHADOW_FORCED_PROFILE, SUN_SHADOW_PARTITION_COUNT,
    SunShadowAtlasProfile, SunShadowViewport,
};
pub use self::frame::{
    SunShadowCasterLists, SunShadowForcedFrame, SunShadowPartition, SunShadowPartitionLists,
    SunShadowReceiverConstants,
};
pub use self::volume::{SUN_SHADOW_MAX_CLIP_PLANES, SunShadowClipPlanes};
