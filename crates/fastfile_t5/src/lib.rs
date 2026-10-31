//! What two crates still compile against from the retail BO1 zone format: the
//! block layout, and the vertex declaration tables. No BO2 path reads a BO1
//! zone. The crate goes when `asset_transport::T5ZoneMemory` and
//! `render_material::VertexFormatLineage::T5` do.

#![no_std]
#![forbid(unsafe_code)]

pub mod vtx_formats;
mod zone;

pub use zone::*;
