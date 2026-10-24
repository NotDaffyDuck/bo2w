//! Shader Model 3 programs: decoding the token binary into an IR and turning a
//! vertex and pixel pair into one WGSL module.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod abi;
mod ir;
mod token;
mod wgsl;

pub use d3d9_decl::{DeclType, Semantic};
pub use d3d9_state::{AlphaTest, CompareFunc};

pub use abi::{
    ConstantSlot, PassLoweringAbi, SamplerSlot, SamplerTextureDimension, VaryingLink, VertexInput,
};
pub use ir::{
    OpcodeSurfaceCoverage, SUPPORTED_OPCODE_SURFACE, Sm3Destination, Sm3Instruction,
    Sm3InstructionBody, Sm3IrError, Sm3LoweringRefusal, Sm3Opcode, Sm3ProgramIr, Sm3Register,
    Sm3RegisterFile, Sm3RelativeAddress, Sm3Source, Sm3SourceModifier, decode_sm3_program,
    named_sm3_opcode_count, validate_supported_opcode_surface,
};
pub use token::{ShaderStage, TokenError};
pub use wgsl::{
    PASS_FRAGMENT_ENTRY, PASS_VERTEX_ENTRY, PassWgsl, Sm3Wgsl, Sm3WgslError, T6_VERTEX_HELPERS,
    TEXTURE_TABLE_BINDING_2D, TEXTURE_TABLE_BINDING_3D, TEXTURE_TABLE_BINDING_CUBE,
    TEXTURE_TABLE_BINDING_SAMPLERS, TEXTURE_TABLE_GROUP, lower_pass_to_wgsl,
    pass_fragment_alpha_test_entry,
};
