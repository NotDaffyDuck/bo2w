//! Render-state ids and the cull values one of them takes.

pub const D3DRS_ZENABLE: u32 = 7;
pub const D3DRS_ZWRITEENABLE: u32 = 14;
pub const D3DRS_CULLMODE: u32 = 22;
pub const D3DRS_ZFUNC: u32 = 23;

pub const D3DCULL_NONE: u32 = 1;
pub const D3DCULL_CW: u32 = 2;
pub const D3DCULL_CCW: u32 = 3;
