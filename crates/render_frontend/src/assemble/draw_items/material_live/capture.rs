//! Turning the authored material definitions into the runtime catalog: the
//! technique sets with their passes, the materials, the shader programs and
//! the sorted-material table.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use asset_material::DesignerMaterial;
use asset_material::HandBuiltShader;
use asset_material::TintMapRemap;
use asset_material::FaceCullingMode;
use asset_material::MaterialDefinitions;
use asset_material::SelfContainedShaderPass;
use asset_material::HeldShaderArg;
use asset_material::SelfContainedShaderLink;
use asset_material::RecipeBundleSummary;
use asset_material::RecipeBundleId;
use asset_material::RecipeBundleLookup;

use super::sort_table::{SortedBuild, build_sorted_material_table};
use super::{
    MaterialAssetKey, MaterialEpochId, MaterialRemapOutcome, LiveArgumentBinding, LiveImageId,
    LiveMaterial, LiveMaterialLedger, LivePassRecord, LiveShaderPair, ActiveShaderCombo,
    LiveShaderProgramId, LiveShaderStage, LiveSortedMaterialTable, LiveShadingTechnique,
    LiveTechniqueBundle, LiveTechniqueBundleKey, LiveTextureBinding, LiveVertexFormat,
    RENDER_RECIPE_SLOT_TOTAL, sort_pass_args,
};
use crate::assemble::draw_items::pass_colour_space::pass_color_space;

/// Vertex declarations met so far, `(slot, declaration)`, sorted by slot.

/// A number no other catalog captured in this run has. It is an identity, not
/// a checksum: two captures can agree on every name and draw-surf key and
/// still differ in shaders, images or map overrides.
fn mint_material_generation_id() -> MaterialEpochId {
    static ISSUED: AtomicU64 = AtomicU64::new(1);
    MaterialEpochId(ISSUED.fetch_add(1, Ordering::Relaxed))
}

/// The content half of a shader's runtime id: 64-bit FNV-1a over its bytes.
fn content_hash(bytes: &[u8]) -> u64 {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const MULTIPLIER: u64 = 0x0000_0100_0000_01b3;
    let mut state = BASIS;
    for &b in bytes {
        state = (state ^ u64::from(b)).wrapping_mul(MULTIPLIER);
    }
    state
}

/// A catalog position as the runtime's `u32` slot; `u32::MAX` stands for "none".
fn slot_or_none(index: Option<usize>) -> u32 {
    index.and_then(|at| u32::try_from(at).ok()).unwrap_or(u32::MAX)
}

// ---------------------------------------------------------------------
// Passes
// ---------------------------------------------------------------------

/// The runtime form of one authored shader argument. Each kind keeps its
/// fields; the vertex and pixel variants of a kind become one binding with
/// a stage.
fn bind_argument(argument: &HeldShaderArg) -> LiveArgumentBinding {
    use LiveShaderStage::{Pixel, Vertex};

    match *argument {
        HeldShaderArg::MaterialVertConst { slot: dest, label_digest } => LiveArgumentBinding::ShaderParameterValue { stage: Vertex, destination: dest, label_digest },
        HeldShaderArg::OwnPixelConstant { slot: target_dst, label_digest } => LiveArgumentBinding::ShaderParameterValue { stage: Pixel, destination: target_dst, label_digest },
        HeldShaderArg::FixedVertexConstant { slot: into, bit_words: words } => LiveArgumentBinding::InlineConstant { stage: Vertex, destination: into, words },
        HeldShaderArg::FixedPixelConstant { slot: dst, words: word_list } => LiveArgumentBinding::InlineConstant { stage: Pixel, destination: dst, words: word_list },
        HeldShaderArg::OwnPixelSampler { slot: dest, label_digest } => LiveArgumentBinding::ShaderSamplerImage { destination: dest, label_digest },
        HeldShaderArg::EngineVertexConstant { slot: target_dst, index, start_row: first_row, line_total } => {
            LiveArgumentBinding::EngineConstant { stage: Vertex, destination: target_dst, index, row_base: first_row, line_total }
        }
        HeldShaderArg::EnginePixelConstant { into: dest, const_slot: index, row_first: first_row, line_total } => {
            LiveArgumentBinding::EngineConstant { stage: Pixel, destination: dest, index, row_base: first_row, line_total }
        }
        HeldShaderArg::EnginePixelSampler { slot: target_dst, index } => LiveArgumentBinding::EngineSuppliedTexture { destination: target_dst, index },
        HeldShaderArg::Unknown { param_type: arg_type, raw } => LiveArgumentBinding::Unknown { kind_code: arg_type, raw },
    }
}

/// The runtime id of the shader a pass reference names, if it names one the
/// corpus has.
fn shader_id(source: &MaterialDefinitions, reference: &SelfContainedShaderLink) -> Option<LiveShaderProgramId> {
    let at = reference.shader_slot?;
    let bytes = &source.shaders.get(at)?.bytecode;
    Some(LiveShaderProgramId { asset_table_position: u32::try_from(at).ok()?, program_digest: content_hash(bytes) })
}

/// The vertex declaration slot a pass names, and the declaration itself when
/// the corpus has it.
fn vertex_decl_of(source: &MaterialDefinitions, authored: &SelfContainedShaderPass) -> (u32, Option<LiveVertexFormat>) {
    let named = authored.vertex_shape;
    let decl = named.and_then(|at| source.vertex_decls.get(at)).map(|d| LiveVertexFormat {
        name: d.name.to_string(),
        family: d.family,
        routing: d.mix_paths,
        stream_tally: d.stream_tally,
        has_spare_source: d.has_spare_source,
    });
    (slot_or_none(named), decl)
}

/// Remember `decl` under `slot`, keeping the list sorted by slot. The first
/// declaration seen for a slot stays.
fn remember_vertex_decl(decls: &mut Vec<(u32, LiveVertexFormat)>, slot: u32, decl: LiveVertexFormat) {
    let at = decls.partition_point(|(known, _)| *known < slot);
    let taken = decls.get(at).is_some_and(|(known, _)| *known == slot);
    if !taken {
        decls.insert(at, (slot, decl));
    }
}

/// One authored pass as the runtime draws it: its shader pair (only when both
/// halves resolve), its counts and flags, its arguments in sorted order, and
/// the colour space its technique slot draws in.
fn capture_pass(
    source: &MaterialDefinitions,
    facts: &RecipeBundleSummary,
    seat: usize,
    authored: &SelfContainedShaderPass,
    decls: &mut Vec<(u32, LiveVertexFormat)>,
) -> LivePassRecord {
    let (decl_slot, decl) = vertex_decl_of(source, authored);
    if let Some(found) = decl {
        remember_vertex_decl(decls, decl_slot, found);
    }
    let halves = shader_id(source, &authored.vertex_shader).zip(shader_id(source, &authored.pixel_shader));
    let mut pass = LivePassRecord {
        stage_program_duo: halves.map(|(vertex, pixel)| LiveShaderPair { vertex, pixel, vertex_format_slot: decl_slot }),
        arguments: authored.arguments.iter().map(bind_argument).collect(),
        color_space: pass_color_space(facts.namespace, seat as u8),
        stable_arg_tally: authored.stable_arg_tally,
        per_obj_arg_tally: authored.per_obj_arg_tally,
        per_prim_arg_tally: authored.per_prim_arg_tally,
        sampler_custom_bits: authored.own_sampler_bits,
        classic_custom_sampler_flags: authored.classic_custom_sampler_flags,
    };
    sort_pass_args(&mut pass);
    pass
}

/// One technique set: each of its `RENDER_RECIPE_SLOT_TOTAL` slots holds the
/// technique the graph puts there, if any. Slots past the runtime's count
/// are dropped.
fn capture_set(source: &MaterialDefinitions, scene_facts: &RecipeBundleSummary, decls: &mut Vec<(u32, LiveVertexFormat)>) -> LiveTechniqueBundle {
    let graph = scene_facts.table.as_ref().and_then(|table| table.graph.as_ref());
    let authored = graph.map_or(&[][..], |g| &g.slots[..]);
    let slots = (0..RENDER_RECIPE_SLOT_TOTAL)
        .map(|seat| {
            let tech_ref = authored.get(seat)?.as_ref()?;
            let passes = tech_ref.passes.iter().map(|p| capture_pass(source, scene_facts, seat, p, decls)).collect();
            Some(LiveShadingTechnique { flags: tech_ref.bits, passes })
        })
        .collect();
    let name = scene_facts.name.to_string();
    LiveTechniqueBundle::new(slots)
        .expect("slots was built TECHNIQUE_SLOT_COUNT long")
        .with_identity(scene_facts.level_vertex_format, scene_facts.namespace, name)
}

/// Every technique set in corpus order, and the vertex declarations their
/// passes name.
fn capture_technique_sets(source: &MaterialDefinitions) -> (Vec<LiveTechniqueBundle>, Vec<(u32, LiveVertexFormat)>) {
    let mut decls = Vec::<(u32, LiveVertexFormat)>::new();
    let sets = source.method_set_facts().iter().map(|facts| capture_set(source, facts, &mut decls)).collect();
    (sets, decls)
}

// ---------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------

/// Four floats kept as their exact bit patterns.
fn float_bits(words: [f32; 4]) -> [u32; 4] {
    words.map(f32::to_bits)
}

/// The draw-surf key a material is baked with, once it has an ordinal.
fn baked_draw_surf_key(authored: &DesignerMaterial, band: u16) -> u64 {
    let techniques = authored.method_table.as_ref();
    let has_slot = |bit: u64| techniques.is_some_and(|t| t.cells & bit != 0);
    let input = dpvs_t6::MaterialPrepassInput {
        technique0_present: has_slot(1),
        has_second_pass: has_slot(2),
        technique0_flags: techniques.map_or(0, |t| t.technique0_flags),
        condition_bits: authored.condition_bits,
        techset_name_has_flag: authored.method_set.as_str().contains("flag_"),
    };
    dpvs_t6::material_order_key(authored.draw_order, dpvs_t6::prepass_flag_of(input), band).packed
}

/// The runtime cull code: 0 drops back faces, 1 front faces, none keeps both.
fn cull_mode_of(face: Option<FaceCullingMode>) -> Option<u8> {
    match face? {
        FaceCullingMode::Back => Some(0),
        FaceCullingMode::Front => Some(1),
        FaceCullingMode::None => None,
    }
}

/// The material's texture slots by name hash, ascending; a slot whose image
/// is missing (or does not fit a runtime id) is kept, bound to nothing.
fn texture_bindings(authored: &DesignerMaterial) -> Vec<(u32, Option<LiveTextureBinding>)> {
    let mut rows = Vec::with_capacity(authored.textures.len());
    for tex in &authored.textures {
        let image = tex.image.and_then(|at| u32::try_from(at).ok()).map(LiveImageId);
        let bound = image.map(|image| LiveTextureBinding { image, texture_sampling_mode: tex.texture_sampling_mode, semantic: tex.semantic });
        rows.push((tex.label_digest, bound));
    }
    rows.sort_by_key(|row| row.0);
    rows
}

/// The material's literal constants by name hash, ascending, as raw bits.
fn constant_bits(authored: &DesignerMaterial) -> Vec<(u32, [u32; 4])> {
    let mut rows: Vec<_> = authored.constants.iter().map(|c| (c.label_digest, float_bits(c.literal))).collect();
    rows.sort_by_key(|row| row.0);
    rows
}

fn capture_material(
    source: &MaterialDefinitions,
    sorted: &LiveSortedMaterialTable,
    index: usize,
    from_author: &DesignerMaterial,
) -> LiveMaterial {
    let techset = from_author.method_set.as_str();
    let resolved = match source.resolve_method_set(RecipeBundleId::new(from_author.namespace, techset)) {
        RecipeBundleLookup::Hit { graph_slot: found, .. } => Some(found),
        _ => None,
    };
    let id = u16::try_from(index).expect("material catalog index exceeds the u16 asset-id domain");
    let [uv_scroll, attenuation_terms, attenuation_start, attenuation_stop] = MaterialDefinitions::material_animation(from_author);
    let env_map = from_author.constants.iter().find(|c| c.name.starts_with(b"envMapParms"));
    let baked = sorted.rank_for_asset_id(index).map(|ordinal| baked_draw_surf_key(from_author, ordinal.sort_band()));
    let lit_by_models = from_author.route.is_some_and(|route| route.uses_model_illum_const);
    LiveMaterial {
        asset_id: MaterialAssetKey(id),
        name: from_author.name.to_string(),
        namespace: from_author.namespace,
        method_set: techset.to_string(),
        baked_facet_draw: baked,
        local_technique_set: LiveTechniqueBundleKey(slot_or_none(resolved)),
        remap: MaterialRemapOutcome::SelfSet,
        state_row_for: from_author.raster_slot,
        state_rows: from_author.raster_bits.clone(),
        camera_zone: from_author.camera_zone,
        draw_sort_key: from_author.draw_order,
        game_flag_info: from_author.game_flag_info,
        condition_bits: from_author.condition_bits,
        facet_type_bits: from_author.facet_type_bits,
        unlit: source.draws_flat(from_author) == Some(true),
        uses_model_illum_const: lit_by_models,
        colour_map_is_square: source.tint_map_remap(from_author) == TintMapRemap::Square,
        occluder_exclusive: source.casts_shadow_only(from_author),
        t6_draw: from_author.t6_draw,
        cull_mode: cull_mode_of(source.face_culling(from_author)),
        uv_scroll_bits: float_bits(uv_scroll),
        attenuation_terms_raw: float_bits(attenuation_terms),
        attenuation_start_raw: float_bits(attenuation_start),
        attenuation_stop_raw: float_bits(attenuation_stop),
        reflection_map_param_bits: env_map
            .map_or([0; 4], |c| float_bits(c.literal)),
        sheet_bits: {
            let c = |name: &[u8], default: f32| {
                let constants = &from_author.constants;
                constants
                    .iter()
                    .find(|c| c.name.starts_with(name))
                    .map_or([default; 4], |c| c.literal)
            };
            let speed = c(b"ColorSpeed", 0.0);
            let hdr = c(b"hdrAmount", 1.0)[0];
            [
                1.0 / c(b"FadeFactor", 0.0)[0].max(1e-4),
                speed[0],
                speed[1],
                c(b"ColorScale", 1.0)[0],
                hdr * hdr,
            ]
            .map(|word| word.to_bits())
        },
        textures: texture_bindings(from_author),
        constants: constant_bits(from_author),
    }
}

/// The index of the first material of each name, sorted by name.
fn first_index_by_name<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut first = BTreeMap::new();
    for (index, name) in names.enumerate() {
        first.entry(name.to_owned()).or_insert(index);
    }
    first.into_iter().collect()
}

// ---------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------

/// A corpus shader as a runtime program, keyed by its slot and content.
/// Shaders with no bytes are left out (`None`).
fn capture_shader(slot: usize, from_author: &HandBuiltShader) -> Option<ActiveShaderCombo> {
    use LiveShaderStage::{Pixel, Vertex};
    let asset_table_position = u32::try_from(slot).expect("shader catalog index exceeds the runtime u32 asset-slot domain");
    if from_author.bytecode.is_empty() {
        return None;
    }
    let program = from_author.bytecode.clone();
    Some(ActiveShaderCombo {
        id: LiveShaderProgramId { asset_table_position, program_digest: content_hash(&program) },
        stage: if from_author.is_corner_stage() { Vertex } else { Pixel },
        name: from_author.name.to_string(),
        program,
    })
}

/// The sorted-material table, or the reason it could not be built.
fn sorted_table_of(built: &Result<SortedBuild, super::LedgerBuildError>) -> LiveSortedMaterialTable {
    let table = match built {
        Ok(table) => table,
        Err(cause) => return LiveSortedMaterialTable::BuildFailed(cause.clone()),
    };
    LiveSortedMaterialTable::Ready {
        asset_ids_by_ordinal: table.asset_ids_by_ordinal.clone(),
        ranks_by_asset_id: table.ranks_by_asset_id.clone(),
        skipped_total: table.skipped_n,
        gap_total: table.slot_gap_n,
    }
}

pub fn capture_runtime_catalog(source: &MaterialDefinitions) -> LiveMaterialLedger {
    let (tech_sets, vert_decls) = capture_technique_sets(source);

    let built = build_sorted_material_table(source);
    let sorted = sorted_table_of(&built);
    // Where the sort first skipped a material, and why; nothing when it failed.
    let (first_skip, skip_tech, skip_reason) = built.map_or((None, None, None), |b| (b.first_skip, b.skipped_tech, b.skipped_cause));

    let mut materials = Vec::with_capacity(source.materials.len());
    for (index, authored) in source.materials.iter().enumerate() {
        materials.push(capture_material(source, &sorted, index, authored));
    }
    let by_name = first_index_by_name(materials.iter().map(|m| m.name.as_str()));
    let programs = source.shaders.iter().enumerate().map(|(slot, s)| capture_shader(slot, s)).collect();

    LiveMaterialLedger {
        revision_stamp: mint_material_generation_id(),
        material_slots_by_label: by_name,
        shader_set: programs,
        ranked_materials: sorted,
        first_ordered_skip_reason: first_skip,
        materials,
        technique_sets: tech_sets,
        decl_rows: vert_decls,
        dropped_recipes: skip_tech,
        drop_reasons: skip_reason,
    }
}

#[cfg(test)]
mod tests {
    use asset_material::ShaderStageVariety;

    use super::*;

    fn shader(name: &str, kind: ShaderStageVariety, bytes: &[u8]) -> HandBuiltShader {
        let name = asset_core::AssetPointer::decode(name);
        HandBuiltShader { namespace: asset_core::AssetCategoryScope::T5, name, kind, bytecode: bytes.to_vec() }
    }

    fn names(shader: Option<usize>) -> SelfContainedShaderLink {
        SelfContainedShaderLink { shader_slot: shader }
    }

    #[test]
    fn the_content_hash_is_64_bit_fnv_1a() {
        assert_eq!(content_hash(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(content_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(content_hash(b"ab"), content_hash(b"ba"));
    }

    #[test]
    fn generation_ids_are_never_reused() {
        let earlier = mint_material_generation_id();
        let later = mint_material_generation_id();
        assert!(later.0 > earlier.0);
    }

    #[test]
    fn every_argument_kind_binds_to_its_runtime_form() {
        use LiveShaderStage::{Pixel, Vertex};

        let quad = Some([1, 2, 3, 4]);
        let top_row = 0;
        let cases = [
            (HeldShaderArg::MaterialVertConst { slot: 1, label_digest: 10 }, LiveArgumentBinding::ShaderParameterValue { stage: Vertex, destination: 1, label_digest: 10 }),
            (HeldShaderArg::OwnPixelConstant { slot: 2, label_digest: 11 }, LiveArgumentBinding::ShaderParameterValue { stage: Pixel, destination: 2, label_digest: 11 }),
            (HeldShaderArg::FixedVertexConstant { slot: 3, bit_words: quad }, LiveArgumentBinding::InlineConstant { stage: Vertex, destination: 3, words: quad }),
            (HeldShaderArg::FixedPixelConstant { slot: 4, words: None }, LiveArgumentBinding::InlineConstant { stage: Pixel, destination: 4, words: None }),
            (HeldShaderArg::OwnPixelSampler { slot: 5, label_digest: 12 }, LiveArgumentBinding::ShaderSamplerImage { destination: 5, label_digest: 12 }),
            (
                HeldShaderArg::EngineVertexConstant { slot: 6, index: 7, start_row: 1, line_total: 2 },
                LiveArgumentBinding::EngineConstant { stage: Vertex, destination: 6, index: 7, row_base: 1, line_total: 2 },
            ),
            (
                HeldShaderArg::EnginePixelConstant { into: 8, const_slot: 9, row_first: top_row, line_total: 4 },
                LiveArgumentBinding::EngineConstant { stage: Pixel, destination: 8, index: 9, row_base: top_row, line_total: 4 },
            ),
            (HeldShaderArg::EnginePixelSampler { slot: 3, index: 40 }, LiveArgumentBinding::EngineSuppliedTexture { destination: 3, index: 40 }),
            (HeldShaderArg::Unknown { param_type: 77, raw: [9; 8] }, LiveArgumentBinding::Unknown { kind_code: 77, raw: [9; 8] }),
        ];
        for (authored, expected) in cases {
            assert_eq!(bind_argument(&authored), expected, "{authored:?}");
        }
    }

    #[test]
    fn a_shader_is_named_by_slot_and_content() {
        let mut source = MaterialDefinitions::default();
        source.shaders.push(shader("vs", ShaderStageVariety::Vertex, b"abc"));
        let expected = LiveShaderProgramId { asset_table_position: 0, program_digest: content_hash(b"abc") };
        assert_eq!(shader_id(&source, &names(Some(0))), Some(expected));
        // No shader named, or one the corpus lacks.
        assert_eq!(shader_id(&source, &names(None)), None);
        assert_eq!(shader_id(&source, &names(Some(5))), None);
    }

    #[test]
    fn shader_programs_keep_their_stage_and_empty_ones_drop_out() {
        let vs = capture_shader(3, &shader("vs", ShaderStageVariety::Vertex, b"xy")).expect("has bytes");
        assert_eq!((vs.stage, vs.id.asset_table_position), (LiveShaderStage::Vertex, 3));
        assert_eq!(vs.id.program_digest, content_hash(b"xy"));
        assert_eq!((vs.name.as_str(), vs.program.as_slice()), ("vs", &b"xy"[..]));
        let ps = capture_shader(4, &shader("ps", ShaderStageVariety::Pixel, b"z")).expect("has bytes");
        assert_eq!(ps.stage, LiveShaderStage::Pixel);
        assert!(capture_shader(5, &shader("empty", ShaderStageVariety::Pixel, b"")).is_none());
    }

    fn decl(label: &str) -> LiveVertexFormat {
        let (family, routing) = Default::default();
        LiveVertexFormat { name: label.into(), family, routing, stream_tally: 1, has_spare_source: 0 }
    }

    #[test]
    fn vertex_declarations_stay_sorted_by_slot_and_the_first_one_wins() {
        let mut decls = Vec::<(u32, LiveVertexFormat)>::new();
        for (slot, name) in [(5, "five"), (2, "two"), (9, "nine"), (5, "again")] {
            remember_vertex_decl(&mut decls, slot, decl(name));
        }
        let kept: Vec<_> = decls.iter().map(|(slot, d)| (*slot, d.name.as_str())).collect();
        assert_eq!(kept, [(2, "two"), (5, "five"), (9, "nine")]);
    }

    #[test]
    fn cull_faces_become_the_runtime_codes() {
        let faces = [Some(FaceCullingMode::Back), Some(FaceCullingMode::Front), Some(FaceCullingMode::None), None];
        assert_eq!(faces.map(cull_mode_of), [Some(0), Some(1), None, None]);
    }

    #[test]
    fn a_name_points_at_its_first_material() {
        let found = first_index_by_name(["wall", "floor", "wall"].into_iter());
        assert_eq!(found, [("floor".to_owned(), 1), ("wall".to_owned(), 0)]);
        assert!(first_index_by_name(std::iter::empty()).is_empty());
    }

    #[test]
    fn a_failed_sort_is_kept_as_the_failure() {
        let failure = super::super::LedgerBuildError::MaterialAssetNumberTooLarge { index: 70_000 };
        let table = sorted_table_of(&Err(failure.clone()));
        assert_eq!(table, LiveSortedMaterialTable::BuildFailed(failure));
    }

    #[test]
    fn an_empty_corpus_captures_an_empty_catalog() {
        let empty = capture_runtime_catalog(&MaterialDefinitions::default());
        assert!(empty.materials.is_empty() && empty.technique_sets.is_empty());
        assert!(empty.shader_set.is_empty() && empty.decl_rows.is_empty());
        assert!(empty.material_slots_by_label.is_empty());
        assert_eq!(empty.first_ordered_skip_reason, None);
        assert!(matches!(empty.ranked_materials, LiveSortedMaterialTable::Ready { skipped_total: 0, .. }));
        let again = capture_runtime_catalog(&MaterialDefinitions::default());
        assert_ne!(empty.revision_stamp, again.revision_stamp);
    }

    #[test]
    fn shaders_in_the_corpus_reach_the_catalog_in_slot_order() {
        let mut source = MaterialDefinitions::default();
        let corpus = [("a", ShaderStageVariety::Vertex, &b"1"[..]), ("b", ShaderStageVariety::Pixel, b""), ("c", ShaderStageVariety::Pixel, b"3")];
        for (name, kind, bytes) in corpus {
            source.shaders.push(shader(name, kind, bytes));
        }
        let catalog = capture_runtime_catalog(&source);
        let present: Vec<_> = catalog.shader_set.iter().map(|p| p.as_ref().map(|p| p.name.as_str())).collect();
        assert_eq!(present, [Some("a"), None, Some("c")]);
    }
}
