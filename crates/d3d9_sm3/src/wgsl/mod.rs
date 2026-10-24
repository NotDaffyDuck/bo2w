//! Lowering a vertex and pixel program pair into one WGSL module.
//!
//! The text and the binding layout are a contract with the renderer and its
//! shader cache, so the emitter is deterministic: the same programs and ABI
//! always give the same bytes.

/// Append formatted text to a `String`; writing to a `String` cannot fail.
macro_rules! emit {
    ($out:expr, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = write!($out, $($arg)*);
    }};
}

mod body;
mod expr;
mod pass;
mod plan;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;

use crate::abi::SamplerTextureDimension;
use crate::ir::{Sm3LoweringRefusal, Sm3Opcode, Sm3Register, Sm3RegisterFile};

pub use pass::{T6_VERTEX_HELPERS, lower_pass_to_wgsl, pass_fragment_alpha_test_entry};

pub const PASS_VERTEX_ENTRY: &str = "sm3_vertex_main";
pub const PASS_FRAGMENT_ENTRY: &str = "sm3_fragment_main";

/// The bind group that holds the texture and sampler tables.
pub const TEXTURE_TABLE_GROUP: u32 = 1;
pub const TEXTURE_TABLE_BINDING_2D: u32 = 0;
pub const TEXTURE_TABLE_BINDING_CUBE: u32 = 1;
pub const TEXTURE_TABLE_BINDING_3D: u32 = 2;
pub const TEXTURE_TABLE_BINDING_SAMPLERS: u32 = 3;

/// A single program lowered on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sm3Wgsl {
    pub source: String,
    pub output_masks: BTreeMap<Sm3Register, u8>,
}

/// One lowered pass: the module text and the sizes the host binds against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassWgsl {
    pub source: String,
    pub attribute_count: usize,
    pub varying_count: usize,
    pub vertex_constant_len: usize,
    pub pixel_constant_len: usize,
    pub sampler_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sm3WgslError {
    LoweringSurface(Sm3LoweringRefusal),
    RelativeConstantsNeedPassAbi,
    SamplersNeedPassAbi,
    UnboundSamplerRegister {
        register: u16,
    },
    UnsupportedInstructionControls {
        at_word: usize,
        opcode: Sm3Opcode,
        controls: u8,
    },
    UnsupportedDestinationFile {
        at_word: usize,
        file: Sm3RegisterFile,
    },
    UnsupportedSourceFile {
        at_word: usize,
        file: Sm3RegisterFile,
    },
    InvalidSamplerOperand {
        at_word: usize,
    },
    InvalidNormaliseDestination {
        at_word: usize,
        component: u8,
    },
    InvalidSincosDestination {
        at_word: usize,
        component: u8,
    },
    RegisterReadBeforeWrite {
        at_word: usize,
        register: Sm3Register,
        component: u8,
    },
    PartialConstantDefinition {
        at_word: usize,
        register: Sm3Register,
        write_mask: u8,
    },
    NoProgramOutput,
    DeclaredOutputNeverWritten {
        register: Sm3Register,
    },
    MissingColourOutput,
    UnsupportedAlphaTestFunc {
        raw_func: Option<u32>,
    },
    UnsupportedPixelOutput {
        register: Sm3Register,
    },
    UnlinkedVertexOutput {
        register: Sm3Register,
    },
    UnboundConstantRegister {
        register: Sm3Register,
    },
    SamplerDeclarationMissing {
        register: u16,
    },
    UnsupportedSamplerTextureType {
        register: u16,
        texture_type: u32,
    },
    PixelOnlyOpcode {
        at_word: usize,
        opcode: Sm3Opcode,
    },
    VertexOnlyOpcode {
        at_word: usize,
        opcode: Sm3Opcode,
    },
    SaturateOnAddress {
        at_word: usize,
    },
    UnbalancedControlFlow {
        at_word: usize,
        kind: &'static str,
    },
}

/// The WGSL name of a register: `r0`, `c12`, `oC1` style, lower case.
fn register_name(register: Sm3Register) -> String {
    use Sm3RegisterFile as F;
    let prefix = match register.file {
        F::Temporary => "r",
        F::Input => "v",
        F::FloatConstant => "c",
        F::Texture => "t",
        F::Address => "a",
        F::RasterOutput => "or",
        F::AttributeOutput => "oa",
        F::Output => "o",
        F::IntegerConstant => "i",
        F::ColourOutput => "oc",
        F::DepthOutput => "od",
        F::BooleanConstant => "b",
        F::Loop => "al",
        F::Float16Temporary => "h",
        F::Misc => "misc",
        F::Label => "label",
        F::Predicate => "p",
        F::Sampler => "s",
        F::Unknown(raw) => return format!("unknown{raw}_{}", register.index),
    };
    format!("{prefix}{}", register.index)
}

fn component_name(component: u8) -> char {
    ['x', 'y', 'z', 'w'][usize::from(component & 3)]
}

fn texture_array_name(dimension: SamplerTextureDimension) -> &'static str {
    match dimension {
        SamplerTextureDimension::D2 => "sm3_textures_2d",
        SamplerTextureDimension::Cube => "sm3_textures_cube",
        SamplerTextureDimension::D3 => "sm3_textures_3d",
    }
}
