//! Building a material execution: pass lookup, state checks and the
//! resolution of each pass's code constants and samplers.

use std::sync::Arc;

use super::lanes::{CodeConstantChannel, CodeSamplerChannel};
use super::refusal::MaterialExecutionDenial;
use super::resolve::{SurfaceSortKeyPair, missing_prepared_technique, select_material_technique};
use super::shell::{CodeLanes, MaterialRunPlan, MaterialPlan, PassPlan};
use crate::prepared::{VisualPassStateBits, PackedCodeParam, PreparedParamStrips, StagedMaterialTable, ResolvedDrawPass};
use crate::{EngineValueSource, LiveMaterialLedger, LivePassRecord, SetupVariantBranch, MethodType};

/// Refuses a pass whose blend state the host cannot reproduce.
pub(super) fn check_pass_state(
    technique_pass_ordinal: u8,
    state: VisualPassStateBits,
) -> Result<(), MaterialExecutionDenial> {
    let Some(fields) = state.unsupported_host_fields() else {
        return Ok(());
    };
    Err(MaterialExecutionDenial::UnhandledRenderState {
        technique_pass_ordinal,
        fields,
        dword0: state.dword0,
        dword1: state.dword1,
    })
}

/// Resolves the code arguments of one pass against the live sources.
struct LaneResolver<'a, S> {
    sources: &'a S,
    technique_pass_ordinal: u8,
}

impl<S: EngineValueSource> LaneResolver<'_, S> {
    fn resolve_lane(
        &self,
        param: PackedCodeParam,
        lanes: &mut CodeLanes,
    ) -> Result<(), MaterialExecutionDenial> {
        let technique_pass_ordinal = self.technique_pass_ordinal;
        match param {
            PackedCodeParam::Constant {
                stage,
                slot_reg: dest,
                index,
                row_base: first_row,
                line_total,
            } => {
                let table_rows = self
                    .sources
                    .shared_constants_for(index)
                    .ok_or(MaterialExecutionDenial::EngineConstantUnavailable {
                        technique_pass_ordinal,
                        stage,
                        index,
                    })?;
                let first = usize::from(first_row);
                let end = first.saturating_add(usize::from(line_total));
                if end > table_rows.len() {
                    return Err(MaterialExecutionDenial::EngineConstantRowsBeyondTable {
                        technique_pass_ordinal,
                        stage,
                        index,
                        row_base: first_row,
                        line_total,
                        rows_remaining: table_rows.len(),
                    });
                }
                lanes.constants.lanes.push(CodeConstantChannel {
                    stage,
                    slot_reg: dest,
                    index,
                    row_base: first_row,
                    line_total,
                    row_data: table_rows,
                });
            }
            PackedCodeParam::Sampler { slot_reg: dest, index: tex } => {
                let texture_sampling_mode = self
                    .sources
                    .sampler_mode_of(tex)
                    .ok_or(MaterialExecutionDenial::EngineTextureUnavailable { technique_pass_ordinal, index: tex })?;
                lanes.samplers.lanes.push(CodeSamplerChannel {
                    sampler_reg: dest,
                    index: tex,
                    texture_sampling_mode,
                    image: self.sources.image_for(tex),
                });
            }
            PackedCodeParam::Unknown { kind_code: kind } => {
                return Err(MaterialExecutionDenial::UnrecognisedArgumentKind {
                    technique_pass_ordinal,
                    kind_code: kind,
                });
            }
        }
        Ok(())
    }
}

/// Refills `lanes` with one entry per code argument of the pass: stable
/// arguments first, then per-primitive, then per-object.
pub(super) fn fill_pass_code(
    belts: &PreparedParamStrips,
    sources: &impl EngineValueSource,
    technique_pass_ordinal: u8,
    lanes: &mut CodeLanes,
) -> Result<(), MaterialExecutionDenial> {
    lanes.constants.lanes.clear();
    lanes.samplers.lanes.clear();
    let resolver = LaneResolver {
        sources,
        technique_pass_ordinal,
    };
    for &argument in [&belts.stable, &belts.each_triangle_batch_inputs, &belts.each_mesh_inputs]
        .into_iter()
        .flat_map(|belt| belt.code.iter())
    {
        resolver.resolve_lane(argument, lanes)?;
    }
    lanes.samplers.rehash();
    lanes.constants.rehash();
    Ok(())
}

/// One pass of the technique, resolved for `vertex_format_code`, with its lanes.
fn plan_pass(
    index: usize,
    pass: &LivePassRecord,
    prepared: Option<&ResolvedDrawPass>,
    vertex_format_code: u8,
    sources: &impl EngineValueSource,
) -> Result<(PassPlan, CodeLanes), MaterialExecutionDenial> {
    let technique_pass_ordinal = u8::try_from(index)
        .map_err(|_| MaterialExecutionDenial::PassNumberTooLarge { technique_pass_ordinal: index })?;
    let shader_pair = pass
        .stage_program_duo
        .ok_or(MaterialExecutionDenial::ShaderProgramAbsent { technique_pass_ordinal })?;
    let unsupported = MaterialExecutionDenial::UnhandledShaderCombination { technique_pass_ordinal, pair: shader_pair };
    let prepared = prepared.ok_or_else(|| unsupported.clone())?;
    let port = prepared.port(vertex_format_code).ok_or(unsupported)?;
    check_pass_state(technique_pass_ordinal, prepared.state)?;
    let belts = prepared
        .belts
        .as_ref()
        .ok_or_else(|| MaterialExecutionDenial::PassArgumentTotalWrong {
            technique_pass_ordinal,
            each_triangle_batch_inputs: pass.per_prim_arg_tally,
            each_mesh_inputs: pass.per_obj_arg_tally,
            stable: pass.stable_arg_tally,
            arg_total: Vec::len(&pass.arguments),
        })?;

    let mut lanes = CodeLanes::default();
    fill_pass_code(belts, sources, technique_pass_ordinal, &mut lanes)?;
    let plan = PassPlan {
        technique_pass_ordinal,
        port,
        stage_program_duo: shader_pair,
        state: prepared.state,
        local_banks: prepared.local_banks(vertex_format_code).cloned(),
        local_samplers: prepared.local_samplers.clone(),
        belts: Arc::new(belts.clone()),
    };
    Ok((plan, lanes))
}

pub fn run_material_pipeline(
    catalog: &LiveMaterialLedger,
    prepared: &StagedMaterialTable,
    engine_sources: &impl EngineValueSource,
    key: SurfaceSortKeyPair,
    shading_technique_kind: MethodType,
    vertex_format_code: u8,
) -> Result<MaterialRunPlan, MaterialExecutionDenial> {
    let (picked, technique) = select_material_technique(catalog, key, shading_technique_kind)?;
    let Some(prepared_tech) = prepared.recipe_for(picked.asset_id, shading_technique_kind) else {
        return Err(missing_prepared_technique(picked, technique, shading_technique_kind));
    };

    let (passes, lanes): (Vec<_>, Vec<_>) = technique
        .passes
        .iter()
        .enumerate()
        .map(|(index, pass)| {
            plan_pass(
                index,
                pass,
                prepared_tech.passes.get(index),
                vertex_format_code,
                engine_sources,
            )
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .unzip();
    let plan = MaterialPlan {
        shading_technique_kind,
        arm: SetupVariantBranch::classify(shading_technique_kind),
        vertex_format_code,
        passes,
    };
    Ok(MaterialRunPlan::from_parts(plan, lanes))
}
