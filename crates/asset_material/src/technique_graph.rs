//! The shader side of a material: technique sets, their passes, and the
//! programs and vertex declarations the passes point at.
//!
//! No Black Ops II material carries a technique set (its draw is the `T6Draw`
//! on the row), so a finished population holds none of these. The types stay
//! because the render crates still read them, and they go when that reading
//! does.

use crate::{AssetNamespace, AssetRef, ROUTING_COUNT, VertexLayoutFamily};

/// The stage a shader program runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    Vertex,
    Pixel,
}

#[derive(Clone, Debug)]
pub struct AuthoredShader {
    pub namespace: AssetNamespace,
    pub name: AssetRef,
    pub kind: ShaderKind,
    pub program: Vec<u8>,
}

impl AuthoredShader {
    pub const fn is_vertex(&self) -> bool {
        matches!(self.kind, ShaderKind::Vertex)
    }

    pub const fn is_pixel(&self) -> bool {
        matches!(self.kind, ShaderKind::Pixel)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredVertexDecl {
    pub family: VertexLayoutFamily,
    pub name: AssetRef,
    pub stream_count: u8,
    pub has_optional_source: u8,
    pub routing: [[u8; 2]; ROUTING_COUNT],
}

/// A program a pass runs, as a row in the population's shader list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedShaderRef {
    pub shader: Option<usize>,
}

/// One value a pass feeds its shaders, and where it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedShaderArgument {
    MaterialVertexConstant {
        destination: u16,
        name_hash: u32,
    },
    LiteralVertexConstant {
        destination: u16,
        words: Option<[u32; 4]>,
    },
    MaterialPixelSampler {
        destination: u16,
        name_hash: u32,
    },
    CodeVertexConstant {
        destination: u16,
        index: u16,
        first_row: u8,
        row_count: u8,
    },
    CodePixelSampler {
        destination: u16,
        index: u32,
    },
    CodePixelConstant {
        destination: u16,
        index: u16,
        first_row: u8,
        row_count: u8,
    },
    MaterialPixelConstant {
        destination: u16,
        name_hash: u32,
    },
    LiteralPixelConstant {
        destination: u16,
        words: Option<[u32; 4]>,
    },
    Unknown {
        argument_type: u16,
        raw: [u8; 8],
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedMaterialPass {
    /// Row in the vertex declaration list.
    pub vertex_decl: Option<usize>,
    pub vertex_shader: OwnedShaderRef,
    pub pixel_shader: OwnedShaderRef,
    /// `arguments` is laid out per-primitive, then per-object, then stable.
    pub per_prim_arg_count: u8,
    pub per_obj_arg_count: u8,
    pub stable_arg_count: u8,
    pub custom_sampler_flags: u8,
    pub t5_custom_sampler_flags: u8,
    pub arguments: Vec<OwnedShaderArgument>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedTechnique {
    pub flags: u16,
    pub passes: Vec<OwnedMaterialPass>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedTechniqueGraph {
    pub slots: Vec<Option<OwnedTechnique>>,
}

/// Which technique slots a set fills, and what each holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechniqueTable {
    /// Bit `n` is set when slot `n` has a technique.
    pub slots: u64,
    pub technique0_flags: u16,
    pub graph: Option<OwnedTechniqueGraph>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechniqueSetFacts {
    pub namespace: AssetNamespace,
    pub name: AssetRef,
    pub table: Option<TechniqueTable>,
    pub world_vert_format: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TechsetKey<'a> {
    pub namespace: AssetNamespace,
    pub name: &'a str,
}

impl<'a> TechsetKey<'a> {
    pub fn new(namespace: AssetNamespace, name: &'a str) -> Self {
        Self { namespace, name }
    }
}

/// How a technique-set lookup came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TechsetResolve<'a> {
    Hit {
        index: usize,
        facts: &'a TechniqueSetFacts,
    },
    /// The set exists but carries no technique graph.
    GraphMissing { index: usize },
    /// Only another game's namespace has it.
    Foreign { got: AssetNamespace, index: usize },
    Missing,
}
