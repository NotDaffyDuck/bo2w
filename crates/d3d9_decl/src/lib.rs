//! What a D3D9 vertex declaration says about each element: how its data is
//! laid out, and which shader input receives it.

#![no_std]
#![forbid(unsafe_code)]

mod decl_type;
mod usage;

pub use decl_type::VertexElementFormat;
pub use usage::*;
