//! Names for the fixed-function numbers a material carries: how depth and alpha are
//! compared, how colours blend, the alpha cutoff, and which render-state slot is set.

#![no_std]
#![forbid(unsafe_code)]

mod alpha_gate;
mod blend_weight;
mod compare;
mod state_ids;

pub use alpha_gate::{ALPHA_CUTOFF_UNIT, AlphaGate};
pub use blend_weight::{BlendWeight};
pub use compare::{
    COMPARE_PASS_ALWAYS, COMPARE_PASS_IF_ABOVE, COMPARE_PASS_IF_ABOVE_OR_SAME, COMPARE_PASS_IF_SAME, CompareOp,
    DEPTH_COMPARE_BELOW_OR_SAME, DEPTH_COMPARE_STRICT_BELOW,
};
pub use state_ids::{
    STATE_FACE_CULL, STATE_DEPTH_TEST, STATE_DEPTH_COMPARE, FACE_CULL_CLOCKWISE, FACE_CULL_DISABLED, STATE_DEPTH_WRITE_TOGGLE,
    WINDING_CULL_COUNTERCLOCKWISE,
};
