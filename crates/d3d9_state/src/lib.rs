//! The fixed-function state vocabulary: compare functions, blend factors, the
//! alpha test and the render-state ids and values a material sets.

#![no_std]
#![forbid(unsafe_code)]

mod alpha_test;
mod blend;
mod cmp;
mod render_state;

pub use alpha_test::{ALPHA_REF_SCALE, AlphaTest};
pub use blend::BlendFactor;
pub use cmp::{
    CompareFunc, D3DCMP_ALWAYS, D3DCMP_EQUAL, D3DCMP_GREATER, D3DCMP_GREATEREQUAL, D3DCMP_LESS,
    D3DCMP_LESSEQUAL,
};
pub use render_state::{
    D3DCULL_CCW, D3DCULL_CW, D3DCULL_NONE, D3DRS_CULLMODE, D3DRS_ZENABLE, D3DRS_ZFUNC,
    D3DRS_ZWRITEENABLE,
};
