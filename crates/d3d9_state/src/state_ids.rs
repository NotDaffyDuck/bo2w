//! Which render-state slot a material writes, and the face-cull values that slot 22 takes.

pub const STATE_DEPTH_TEST: u32 = 7;
pub const STATE_DEPTH_WRITE_TOGGLE: u32 = 14;
pub const STATE_FACE_CULL: u32 = 22;
pub const STATE_DEPTH_COMPARE: u32 = 23;

pub const FACE_CULL_DISABLED: u32 = 1;
pub const FACE_CULL_CLOCKWISE: u32 = 2;
pub const WINDING_CULL_COUNTERCLOCKWISE: u32 = 3;
