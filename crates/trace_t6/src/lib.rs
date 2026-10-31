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
mod hit;
mod capsule;
mod result;

pub use brush::{AXIAL_SIDES, Brush, BrushSide, Plane, test_box_in_brush, trace_through_brush};
pub use hit::{
    NO_ENTITY_SLOT, ENTITYNUM_WORLD, MAX_GENTITIES, TraceHitType, WORLD_ENTITY_SLOT, entity_hit_slot, glass_hit_slot,
    trace_get_entity_hit_id, trace_set_hit_id,
};
pub use result::{SURF_TYPE_MASK, SURF_TYPE_SHIFT, Trace, TraceResult, surface_kind_of_flags};
pub use capsule::{SURFACE_CLIP_SLACK, SweepBoxSize, TraceWork, trace_brushes};
