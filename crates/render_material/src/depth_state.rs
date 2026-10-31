//! Depth write and depth test from a draw state's second word (`loadBits[1]`):
//! bit 0 writes depth, bit 1 turns the test off, bits 2-3 pick the compare (the
//! same bits as BO2's `DrawState::decode` in asset_t6).

pub use d3d9_state::{
    STATE_DEPTH_WRITE_TOGGLE,
};
pub use d3d9_state::COMPARE_PASS_ALWAYS;
pub use d3d9_state::COMPARE_PASS_IF_SAME;
pub use d3d9_state::DEPTH_COMPARE_STRICT_BELOW;
pub use d3d9_state::DEPTH_COMPARE_BELOW_OR_SAME;
pub use d3d9_state::STATE_DEPTH_TEST;
pub use d3d9_state::STATE_DEPTH_COMPARE;

pub const DEPTH_WRITE_ENABLED_BIT: u32 = 0x1;

pub const DEPTH_COMPARE_OFF: u32 = 0x2;

pub const DEPTH_COMPARE_FIELD_SHIFT: u32 = 2;

pub const DEPTH_COMPARE_FIELD_MASK: u32 = 0xc;

pub const DEPTH_COMPARE_ALWAYS_PASS: u32 = 0x0;

pub const DEPTH_COMPARE_NEARER: u32 = 0x4;

pub const DEPTH_COMPARE_SAME: u32 = 0x8;

pub const DEPTH_COMPARE_NEARER_OR_SAME: u32 = 0xc;

pub const DEPTH_COMPARE_LOOKUP: [u32; 4] =
    [COMPARE_PASS_ALWAYS, DEPTH_COMPARE_STRICT_BELOW, COMPARE_PASS_IF_SAME, DEPTH_COMPARE_BELOW_OR_SAME];

#[inline]
pub const fn depth_test_field(dword1: u32) -> u32 {
    (dword1 & DEPTH_COMPARE_FIELD_MASK) >> DEPTH_COMPARE_FIELD_SHIFT
}

#[inline]
pub const fn depth_writing_on(dword1: u32) -> bool {
    (dword1 & DEPTH_WRITE_ENABLED_BIT) != 0
}

#[inline]
pub const fn depth_testing_on(dword1: u32) -> bool {
    (dword1 & DEPTH_COMPARE_OFF) == 0
}

#[inline]
pub const fn legacy_depth_compare_from_state_word(dword1: u32) -> u32 {
    DEPTH_COMPARE_LOOKUP[depth_test_field(dword1) as usize]
}
