//! bo2mp: Black Ops II's collision traces, written from BO2's own order and
//! numbers: the trace result (`trace_t`), its hit ids, the trace setup that
//! turns a box into BO2's capsule (`CM_Trace`), and the sweep and position
//! test against one brush (`CM_TraceThroughBrush`, `CM_TestBoxInBrush`).
//!
//! It depends on no game crate, so another project can use it as a path
//! dependency. The map tree, leafs and models that feed it brushes live in
//! the clip map crate.

#![no_std]
#![forbid(unsafe_code)]

mod brush;
mod bullet;
mod hit;
mod trace;
mod work;

pub use brush::{AXIAL_SIDES, Brush, BrushSide, Plane, test_box_in_brush, trace_through_brush};
pub use bullet::BulletFireParams;
pub use hit::{
    DynEntDrawType, ENTITYNUM_NONE, ENTITYNUM_WORLD, MAX_GENTITIES, TraceHitType,
    trace_get_dyn_ent_hit_id, trace_get_entity_hit_id, trace_get_glass_hit_id, trace_set_hit_id,
};
pub use trace::{SURF_TYPE_MASK, SURF_TYPE_SHIFT, Trace, surface_type_from_flags};
pub use work::{SURFACE_CLIP_EPSILON, TraceExtents, TraceWork, trace_brushes};
