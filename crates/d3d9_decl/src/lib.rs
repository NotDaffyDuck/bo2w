//! Vertex declaration vocabulary: what an element's data looks like in memory
//! and which shader input it feeds.

#![no_std]
#![forbid(unsafe_code)]

mod decl_type;
mod usage;

pub use decl_type::DeclType;
pub use usage::{
    D3DDECLUSAGE_COLOR, D3DDECLUSAGE_DEPTH, D3DDECLUSAGE_NORMAL, D3DDECLUSAGE_POSITION,
    D3DDECLUSAGE_TEXCOORD, Semantic,
};
