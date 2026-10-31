//! A pass's arguments, split into the three update belts and pre-bound to the
//! material's own textures and constants.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::argument::arg_kind;
use crate::catalog::{LiveImageId, LiveMaterial, LivePassRecord, LiveTextureBinding};
use crate::{LiveArgumentBinding, LiveShaderStage};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnableParamSource {
    ShaderSamplerImage {
        slot_reg: u16,
        label_digest: u32,
        texture: LiveTextureBinding,
    },
    EngineSuppliedTexture {
        slot_reg: u16,
        index: u32,
        texture_sampling_mode: u8,
        image: Option<LiveImageId>,
    },
    ShaderParameterValue {
        stage: LiveShaderStage,
        slot_reg: u16,
        label_digest: u32,
        lane_bits: [u32; 4],
    },
    InlineConstant {
        stage: LiveShaderStage,
        slot_reg: u16,
        lane_bits: [u32; 4],
    },
    EngineConstant {
        stage: LiveShaderStage,
        slot_reg: u16,
        index: u16,
        row_base: u8,
        line_total: u8,
        row_data: Arc<[[u32; 4]]>,
    },
}

/// An argument the engine fills in per draw rather than the material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PackedCodeParam {
    Constant {
        stage: LiveShaderStage,
        slot_reg: u16,
        index: u16,
        row_base: u8,
        line_total: u8,
    },
    Sampler {
        slot_reg: u16,
        index: u32,
    },
    Unknown {
        kind_code: u16,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedParamSpan {
    /// One entry per argument; `None` where the argument is not material-local
    /// or the material has no value for it.
    pub local: Vec<Option<RunnableParamSource>>,
    pub code: Vec<PackedCodeParam>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedParamStrips {
    pub each_triangle_batch_inputs: PreparedParamSpan,
    pub each_mesh_inputs: PreparedParamSpan,
    pub stable: PreparedParamSpan,
}

impl PreparedParamStrips {
    /// The belts in the order their locals are written into the banks.
    pub(super) fn in_bank_order(&self) -> [&PreparedParamSpan; 3] {
        [&self.each_triangle_batch_inputs, &self.each_mesh_inputs, &self.stable]
    }

    /// `None` when the pass's belt counts do not add up to its argument list.
    pub(super) fn from_pass(material: &LiveMaterial, pass: &LivePassRecord) -> Option<Self> {
        let each_triangle_batch_inputs = usize::from(pass.per_prim_arg_tally);
        let each_mesh_inputs = usize::from(pass.per_obj_arg_tally);
        let stable = usize::from(pass.stable_arg_tally);
        if each_triangle_batch_inputs.saturating_add(each_mesh_inputs).saturating_add(stable) != pass.arguments.len() {
            return None;
        }
        let (prim_args, rest) = pass.arguments.split_at(each_triangle_batch_inputs);
        let (obj_args, stable_args) = rest.split_at(each_mesh_inputs);
        Some(Self {
            each_triangle_batch_inputs: PreparedParamSpan::new(material, prim_args),
            each_mesh_inputs: PreparedParamSpan::new(material, obj_args),
            stable: PreparedParamSpan::new(material, stable_args),
        })
    }
}

impl PreparedParamSpan {
    fn new(material: &LiveMaterial, args: &[LiveArgumentBinding]) -> Self {
        Self {
            local: bind_material_locals(material, args),
            code: args.iter().filter_map(code_arg).collect(),
        }
    }
}

fn code_arg(argument: &LiveArgumentBinding) -> Option<PackedCodeParam> {
    match *argument {
        LiveArgumentBinding::EngineConstant {
            stage,
            destination: dest,
            index,
            row_base: first_row,
            line_total,
        } => Some(PackedCodeParam::Constant {
            stage,
            slot_reg: dest,
            index,
            row_base: first_row,
            line_total,
        }),
        LiveArgumentBinding::EngineSuppliedTexture { destination: dest, index } => {
            Some(PackedCodeParam::Sampler { slot_reg: dest, index })
        }
        LiveArgumentBinding::Unknown { kind_code: kind, .. } => {
            Some(PackedCodeParam::Unknown { kind_code: kind })
        }
        LiveArgumentBinding::ShaderSamplerImage { .. }
        | LiveArgumentBinding::ShaderParameterValue { .. }
        | LiveArgumentBinding::InlineConstant { .. } => None,
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

fn seek_ascending<T: Copy>(table: &[(u32, T)], at: &mut usize, wanted: u32) -> Option<T> {
    // Remapped code samplers can ask for hashes outside the authored order.
    if table.get(*at).is_none_or(|&(hash, _)| hash > wanted) {
        *at = 0;
    }
    while let Some(&(hash, value)) = table.get(*at) {
        match hash.cmp(&wanted) {
            Ordering::Equal => return Some(value),
            Ordering::Greater => return None,
            Ordering::Less => *at += 1,
        }
    }
    None
}

fn bind_one(
    material: &LiveMaterial,
    argument: &LiveArgumentBinding,
    cursors: &mut Cursors,
) -> Option<RunnableParamSource> {
    match *argument {
        LiveArgumentBinding::ShaderSamplerImage {
            destination: dest,
            label_digest,
        } => {
            let texture = seek_ascending(&material.textures, &mut cursors.textures, label_digest)??;
            Some(RunnableParamSource::ShaderSamplerImage {
                slot_reg: dest,
                label_digest,
                texture,
            })
        }
        LiveArgumentBinding::ShaderParameterValue {
            stage,
            destination: dest,
            label_digest,
        } => {
            let words = seek_ascending(&material.constants, &mut cursors.constants, label_digest)?;
            Some(RunnableParamSource::ShaderParameterValue {
                stage,
                slot_reg: dest,
                label_digest,
                lane_bits: words,
            })
        }
        LiveArgumentBinding::InlineConstant {
            stage,
            destination: dest,
            words,
        } => Some(RunnableParamSource::InlineConstant {
            stage,
            slot_reg: dest,
            lane_bits: words?,
        }),
        LiveArgumentBinding::EngineSuppliedTexture { .. }
        | LiveArgumentBinding::EngineConstant { .. }
        | LiveArgumentBinding::Unknown { .. } => None,
    }
}

/// Argument kinds in the order the authored argument list groups them.
const AUTHORED_KIND_ORDER: [u16; 8] = [
    arg_kind::SURFACE_VERTEX_CONSTANT,
    arg_kind::INLINE_VERTEX_CONSTANT,
    arg_kind::SURFACE_FRAGMENT_SAMPLER,
    arg_kind::ENGINE_VERTEX_CONSTANT,
    arg_kind::ENGINE_FRAGMENT_SAMPLER,
    arg_kind::ENGINE_FRAGMENT_CONSTANT,
    arg_kind::SURFACE_FRAGMENT_CONSTANT,
    arg_kind::INLINE_FRAGMENT_CONSTANT,
];

fn bind_material_locals(
    material: &LiveMaterial,
    args: &[LiveArgumentBinding],
) -> Vec<Option<RunnableParamSource>> {
    let mut bound = vec![None; args.len()];
    let mut cursors = Cursors::default();
    let mut next = 0;
    for kind in AUTHORED_KIND_ORDER {
        // Vertex and pixel constants are separate tables in the source data,
        // so the constant search restarts when the pixel constants begin.
        if kind == arg_kind::SURFACE_FRAGMENT_CONSTANT {
            cursors.constants = 0;
        }
        while next < args.len() && args[next].material_argument_kind() == kind {
            bound[next] = bind_one(material, &args[next], &mut cursors);
            next += 1;
        }
    }
    for index in next..args.len() {
        bound[index] = bind_one(material, &args[index], &mut cursors);
    }
    bound
}
