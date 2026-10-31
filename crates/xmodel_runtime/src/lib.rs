//! Retained model runtime for both the authority and the client. It poses a
//! model's bones from its animation tree and reads back the collision shapes.
//! The work is done in `dobj_t6` and `xanim_t6`; this crate is where both
//! sides import it from, so it passes their public items straight through.

mod breakables;

pub use breakables::*;
pub use dobj_t6::*;
pub use xanim_t6::*;
