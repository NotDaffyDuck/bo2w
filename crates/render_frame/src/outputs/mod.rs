//! What one rendered frame is made of, as the frontend publishes it: the
//! per-product draw lists, the revision counters that say what moved, and the
//! snapshot that carries them across to the renderer.

mod revisions;
mod segments;
mod frame_snap;
mod status;

pub use self::revisions::{InputChangeCounters, publish_rows};
pub use self::segments::{COMPACT_SEGMENT_CLAIMANTS, CompactSegment, CompactSegments};
pub use self::frame_snap::{FrameOutputsSnapshot, FocusedFrameSnapshot};
pub use self::status::{
    FrameOutputItem, FrameOutputKind, FrameOutputState, AbsentOutputReason, OutputTarget,
    ConeShadowFrameSlot,
};
