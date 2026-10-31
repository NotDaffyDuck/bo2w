//! Draw packing is done in `render_frame`. The frontend passes it on under
//! the names its callers already import.

pub use render_frame::packing::{
    RIGID_MODEL_TESS_FLAGS_PACKED_BRANCH, PackableDrawCall, BundleKind,
    pack_cone_shadow_frontend, pack_daylight_shadow_frontend,
};
