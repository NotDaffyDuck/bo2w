//! Turning the authored material definitions into the runtime catalog: the
//! technique sets with their passes, the materials, the shader programs and
//! the sorted-material table.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use asset_material::{
    AuthoredMaterial, AuthoredShader, ColorMapTransform, MaterialCullFace, MaterialDefinitions,
    OwnedMaterialPass, OwnedShaderArgument, OwnedShaderRef, TechniqueSetFacts, TechsetKey,
    TechsetResolve,
};

use super::sort_table::{SortedBuild, build_sorted_material_table};
use super::{
    MaterialAssetId, MaterialGenerationId, RemapResolution, RuntimeArgumentBinding, RuntimeImageId,
    RuntimeMaterial, RuntimeMaterialCatalog, RuntimePass, RuntimeShaderPair, RuntimeShaderProgram,
    RuntimeShaderProgramId, RuntimeShaderStage, RuntimeSortedMaterialTable, RuntimeTechnique,
    RuntimeTechniqueSet, RuntimeTechniqueSetId, RuntimeTextureBinding, RuntimeVertexDecl,
    TECHNIQUE_SLOT_COUNT, sort_pass_args,
};
use crate::assemble::drawsurf::pass_color_space::pass_color_space;

/// A number no other catalog captured in this run has. It is an identity, not
/// a checksum: two captures can agree on every name and draw-surf key and
/// still differ in shaders, images or map overrides.
fn mint_material_generation_id() -> MaterialGenerationId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    MaterialGenerationId(NEXT.fetch_add(1, Ordering::Relaxed))
}

/// FNV-1a over the program bytes.
fn program_hash(program: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    program.iter().fold(OFFSET, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

// ---------------------------------------------------------------------
// Passes
// ---------------------------------------------------------------------

fn bind_argument(argument: &OwnedShaderArgument) -> RuntimeArgumentBinding {
    use OwnedShaderArgument as Arg;
    use RuntimeShaderStage::{Pixel, Vertex};

    match *argument {
        Arg::MaterialVertexConstant {
            destination,
            name_hash,
        } => RuntimeArgumentBinding::MaterialConstant {
            stage: Vertex,
            destination,
            name_hash,
        },
        Arg::MaterialPixelConstant {
            destination,
            name_hash,
        } => RuntimeArgumentBinding::MaterialConstant {
            stage: Pixel,
            destination,
            name_hash,
        },
        Arg::LiteralVertexConstant { destination, words } => {
            RuntimeArgumentBinding::LiteralConstant {
                stage: Vertex,
                destination,
                words,
            }
        }
        Arg::LiteralPixelConstant { destination, words } => {
            RuntimeArgumentBinding::LiteralConstant {
                stage: Pixel,
                destination,
                words,
            }
        }
        Arg::MaterialPixelSampler {
            destination,
            name_hash,
        } => RuntimeArgumentBinding::MaterialTexture {
            destination,
            name_hash,
        },
        Arg::CodeVertexConstant {
            destination,
            index,
            first_row,
            row_count,
        } => RuntimeArgumentBinding::CodeConstant {
            stage: Vertex,
            destination,
            index,
            first_row,
            row_count,
        },
        Arg::CodePixelConstant {
            destination,
            index,
            first_row,
            row_count,
        } => RuntimeArgumentBinding::CodeConstant {
            stage: Pixel,
            destination,
            index,
            first_row,
            row_count,
        },
        Arg::CodePixelSampler { destination, index } => {
            RuntimeArgumentBinding::CodeTexture { destination, index }
        }
        Arg::Unknown { argument_type, raw } => {
            RuntimeArgumentBinding::Unknown { argument_type, raw }
        }
    }
}

/// The runtime id of the shader a pass reference names, if it names one the
/// corpus has.
fn shader_id(
    source: &MaterialDefinitions,
    reference: &OwnedShaderRef,
) -> Option<RuntimeShaderProgramId> {
    let index = reference.shader?;
    let shader = source.shaders.get(index)?;
    Some(RuntimeShaderProgramId {
        asset_slot: u32::try_from(index).ok()?,
        program_hash: program_hash(&shader.program),
    })
}

/// The vertex declaration slot a pass names, and the declaration itself when
/// the corpus has it.
fn vertex_decl_of(
    source: &MaterialDefinitions,
    pass: &OwnedMaterialPass,
) -> (u32, Option<RuntimeVertexDecl>) {
    let slot = pass
        .vertex_decl
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(u32::MAX);
    let decl = pass
        .vertex_decl
        .and_then(|index| source.vertex_decls.get(index))
        .map(|authored| RuntimeVertexDecl {
            family: authored.family,
            name: authored.name.to_string(),
            stream_count: authored.stream_count,
            has_optional_source: authored.has_optional_source,
            routing: authored.routing,
        });
    (slot, decl)
}

/// Remember `decl` under `slot`, keeping the list sorted by slot. The first
/// declaration seen for a slot stays.
fn remember_vertex_decl(
    decls: &mut Vec<(u32, RuntimeVertexDecl)>,
    slot: u32,
    decl: RuntimeVertexDecl,
) {
    if let Err(at) = decls.binary_search_by_key(&slot, |(known, _)| *known) {
        decls.insert(at, (slot, decl));
    }
}

fn capture_pass(
    source: &MaterialDefinitions,
    facts: &TechniqueSetFacts,
    slot: usize,
    pass: &OwnedMaterialPass,
    decls: &mut Vec<(u32, RuntimeVertexDecl)>,
) -> RuntimePass {
    let (vertex_decl_slot, decl) = vertex_decl_of(source, pass);
    if let Some(decl) = decl {
        remember_vertex_decl(decls, vertex_decl_slot, decl);
    }
    let shader_pair = shader_id(source, &pass.vertex_shader)
        .zip(shader_id(source, &pass.pixel_shader))
        .map(|(vertex, pixel)| RuntimeShaderPair {
            vertex,
            pixel,
            vertex_decl_slot,
        });
    let mut captured = RuntimePass {
        shader_pair,
        custom_sampler_flags: pass.custom_sampler_flags,
        t5_custom_sampler_flags: pass.t5_custom_sampler_flags,
        per_prim_arg_count: pass.per_prim_arg_count,
        per_obj_arg_count: pass.per_obj_arg_count,
        stable_arg_count: pass.stable_arg_count,
        arguments: pass.arguments.iter().map(bind_argument).collect(),
        color_space: pass_color_space(facts.namespace, slot as u8),
    };
    sort_pass_args(&mut captured);
    captured
}

fn capture_technique_sets(
    source: &MaterialDefinitions,
) -> (Vec<RuntimeTechniqueSet>, Vec<(u32, RuntimeVertexDecl)>) {
    let mut decls = Vec::new();
    let mut sets = Vec::new();
    for facts in source.technique_set_facts() {
        let mut slots = vec![None; TECHNIQUE_SLOT_COUNT];
        let graph = facts.table.as_ref().and_then(|table| table.graph.as_ref());
        for (slot, technique) in graph
            .iter()
            .flat_map(|graph| graph.slots.iter())
            .enumerate()
        {
            let (Some(technique), Some(place)) = (technique, slots.get_mut(slot)) else {
                continue;
            };
            let passes = technique
                .passes
                .iter()
                .map(|pass| capture_pass(source, facts, slot, pass, &mut decls))
                .collect();
            *place = Some(RuntimeTechnique {
                flags: technique.flags,
                passes,
            });
        }
        let set = RuntimeTechniqueSet::new(slots)
            .expect("slots was built TECHNIQUE_SLOT_COUNT long")
            .with_identity(
                facts.world_vert_format,
                facts.namespace,
                facts.name.to_string(),
            );
        sets.push(set);
    }
    (sets, decls)
}

// ---------------------------------------------------------------------
// Materials
// ---------------------------------------------------------------------

fn float_bits(words: [f32; 4]) -> [u32; 4] {
    words.map(f32::to_bits)
}

/// The draw-surf key a material is baked with, once it has an ordinal.
fn baked_draw_surf_key(material: &AuthoredMaterial, band: u16) -> u64 {
    let table = material.technique_table.as_ref();
    let prepass = dpvs_t6::material_prepass(dpvs_t6::MaterialPrepassInput {
        technique0_present: table.is_some_and(|table| table.slots & 1 != 0),
        technique1_present: table.is_some_and(|table| table.slots & 2 != 0),
        technique0_flags: table.map_or(0, |table| table.technique0_flags),
        state_flags: material.state_flags,
        techset_name_has_flag: material.technique_set.as_str().contains("flag_"),
    });
    dpvs_t6::bake_material_draw_surf_key(material.sort_key, prepass, band).packed
}

fn cull_mode_of(face: Option<MaterialCullFace>) -> Option<u8> {
    match face {
        Some(MaterialCullFace::Back) => Some(0),
        Some(MaterialCullFace::Front) => Some(1),
        Some(MaterialCullFace::None) | None => None,
    }
}

fn texture_bindings(material: &AuthoredMaterial) -> Vec<(u32, Option<RuntimeTextureBinding>)> {
    let mut textures: Vec<_> = material
        .textures
        .iter()
        .map(|texture| {
            let binding = texture
                .image
                .and_then(|index| u32::try_from(index).ok())
                .map(|image| RuntimeTextureBinding {
                    image: RuntimeImageId(image),
                    sampler_state: texture.sampler_state,
                    semantic: texture.semantic,
                });
            (texture.name_hash, binding)
        })
        .collect();
    textures.sort_by_key(|&(hash, _)| hash);
    textures
}

fn constant_bits(material: &AuthoredMaterial) -> Vec<(u32, [u32; 4])> {
    let mut constants: Vec<_> = material
        .constants
        .iter()
        .map(|constant| (constant.name_hash, float_bits(constant.literal)))
        .collect();
    constants.sort_by_key(|&(hash, _)| hash);
    constants
}

fn capture_material(
    source: &MaterialDefinitions,
    sorted: &RuntimeSortedMaterialTable,
    index: usize,
    material: &AuthoredMaterial,
) -> RuntimeMaterial {
    let key = TechsetKey::new(material.namespace, material.technique_set.as_str());
    let local_technique_set = match source.resolve_technique_set(key) {
        TechsetResolve::Hit { index, .. } => u32::try_from(index).unwrap_or(u32::MAX),
        _ => u32::MAX,
    };
    let asset_id = MaterialAssetId(
        u16::try_from(index).expect("material catalog index exceeds the u16 asset-id domain"),
    );
    let [uv_anim, falloff_parms, falloff_begin, falloff_end] =
        MaterialDefinitions::material_animation(material);
    RuntimeMaterial {
        asset_id,
        name: material.name.to_string(),
        namespace: material.namespace,
        technique_set: material.technique_set.to_string(),
        baked_draw_surf: sorted
            .ordinal_for_asset_id(index)
            .map(|ordinal| baked_draw_surf_key(material, ordinal.sort_band())),
        local_technique_set: RuntimeTechniqueSetId(local_technique_set),
        remap: RemapResolution::SelfSet,
        state_bits_entry: material.state_bits_entry,
        state_bits_table: material.state_bits.clone(),
        camera_region: material.camera_region,
        sort_key: material.sort_key,
        info_game_flags: material.info_game_flags,
        state_flags: material.state_flags,
        surface_type_bits: material.surface_type_bits,
        unlit: source.is_unlit(material).unwrap_or(false),
        uses_model_lighting_const: material
            .route
            .is_some_and(|route| route.uses_model_lighting_const),
        square_color_map: source.color_map_transform(material) == ColorMapTransform::Square,
        shadow_only: source.is_shadowcaster(material),
        t6_draw: material.t6_draw,
        cull_mode: cull_mode_of(source.cull_face(material)),
        uv_anim_bits: float_bits(uv_anim),
        falloff_parms_bits: float_bits(falloff_parms),
        falloff_begin_bits: float_bits(falloff_begin),
        falloff_end_bits: float_bits(falloff_end),
        env_map_parms_bits: material
            .constants
            .iter()
            .find(|constant| constant.name.starts_with(b"envMapParms"))
            .map_or([0; 4], |constant| float_bits(constant.literal)),
        textures: texture_bindings(material),
        constants: constant_bits(material),
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

fn capture_shader(asset_slot: usize, shader: &AuthoredShader) -> Option<RuntimeShaderProgram> {
    let stage = if shader.is_vertex() {
        RuntimeShaderStage::Vertex
    } else {
        RuntimeShaderStage::Pixel
    };
    let asset_slot = u32::try_from(asset_slot)
        .expect("shader catalog index exceeds the runtime u32 asset-slot domain");
    (!shader.program.is_empty()).then(|| RuntimeShaderProgram {
        id: RuntimeShaderProgramId {
            asset_slot,
            program_hash: program_hash(&shader.program),
        },
        stage,
        name: shader.name.to_string(),
        program: shader.program.clone(),
    })
}

/// How many pass arguments the corpus has that no argument type claims.
fn remaining_unknown_arg_n(source: &MaterialDefinitions) -> u32 {
    let unknown = source
        .technique_set_facts()
        .iter()
        .filter_map(|facts| facts.table.as_ref()?.graph.as_ref())
        .flat_map(|graph| graph.slots.iter().flatten())
        .flat_map(|technique| &technique.passes)
        .flat_map(|pass| &pass.arguments)
        .filter(|argument| matches!(argument, OwnedShaderArgument::Unknown { .. }))
        .count();
    u32::try_from(unknown).unwrap_or(u32::MAX)
}

fn sorted_table_of(
    built: &Result<SortedBuild, super::CatalogBuildError>,
) -> RuntimeSortedMaterialTable {
    match built {
        Ok(built) => RuntimeSortedMaterialTable::Ready {
            asset_ids_by_ordinal: built.asset_ids_by_ordinal.clone(),
            ordinals_by_asset_id: built.ordinals_by_asset_id.clone(),
            skipped_n: built.skipped_n,
            slot_gap_n: built.slot_gap_n,
        },
        Err(cause) => RuntimeSortedMaterialTable::BuildFailed(cause.clone()),
    }
}

pub fn capture_runtime_catalog(source: &MaterialDefinitions) -> RuntimeMaterialCatalog {
    let (technique_sets, vertex_decls) = capture_technique_sets(source);

    let built = build_sorted_material_table(source);
    let sorted_materials = sorted_table_of(&built);
    let (sorted_first_skip, skip_tech, skip_cause) = match built {
        Ok(built) => (built.first_skip, built.skip_tech, built.skip_cause),
        Err(_) => (None, None, None),
    };

    let materials: Vec<_> = source
        .materials
        .iter()
        .enumerate()
        .map(|(index, material)| capture_material(source, &sorted_materials, index, material))
        .collect();
    let material_indices_by_name =
        first_index_by_name(materials.iter().map(|material| material.name.as_str()));
    let shader_programs = source
        .shaders
        .iter()
        .enumerate()
        .map(|(slot, shader)| capture_shader(slot, shader))
        .collect();

    RuntimeMaterialCatalog {
        generation_id: mint_material_generation_id(),
        materials,
        material_indices_by_name,
        technique_sets,
        shader_programs,
        vertex_decls,
        sorted_materials,
        sorted_first_skip,
        skip_tech,
        skip_cause,
        leftover_t5_arg_n: source.leftover_t5_arg_n,
        leftover_t5_arg: source.leftover_t5_arg_top(),
        leftover_t5_arg2: source.leftover_t5_arg_ranked(1),
        leftover_t5_arg3: source.leftover_t5_arg_ranked(2),
        leftover_t5_arg4: source.leftover_t5_arg_ranked(3),
        leftover_t5_arg5: source.leftover_t5_arg_ranked(4),
        leftover_t5_dest6: source.leftover_t5_arg_dest(6),
        leftover_t5_dest7: source.leftover_t5_arg_dest(7),
        leftover_t5_dest9: source.leftover_t5_arg_dest(9),
        leftover_t5_dest10: source.leftover_t5_arg_dest(10),
        leftover_t5_dest11: source.leftover_t5_arg_dest(11),
        leftover_t5_dest17: source.leftover_t5_arg_dest(17),
        leftover_t5_dest18: source.leftover_t5_arg_dest(18),
        leftover_t5_dest19: source.leftover_t5_arg_dest(19),
        leftover_t5_dest20: source.leftover_t5_arg_dest(20),
        leftover_t5_dest24: source.leftover_t5_arg_dest(24),
        leftover_t5_dest25: source.leftover_t5_arg_dest(25),
        leftover_t5_dest26: source.leftover_t5_arg_dest(26),
        leftover_t5_dest27: source.leftover_t5_arg_dest(27),
        leftover_unknown_n: remaining_unknown_arg_n(source),
    }
}

#[cfg(test)]
mod tests {
    use asset_material::ShaderKind;

    use super::*;

    fn shader(name: &str, kind: ShaderKind, program: &[u8]) -> AuthoredShader {
        AuthoredShader {
            namespace: asset_core::AssetNamespace::T5,
            name: asset_core::AssetRef::decode(name),
            kind,
            program: program.to_vec(),
        }
    }

    fn shader_ref(shader: Option<usize>) -> OwnedShaderRef {
        OwnedShaderRef {
            shader,
        }
    }

    #[test]
    fn program_hash_is_fnv_1a() {
        assert_eq!(program_hash(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(program_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(program_hash(b"ab"), program_hash(b"ba"));
    }

    #[test]
    fn generation_ids_are_never_reused() {
        let first = mint_material_generation_id();
        let second = mint_material_generation_id();
        assert!(second.0 > first.0);
    }

    #[test]
    fn every_argument_kind_binds_to_its_runtime_form() {
        use RuntimeShaderStage::{Pixel, Vertex};

        let words = Some([1, 2, 3, 4]);
        let cases = [
            (
                OwnedShaderArgument::MaterialVertexConstant {
                    destination: 1,
                    name_hash: 10,
                },
                RuntimeArgumentBinding::MaterialConstant {
                    stage: Vertex,
                    destination: 1,
                    name_hash: 10,
                },
            ),
            (
                OwnedShaderArgument::MaterialPixelConstant {
                    destination: 2,
                    name_hash: 11,
                },
                RuntimeArgumentBinding::MaterialConstant {
                    stage: Pixel,
                    destination: 2,
                    name_hash: 11,
                },
            ),
            (
                OwnedShaderArgument::LiteralVertexConstant {
                    destination: 3,
                    words,
                },
                RuntimeArgumentBinding::LiteralConstant {
                    stage: Vertex,
                    destination: 3,
                    words,
                },
            ),
            (
                OwnedShaderArgument::LiteralPixelConstant {
                    destination: 4,
                    words: None,
                },
                RuntimeArgumentBinding::LiteralConstant {
                    stage: Pixel,
                    destination: 4,
                    words: None,
                },
            ),
            (
                OwnedShaderArgument::MaterialPixelSampler {
                    destination: 5,
                    name_hash: 12,
                },
                RuntimeArgumentBinding::MaterialTexture {
                    destination: 5,
                    name_hash: 12,
                },
            ),
            (
                OwnedShaderArgument::CodeVertexConstant {
                    destination: 6,
                    index: 7,
                    first_row: 1,
                    row_count: 2,
                },
                RuntimeArgumentBinding::CodeConstant {
                    stage: Vertex,
                    destination: 6,
                    index: 7,
                    first_row: 1,
                    row_count: 2,
                },
            ),
            (
                OwnedShaderArgument::CodePixelConstant {
                    destination: 8,
                    index: 9,
                    first_row: 0,
                    row_count: 4,
                },
                RuntimeArgumentBinding::CodeConstant {
                    stage: Pixel,
                    destination: 8,
                    index: 9,
                    first_row: 0,
                    row_count: 4,
                },
            ),
            (
                OwnedShaderArgument::CodePixelSampler {
                    destination: 3,
                    index: 40,
                },
                RuntimeArgumentBinding::CodeTexture {
                    destination: 3,
                    index: 40,
                },
            ),
            (
                OwnedShaderArgument::Unknown {
                    argument_type: 77,
                    raw: [9; 8],
                },
                RuntimeArgumentBinding::Unknown {
                    argument_type: 77,
                    raw: [9; 8],
                },
            ),
        ];
        for (argument, bound) in cases {
            assert_eq!(bind_argument(&argument), bound, "{argument:?}");
        }
    }

    #[test]
    fn a_shader_is_named_by_slot_and_content() {
        let mut source = MaterialDefinitions::default();
        source
            .shaders
            .push(shader("vs", ShaderKind::Vertex, b"abc"));
        let id = shader_id(&source, &shader_ref(Some(0)));
        assert_eq!(
            id,
            Some(RuntimeShaderProgramId {
                asset_slot: 0,
                program_hash: program_hash(b"abc")
            })
        );
        // No shader named, or one the corpus lacks.
        assert_eq!(shader_id(&source, &shader_ref(None)), None);
        assert_eq!(shader_id(&source, &shader_ref(Some(5))), None);
    }

    #[test]
    fn shader_programs_keep_their_stage_and_empty_ones_drop_out() {
        let vertex =
            capture_shader(3, &shader("vs", ShaderKind::Vertex, b"xy")).expect("has bytes");
        assert_eq!(vertex.stage, RuntimeShaderStage::Vertex);
        assert_eq!(vertex.id.asset_slot, 3);
        assert_eq!(vertex.id.program_hash, program_hash(b"xy"));
        assert_eq!(vertex.name, "vs");
        assert_eq!(vertex.program, b"xy");
        let pixel = capture_shader(4, &shader("ps", ShaderKind::Pixel, b"z")).expect("has bytes");
        assert_eq!(pixel.stage, RuntimeShaderStage::Pixel);
        assert!(capture_shader(5, &shader("empty", ShaderKind::Pixel, b"")).is_none());
    }

    fn decl(name: &str) -> RuntimeVertexDecl {
        RuntimeVertexDecl {
            family: Default::default(),
            name: name.into(),
            stream_count: 1,
            has_optional_source: 0,
            routing: Default::default(),
        }
    }

    #[test]
    fn vertex_declarations_stay_sorted_by_slot_and_the_first_one_wins() {
        let mut decls = Vec::new();
        remember_vertex_decl(&mut decls, 5, decl("five"));
        remember_vertex_decl(&mut decls, 2, decl("two"));
        remember_vertex_decl(&mut decls, 9, decl("nine"));
        remember_vertex_decl(&mut decls, 5, decl("again"));
        let seen: Vec<_> = decls
            .iter()
            .map(|(slot, decl)| (*slot, decl.name.as_str()))
            .collect();
        assert_eq!(seen, [(2, "two"), (5, "five"), (9, "nine")]);
    }

    #[test]
    fn cull_faces_become_the_runtime_codes() {
        assert_eq!(cull_mode_of(Some(MaterialCullFace::Back)), Some(0));
        assert_eq!(cull_mode_of(Some(MaterialCullFace::Front)), Some(1));
        assert_eq!(cull_mode_of(Some(MaterialCullFace::None)), None);
        assert_eq!(cull_mode_of(None), None);
    }

    #[test]
    fn a_name_points_at_its_first_material() {
        assert_eq!(
            first_index_by_name(["wall", "floor", "wall"].into_iter()),
            [("floor".to_owned(), 1), ("wall".to_owned(), 0)]
        );
        assert!(first_index_by_name(std::iter::empty()).is_empty());
    }

    #[test]
    fn a_failed_sort_is_kept_as_the_failure() {
        let failure = super::super::CatalogBuildError::MaterialAssetIdOverflow { index: 70_000 };
        assert_eq!(
            sorted_table_of(&Err(failure.clone())),
            RuntimeSortedMaterialTable::BuildFailed(failure)
        );
    }

    #[test]
    fn an_empty_corpus_captures_an_empty_catalog() {
        let first = capture_runtime_catalog(&MaterialDefinitions::default());
        assert!(first.materials.is_empty());
        assert!(first.technique_sets.is_empty());
        assert!(first.shader_programs.is_empty());
        assert!(first.vertex_decls.is_empty());
        assert!(first.material_indices_by_name.is_empty());
        assert_eq!(first.leftover_unknown_n, 0);
        assert_eq!(first.sorted_first_skip, None);
        assert!(matches!(
            first.sorted_materials,
            RuntimeSortedMaterialTable::Ready { skipped_n: 0, .. }
        ));
        let second = capture_runtime_catalog(&MaterialDefinitions::default());
        assert_ne!(first.generation_id, second.generation_id);
    }

    #[test]
    fn shaders_in_the_corpus_reach_the_catalog_in_slot_order() {
        let mut source = MaterialDefinitions::default();
        source.shaders.push(shader("a", ShaderKind::Vertex, b"1"));
        source.shaders.push(shader("b", ShaderKind::Pixel, b""));
        source.shaders.push(shader("c", ShaderKind::Pixel, b"3"));
        let catalog = capture_runtime_catalog(&source);
        let present: Vec<_> = catalog
            .shader_programs
            .iter()
            .map(|program| program.as_ref().map(|p| p.name.as_str()))
            .collect();
        assert_eq!(present, [Some("a"), None, Some("c")]);
    }
}
