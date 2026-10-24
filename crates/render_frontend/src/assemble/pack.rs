//! Draw packing lives in `render_frame`; the frontend re-exports it under the
//! names its callers already use.

pub use render_frame::packing::{
    HOST_XMODEL_RIGID_TESS_INFO_PACKED_ARM, PackDraw, PackKind, pack_spot_shadow_frontend,
    pack_sun_shadow_frontend,
};
