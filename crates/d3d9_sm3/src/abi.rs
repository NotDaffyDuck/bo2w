//! What a pass tells the emitter about its surroundings: which vertex
//! attributes exist, which registers link the two stages, and which constant
//! and sampler slots the host bound.

use alloc::vec::Vec;

use d3d9_decl::{DeclType, Semantic};
use d3d9_state::AlphaTest;

use crate::ir::Sm3Register;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SamplerTextureDimension {
    D2,
    Cube,
    D3,
}

/// One vertex-shader input register. With no `location` the pass has no
/// attribute for it and the register reads as a constant default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexInput {
    pub register: Sm3Register,
    pub location: Option<u32>,
    pub decl_type: Option<DeclType>,
    pub semantic: Semantic,
}

/// A vertex output register and the pixel input register it feeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VaryingLink {
    pub semantic: Semantic,
    pub vertex_register: Sm3Register,
    pub pixel_register: Option<Sm3Register>,
    pub location: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstantSlot {
    pub register: u16,
    /// The program defines this constant itself, so the host need not supply it.
    pub program_defined: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SamplerSlot {
    pub register: u16,
    pub dimension: SamplerTextureDimension,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassLoweringAbi {
    pub vertex_inputs: Vec<VertexInput>,
    pub position: VaryingLink,
    pub varyings: Vec<VaryingLink>,
    pub vertex_constants: Vec<ConstantSlot>,
    pub pixel_constants: Vec<ConstantSlot>,
    pub samplers: Vec<SamplerSlot>,
    /// One extra pixel entry point is emitted per alpha test.
    pub alpha_tests: Vec<AlphaTest>,
}
