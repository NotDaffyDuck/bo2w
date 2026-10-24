//! Building a material execution: pass lookup, state checks and the
//! resolution of each pass's code constants and samplers.

use std::sync::Arc;

use super::lanes::{
    PackedCodeConstantLane, PackedCodeConstants, PackedCodeSamplerLane, PackedCodeSamplers,
};
use super::refusal::MaterialRefusal;
use super::resolve::{MaterialDrawKey, missing_prepared_technique, resolve_material_technique};
use super::shell::{MaterialExecution, StableMaterialShell, StablePassShell};
use crate::prepared::{GfxPassStateBits, PackedCodeArg, PreparedArgBelts, PreparedMaterialTable};
use crate::{
    CodeSourceLookup, RuntimeMaterialCatalog, RuntimeShaderStage, SetupArm, TechType,
};

pub(super) fn check_pass_state(
    pass_index: u8,
    state: GfxPassStateBits,
) -> Result<(), MaterialRefusal> {
    match state.unsupported_host_fields() {
        None => Ok(()),
        Some(fields) => Err(MaterialRefusal::UnsupportedState {
            pass_index,
            fields,
            word0: state.word0,
            word1: state.word1,
        }),
    }
}

fn constant_lane(
    sources: &impl CodeSourceLookup,
    pass_index: u8,
    stage: RuntimeShaderStage,
    destination: u16,
    index: u16,
    first_row: u8,
    row_count: u8,
) -> Result<PackedCodeConstantLane, MaterialRefusal> {
    let Some(rows) = sources.constant_arc(index) else {
        return Err(MaterialRefusal::MissingCodeConstant {
            pass_index,
            stage,
            index,
        });
    };
    let start = usize::from(first_row);
    if rows.get(start..start.saturating_add(usize::from(row_count))).is_none() {
        return Err(MaterialRefusal::CodeConstantRowsOutOfRange {
            pass_index,
            stage,
            index,
            first_row,
            row_count,
            available_rows: rows.len(),
        });
    }
    Ok(PackedCodeConstantLane {
        stage,
        destination,
        index,
        first_row,
        row_count,
        rows,
    })
}

fn sampler_lane(
    sources: &impl CodeSourceLookup,
    pass_index: u8,
    destination: u16,
    index: u32,
) -> Result<PackedCodeSamplerLane, MaterialRefusal> {
    let Some(sampler_state) = sources.texture(index) else {
        return Err(MaterialRefusal::MissingCodeTexture { pass_index, index });
    };
    Ok(PackedCodeSamplerLane {
        register: destination,
        index,
        sampler_state,
        image: sources.texture_image(index),
    })
}

/// Refills `constants` and `samplers` with one lane per code argument of the
/// pass, in stable, per-primitive, per-object order.
pub(super) fn fill_pass_code(
    belts: &PreparedArgBelts,
    sources: &impl CodeSourceLookup,
    pass_index: u8,
    constants: &mut PackedCodeConstants,
    samplers: &mut PackedCodeSamplers,
) -> Result<(), MaterialRefusal> {
    constants.lanes.clear();
    samplers.lanes.clear();
    let arguments = belts
        .stable
        .code
        .iter()
        .chain(&belts.per_prim.code)
        .chain(&belts.per_obj.code)
        .copied();
    for argument in arguments {
        match argument {
            PackedCodeArg::Constant {
                stage,
                destination,
                index,
                first_row,
                row_count,
            } => constants.lanes.push(constant_lane(
                sources,
                pass_index,
                stage,
                destination,
                index,
                first_row,
                row_count,
            )?),
            PackedCodeArg::Sampler { destination, index } => samplers
                .lanes
                .push(sampler_lane(sources, pass_index, destination, index)?),
            PackedCodeArg::Unknown { argument_type } => {
                return Err(MaterialRefusal::UnknownArgumentType {
                    pass_index,
                    argument_type,
                });
            }
        }
    }
    samplers.rehash();
    constants.rehash();
    Ok(())
}

pub fn execute_material(
    catalog: &RuntimeMaterialCatalog,
    prepared: &PreparedMaterialTable,
    code_sources: &impl CodeSourceLookup,
    key: MaterialDrawKey,
    tech_type: TechType,
    vertex_type: u8,
) -> Result<MaterialExecution, MaterialRefusal> {
    execute_material_with_shell(catalog, prepared, code_sources, key, tech_type, vertex_type)
}

pub fn execute_material_with_shell(
    catalog: &RuntimeMaterialCatalog,
    prepared: &PreparedMaterialTable,
    code_sources: &impl CodeSourceLookup,
    key: MaterialDrawKey,
    tech_type: TechType,
    vertex_type: u8,
) -> Result<MaterialExecution, MaterialRefusal> {
    let (material, technique) = resolve_material_technique(catalog, key, tech_type)?;
    let Some(prepared_tech) = prepared.technique(material.asset_id, tech_type) else {
        return Err(missing_prepared_technique(material, technique, tech_type));
    };

    let pass_total = technique.passes.len();
    let mut passes = Vec::with_capacity(pass_total);
    let mut code_constants = Vec::with_capacity(pass_total);
    let mut code_samplers = Vec::with_capacity(pass_total);

    for (index, pass) in technique.passes.iter().enumerate() {
        let pass_index = u8::try_from(index)
            .map_err(|_| MaterialRefusal::PassIndexOverflow { pass_index: index })?;
        let pair = pass
            .shader_pair
            .ok_or(MaterialRefusal::ShaderProgramMissing { pass_index })?;
        let unsupported_pair = MaterialRefusal::UnsupportedShaderPair { pass_index, pair };
        let prepared_pass = prepared_tech
            .passes
            .get(index)
            .ok_or_else(|| unsupported_pair.clone())?;
        let port = prepared_pass
            .port
            .get(usize::from(vertex_type))
            .copied()
            .flatten()
            .ok_or(unsupported_pair)?;
        check_pass_state(pass_index, prepared_pass.state)?;
        let Some(belts) = prepared_pass.belts.as_ref() else {
            return Err(MaterialRefusal::PassArgCountMismatch {
                pass_index,
                per_prim: pass.per_prim_arg_count,
                per_obj: pass.per_obj_arg_count,
                stable: pass.stable_arg_count,
                arguments: pass.arguments.len(),
            });
        };

        let mut constants = PackedCodeConstants::default();
        let mut samplers = PackedCodeSamplers::default();
        fill_pass_code(belts, code_sources, pass_index, &mut constants, &mut samplers)?;
        code_constants.push(constants);
        code_samplers.push(samplers);
        passes.push(StablePassShell {
            pass_index,
            port,
            shader_pair: pair,
            state: prepared_pass.state,
            local_banks: prepared_pass
                .local_banks
                .get(usize::from(vertex_type))
                .cloned()
                .flatten(),
            local_samplers: prepared_pass.local_samplers.clone(),
            belts: Arc::new(belts.clone()),
        });
    }

    Ok(MaterialExecution::from_parts(
        StableMaterialShell {
            tech_type,
            arm: SetupArm::classify(tech_type),
            vertex_type,
            passes,
        },
        code_constants,
        code_samplers,
    ))
}
