//! The shader side of a material: technique sets, their passes, and the
//! programs and vertex declarations the passes point at.
//!
//! No Black Ops II material carries a technique set (its draw is the `T6Draw`
//! on the row), so a finished population holds none of these. The types stay
//! because the render crates still read them, and they go when that reading
//! does.

use crate::{AssetCategoryScope, AssetPointer, MIX_PATH_TOTAL, VertexFormatLineage};

/// The stage a shader program runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderStageVariety {
    Vertex,
    Pixel,
}

#[derive(Clone, Debug)]
pub struct HandBuiltShader {
    pub namespace: AssetCategoryScope,
    pub name: AssetPointer,
    pub kind: ShaderStageVariety,
    pub bytecode: Vec<u8>,
}

impl HandBuiltShader {
    pub const fn is_corner_stage(&self) -> bool {
        match self.kind {
            ShaderStageVariety::Vertex => true,
            ShaderStageVariety::Pixel => false,
        }
    }

    pub const fn is_fragment_stage(&self) -> bool {
        !self.is_corner_stage()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexDeclRow {
    pub family: VertexFormatLineage,
    pub name: AssetPointer,
    pub stream_tally: u8,
    pub has_spare_source: u8,
    pub mix_paths: [[u8; 2]; MIX_PATH_TOTAL],
}

/// A program a pass runs, as a row in the population's shader list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfContainedShaderLink {
    pub shader_slot: Option<usize>,
}

/// One value a pass feeds its shaders, and where it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeldShaderArg {
    MaterialVertConst { slot: u16, label_digest: u32 },
    FixedVertexConstant { slot: u16, bit_words: Option<[u32; 4]> },
    OwnPixelSampler { slot: u16, label_digest: u32 },
    EngineVertexConstant { slot: u16, index: u16, start_row: u8, line_total: u8 },
    EnginePixelSampler { slot: u16, index: u32 },
    EnginePixelConstant { into: u16, const_slot: u16, row_first: u8, line_total: u8 },
    OwnPixelConstant { slot: u16, label_digest: u32 },
    FixedPixelConstant { slot: u16, words: Option<[u32; 4]> },
    Unknown { param_type: u16, raw: [u8; 8] },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfContainedShaderPass {
    /// Row in the vertex declaration list.
    pub vertex_shape: Option<usize>,
    pub vertex_shader: SelfContainedShaderLink,
    pub pixel_shader: SelfContainedShaderLink,
    /// `arguments` is laid out per-primitive, then per-object, then stable.
    pub per_prim_arg_tally: u8,
    pub per_obj_arg_tally: u8,
    pub stable_arg_tally: u8,
    pub own_sampler_bits: u8,
    pub classic_custom_sampler_flags: u8,
    pub arguments: Vec<HeldShaderArg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfContainedRenderRecipe {
    pub bits: u16,
    pub passes: Vec<SelfContainedShaderPass>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechniqueGraph {
    pub slots: Vec<Option<SelfContainedRenderRecipe>>,
}

/// Which technique slots a set fills, and what each holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechniqueSlotTable {
    /// Bit `n` is set when slot `n` has a technique.
    pub cells: u64,
    pub technique0_flags: u16,
    pub graph: Option<TechniqueGraph>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecipeBundleSummary {
    pub namespace: AssetCategoryScope,
    pub name: AssetPointer,
    pub table: Option<TechniqueSlotTable>,
    pub level_vertex_format: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RecipeBundleId<'a> {
    pub namespace: AssetCategoryScope,
    pub name: &'a str,
}

impl<'a> RecipeBundleId<'a> {
    pub fn new(namespace: AssetCategoryScope, asset_name: &'a str) -> Self {
        Self { name: asset_name, namespace }
    }
}

/// How a technique-set lookup came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipeBundleLookup<'a> {
    Hit { graph_slot: usize, facts: &'a RecipeBundleSummary },
    /// The set exists but carries no technique graph.
    GraphMissing { index: usize },
    /// Only another game's namespace has it; `holder` is that namespace.
    Foreign { holder: AssetCategoryScope, entry: usize },
    Missing,
}
