//! What one rendered frame is made of, as the frontend publishes it: the
//! per-product draw lists, the revision counters that say what moved, and the
//! snapshot that carries them across to the renderer.

mod revisions;
mod segments;
mod snapshot;
mod status;

pub use self::revisions::{SourceRevisions, publish_rows};
pub use self::segments::{PACKED_SEGMENT_OWNERS, PackedSegment, PackedSegments};
pub use self::snapshot::{FrameProductsSnapshot, RenderFocusFrame};
pub use self::status::{
    FrameProduct, FrameProductKind, FrameProductStatus, MissingProductCause, ProductTarget,
    SpotShadowFrameSlot,
};
