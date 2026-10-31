//! Names the renderer crates share about Shader Model 3 passes.
//!
//! The pass lowering that used to live here is gone; what is left is the
//! layout of the texture table, the shape of a lowered pass and the list of
//! opcodes the renderer reports as covered.

/// Bind group that holds the texture table.
pub const TEX_TABLE_BIND_GROUP: u32 = 1;
/// Binding of the 2D textures inside that group.
pub const TEX_TABLE_SLOT_2D: u32 = 0;
/// Binding of the cube textures.
pub const TEX_TABLE_SLOT_CUBE: u32 = 1;
/// Binding of the volume textures.
pub const TEX_TABLE_SLOT_3D: u32 = 2;
/// Binding of the sampler array.
pub const TEX_TABLE_SLOT_SAMPLERS: u32 = 3;


/// What a sampler reads from: a flat image, a cube or a volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SamplerTexShape {
    D2,
    Cube,
    D3,
}

/// A lowered pass: its WGSL source and the sizes the host binds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassShaderText {
    pub source: String,
    pub attribute_tally: usize,
    pub interpolant_tally: usize,
    pub vs_const_count: usize,
    pub ps_const_count: usize,
    pub sampler_tally: usize,
}

/// Opcode numbers counted as covered, lowest first.
pub const SUPPORTED_OP_FACET: &[u16] = &[
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
    0x0e, 0x0f, 0x12, 0x13, 0x1f, 0x20, 0x23, 0x24, 0x25, 0x26, 0x27, 0x29, 0x2a, 0x2b,
    0x2e, 0x30, 0x41, 0x42, 0x51, 0x58, 0x5a, 0x5b, 0x5c, 0x5d, 0x5f,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_opcodes_are_sorted_and_unique() {
        assert_eq!(SUPPORTED_OP_FACET.len(), 39);
        assert!(SUPPORTED_OP_FACET.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
