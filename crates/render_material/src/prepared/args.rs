//! A pass's arguments, split into the three update belts and pre-bound to the
//! material's own textures and constants.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::argument::mtl_arg;
use crate::catalog::{RuntimeImageId, RuntimeMaterial, RuntimePass, RuntimeTextureBinding};
use crate::{RuntimeArgumentBinding, RuntimeShaderStage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutableArgumentBinding {
    MaterialTexture {
        destination: u16,
        name_hash: u32,
        texture: RuntimeTextureBinding,
    },
    CodeTexture {
        destination: u16,
        index: u32,
        sampler_state: u8,
        image: Option<RuntimeImageId>,
    },
    MaterialConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        name_hash: u32,
        words: [u32; 4],
    },
    LiteralConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        words: [u32; 4],
    },
    CodeConstant {
        stage: RuntimeShaderStage,
        destination: u16,
        index: u16,
        first_row: u8,
        row_count: u8,
        rows: Arc<[[u32; 4]]>,
    },
}

/// An argument the engine fills in per draw rather than the material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PackedCodeArg {
    Constant {
        stage: RuntimeShaderStage,
        destination: u16,
        index: u16,
        first_row: u8,
        row_count: u8,
    },
    Sampler {
        destination: u16,
        index: u32,
    },
    Unknown {
        argument_type: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedArgSlice {
    /// One entry per argument; `None` where the argument is not material-local
    /// or the material has no value for it.
    pub local: Vec<Option<ExecutableArgumentBinding>>,
    pub code: Vec<PackedCodeArg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedArgBelts {
    pub per_prim: PreparedArgSlice,
    pub per_obj: PreparedArgSlice,
    pub stable: PreparedArgSlice,
}

impl PreparedArgBelts {
    /// The belts in the order their locals are written into the banks.
    pub(super) fn in_bank_order(&self) -> [&PreparedArgSlice; 3] {
        [&self.per_prim, &self.per_obj, &self.stable]
    }

    /// `None` when the pass's belt counts do not add up to its argument list.
    pub(super) fn from_pass(material: &RuntimeMaterial, pass: &RuntimePass) -> Option<Self> {
        let per_prim = usize::from(pass.per_prim_arg_count);
        let per_obj = usize::from(pass.per_obj_arg_count);
        let stable = usize::from(pass.stable_arg_count);
        if per_prim.saturating_add(per_obj).saturating_add(stable) != pass.arguments.len() {
            return None;
        }
        let (prim_args, rest) = pass.arguments.split_at(per_prim);
        let (obj_args, stable_args) = rest.split_at(per_obj);
        Some(Self {
            per_prim: PreparedArgSlice::new(material, prim_args),
            per_obj: PreparedArgSlice::new(material, obj_args),
            stable: PreparedArgSlice::new(material, stable_args),
        })
    }
}

impl PreparedArgSlice {
    fn new(material: &RuntimeMaterial, args: &[RuntimeArgumentBinding]) -> Self {
        Self {
            local: bind_material_locals(material, args),
            code: args.iter().filter_map(code_arg).collect(),
        }
    }
}

fn code_arg(argument: &RuntimeArgumentBinding) -> Option<PackedCodeArg> {
    match *argument {
        RuntimeArgumentBinding::CodeConstant {
            stage,
            destination,
            index,
            first_row,
            row_count,
        } => Some(PackedCodeArg::Constant {
            stage,
            destination,
            index,
            first_row,
            row_count,
        }),
        RuntimeArgumentBinding::CodeTexture { destination, index } => {
            Some(PackedCodeArg::Sampler { destination, index })
        }
        RuntimeArgumentBinding::Unknown { argument_type, .. } => {
            Some(PackedCodeArg::Unknown { argument_type })
        }
        RuntimeArgumentBinding::MaterialTexture { .. }
        | RuntimeArgumentBinding::MaterialConstant { .. }
        | RuntimeArgumentBinding::LiteralConstant { .. } => None,
    }
}

/// Where the last lookup stopped in each of the material's hash-sorted
/// tables. Arguments arrive roughly in hash order, so most lookups continue
/// from the previous hit instead of rescanning.
#[derive(Default)]
struct Cursors {
    textures: usize,
    constants: usize,
}

fn find_sorted<T: Copy>(table: &[(u32, T)], cursor: &mut usize, wanted: u32) -> Option<T> {
    // Remapped code samplers can ask for hashes outside the authored order.
    if table.get(*cursor).is_none_or(|&(hash, _)| hash > wanted) {
        *cursor = 0;
    }
    while let Some(&(hash, value)) = table.get(*cursor) {
        match hash.cmp(&wanted) {
            Ordering::Equal => return Some(value),
            Ordering::Greater => return None,
            Ordering::Less => *cursor += 1,
        }
    }
    None
}

fn bind_one(
    material: &RuntimeMaterial,
    argument: &RuntimeArgumentBinding,
    cursors: &mut Cursors,
) -> Option<ExecutableArgumentBinding> {
    match *argument {
        RuntimeArgumentBinding::MaterialTexture {
            destination,
            name_hash,
        } => {
            let texture = find_sorted(&material.textures, &mut cursors.textures, name_hash)??;
            Some(ExecutableArgumentBinding::MaterialTexture {
                destination,
                name_hash,
                texture,
            })
        }
        RuntimeArgumentBinding::MaterialConstant {
            stage,
            destination,
            name_hash,
        } => {
            let words = find_sorted(&material.constants, &mut cursors.constants, name_hash)?;
            Some(ExecutableArgumentBinding::MaterialConstant {
                stage,
                destination,
                name_hash,
                words,
            })
        }
        RuntimeArgumentBinding::LiteralConstant {
            stage,
            destination,
            words,
        } => Some(ExecutableArgumentBinding::LiteralConstant {
            stage,
            destination,
            words: words?,
        }),
        RuntimeArgumentBinding::CodeTexture { .. }
        | RuntimeArgumentBinding::CodeConstant { .. }
        | RuntimeArgumentBinding::Unknown { .. } => None,
    }
}

/// Argument kinds in the order the authored argument list groups them.
const AUTHORED_KIND_ORDER: [u16; 8] = [
    mtl_arg::MATERIAL_VERTEX_CONST,
    mtl_arg::LITERAL_VERTEX_CONST,
    mtl_arg::MATERIAL_PIXEL_SAMPLER,
    mtl_arg::CODE_VERTEX_CONST,
    mtl_arg::CODE_PIXEL_SAMPLER,
    mtl_arg::CODE_PIXEL_CONST,
    mtl_arg::MATERIAL_PIXEL_CONST,
    mtl_arg::LITERAL_PIXEL_CONST,
];

fn bind_material_locals(
    material: &RuntimeMaterial,
    args: &[RuntimeArgumentBinding],
) -> Vec<Option<ExecutableArgumentBinding>> {
    let mut bound = vec![None; args.len()];
    let mut cursors = Cursors::default();
    let mut next = 0;
    for kind in AUTHORED_KIND_ORDER {
        // Vertex and pixel constants are separate tables in the source data,
        // so the constant search restarts when the pixel constants begin.
        if kind == mtl_arg::MATERIAL_PIXEL_CONST {
            cursors.constants = 0;
        }
        while next < args.len() && args[next].mtl_arg_type() == kind {
            bound[next] = bind_one(material, &args[next], &mut cursors);
            next += 1;
        }
    }
    for index in next..args.len() {
        bound[index] = bind_one(material, &args[index], &mut cursors);
    }
    bound
}
