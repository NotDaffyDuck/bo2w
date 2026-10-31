//! Vertex layouts, the texture-table binding contract, retained draw items,
//! texture-bind identities and the code-constant helpers.

use glam::{Mat4, Vec3};

use d3d9_sm3::PassShaderText;
use render_frame::view_math::{
    eye_relative_clip, flip_matrix_rows_to_words, engine_transpose_matrix_rows,
    vec4_bits,
};
use render_frame::{
    LevelViewLane, LIVE_INDEX_BUFFER_CAP, RENDER_EFFECT_DEPTH_HACK, HeldDrawItem,
    HeldDrawKind, SurfaceBakedLightId, SurfaceCubeProbeId, FacetSamplerInputs,
    FLAT_TEXTURE_TABLE_SLOTS, VOLUME_TEXTURE_TABLE_SIZE, CUBEMAP_TEXTURE_TABLE_SIZE,
    SAMPLER_TABLE_SIZE, TextureBindingKey, GpuBindResourceKind, WgpuLayoutRejection,
    TextureTableDimension, GpuStageVisibility, GpuVertexElementKind, MODEL_OBJECT_ID_FIRST_PERSON_WEAPON, plan_gpu_pass_bindings,
    viewmodel_fx_flags, table_bind_entries, model_tess_packed_arm,
    model_tess_vert_decl,
};
use render_material::decl_table::{VertexElemType, VertexStreamArrangement};
use render_material::{
    CodeSamplerChannel, LocalSamplerBundle, PassShaderContract, ShaderPortKey, LiveImageId,
    PassVertexInput, VertexFormatLineage,
};

// ---- code constants -------------------------------------------------------

#[test]
fn float_bits_are_the_raw_ieee_bits() {
    assert_eq!(vec4_bits([1.0, -0.0, 0.5, f32::INFINITY]), [
        0x3f80_0000,
        0x8000_0000,
        0x3f00_0000,
        0x7f80_0000
    ]);
}

#[test]
fn a_matrix_is_handed_over_one_transposed_row_at_a_time() {
    let m = Mat4::from_cols_array(&[
        1.0, 2.0, 3.0, 4.0, //
        5.0, 6.0, 7.0, 8.0, //
        9.0, 10.0, 11.0, 12.0, //
        13.0, 14.0, 15.0, 16.0,
    ]);
    let rows = flip_matrix_rows_to_words(m);
    assert_eq!(rows[0], vec4_bits([1.0, 5.0, 9.0, 13.0]));
    assert_eq!(rows[3], vec4_bits([4.0, 8.0, 12.0, 16.0]));
    assert_eq!(engine_transpose_matrix_rows(m), rows.to_vec());
}

#[test]
fn camera_relative_projection_undoes_the_view_origin() {
    let clip_from_world = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)) * Mat4::from_scale(Vec3::splat(2.0));
    let origin = Vec3::new(10.0, 0.0, -4.0);
    let relative = eye_relative_clip(clip_from_world, origin);
    // A point given relative to the origin lands where the world point does.
    let p = Vec3::new(3.0, 5.0, 7.0);
    let direct = clip_from_world.transform_point3(p + origin);
    assert!(relative.transform_point3(p).distance(direct) < 1e-5);
}

// ---- vertex helpers -------------------------------------------------------

#[test]
fn only_tess_infos_one_three_and_four_are_packed() {
    for byte in 0..=255u8 {
        let packed = matches!(byte, 1 | 3 | 4);
        assert_eq!(model_tess_packed_arm(byte), packed, "tess info {byte}");
        assert_eq!(model_tess_vert_decl(byte), if packed { 1 } else { 2 });
    }
}

#[test]
fn surface_sampler_inputs_start_unbound() {
    let none = FacetSamplerInputs::default();
    assert!(none.reflection_probe.is_none() && none.primary_lightmap.is_none());
    let bound = FacetSamplerInputs {
        reflection_probe: Some(SurfaceCubeProbeId(2)),
        primary_lightmap: Some(SurfaceBakedLightId(1)),
        secondary_lightmap: None,
    };
    assert_ne!(none, bound);
    assert_eq!(LIVE_INDEX_BUFFER_CAP, 1 << 20);
}

// ---- texture table --------------------------------------------------------

#[test]
fn the_texture_table_has_four_bindings_in_one_group() {
    let entries = table_bind_entries();
    let dims = [
        (TextureTableDimension::D2, FLAT_TEXTURE_TABLE_SLOTS),
        (TextureTableDimension::Cube, CUBEMAP_TEXTURE_TABLE_SIZE),
        (TextureTableDimension::D3, VOLUME_TEXTURE_TABLE_SIZE),
    ];
    for (entry, (dimension, count)) in entries.iter().zip(dims) {
        assert_eq!(entry.kind, GpuBindResourceKind::TextureList { dimension, count });
    }
    assert_eq!(
        entries[3].kind,
        GpuBindResourceKind::SamplerArray { count: SAMPLER_TABLE_SIZE }
    );
    assert!(entries.iter().all(|e| e.band == entries[0].band));
    assert!(entries.iter().all(|e| e.visibility == GpuStageVisibility::VertexAndPixelStage));
    assert!(entries.iter().all(|e| e.register_packed.is_none()));
    let mut bindings: Vec<u16> = entries.iter().map(|e| e.binding).collect();
    bindings.sort_unstable();
    bindings.dedup();
    assert_eq!(bindings.len(), 4);
}

// ---- pass layouts ---------------------------------------------------------

const VERTEX_TYPE: u8 = 2;

fn stride() -> u16 {
    VertexFormatLineage::Fallback.host_stride_bytes(VERTEX_TYPE, 0).expect("stream 0 has an extent")
}

fn attribute(location: u32, offset: u8, decl_type: VertexElemType) -> PassVertexInput {
    PassVertexInput {
        source: 0,
        layout: VertexStreamArrangement { stream: 0, offset, decl_type },
        location,
    }
}

fn abi(attributes: Vec<PassVertexInput>) -> PassShaderContract {
    PassShaderContract { vertex_family: VertexFormatLineage::Fallback, vertex_format_code: VERTEX_TYPE, attributes }
}

fn module() -> PassShaderText {
    PassShaderText {
        source: String::new(),
        attribute_tally: 0,
        interpolant_tally: 0,
        vs_const_count: 0,
        ps_const_count: 0,
        sampler_tally: 0,
    }
}

#[test]
fn a_pass_without_attributes_still_gets_the_constants_buffer_and_texture_table() {
    let layout = plan_gpu_pass_bindings(&abi(Vec::new()), &module()).unwrap();
    assert!(layout.vertex_buffers.is_empty());
    assert_eq!(layout.bind_entries.len(), 5);
    let first = layout.bind_entries[0];
    assert_eq!((first.band, first.binding), (0, 0));
    assert_eq!(first.kind, GpuBindResourceKind::ReadableDataBuffer);
    assert_eq!(first.visibility, GpuStageVisibility::VertexAndPixelStage);
    assert_eq!(layout.bind_entries[1..], table_bind_entries());
}

#[test]
fn attributes_are_grouped_by_stream_and_ordered_by_offset() {
    let layout = plan_gpu_pass_bindings(
        &abi(vec![
            attribute(5, 12, VertexElemType::ThreeFloats),
            attribute(2, 0, VertexElemType::QuadFloat),
            attribute(9, 12, VertexElemType::FourBytesRaw),
        ]),
        &module(),
    )
    .unwrap();
    assert_eq!(layout.vertex_buffers.len(), 1);
    let buffer = &layout.vertex_buffers[0];
    assert_eq!((buffer.slot, buffer.array_stride), (0, u64::from(stride())));
    let seen: Vec<(u32, u64, GpuVertexElementKind)> =
        buffer.attributes.iter().map(|a| (a.location, a.offset, a.format)).collect();
    assert_eq!(
        seen,
        [
            (2, 0, GpuVertexElementKind::Float32x4),
            (5, 12, GpuVertexElementKind::Float32x3),
            (9, 12, GpuVertexElementKind::Uint8x4),
        ]
    );
}

#[test]
fn every_declaration_type_maps_to_a_wgpu_format() {
    let cases = [
        (VertexElemType::TwoFloats, GpuVertexElementKind::Float32x2),
        (VertexElemType::Float16x2, GpuVertexElementKind::Float16x2),
        (VertexElemType::ThreeFloats, GpuVertexElementKind::Float32x3),
        (VertexElemType::QuadFloat, GpuVertexElementKind::Float32x4),
        (VertexElemType::PackedColorBytes, GpuVertexElementKind::Unorm8x4),
        (VertexElemType::FourBytesNormalised, GpuVertexElementKind::Unorm8x4),
        (VertexElemType::FourBytesRaw, GpuVertexElementKind::Uint8x4),
        (VertexElemType::T6UnitVec, GpuVertexElementKind::Uint32),
        (VertexElemType::T6TexCoord, GpuVertexElementKind::Uint32),
        (VertexElemType::T6WorldTexLmap, GpuVertexElementKind::Uint32x4),
    ];
    for (decl, expected) in cases {
        let layout = plan_gpu_pass_bindings(&abi(vec![attribute(0, 0, decl)]), &module())
            .unwrap_or_else(|why| panic!("{decl:?} refused: {why:?}"));
        assert_eq!(layout.vertex_buffers[0].attributes[0].format, expected, "{decl:?}");
    }
}

#[test]
fn an_unknown_declaration_type_is_refused() {
    let err = plan_gpu_pass_bindings(&abi(vec![attribute(0, 0, VertexElemType::Unknown(77))]), &module())
        .unwrap_err();
    assert_eq!(err, WgpuLayoutRejection::UnlistedVertexFormat { stream_slot: 0, decl_code: 77 });
}

#[test]
fn two_attributes_may_not_share_a_location() {
    let err = plan_gpu_pass_bindings(
        &abi(vec![attribute(3, 0, VertexElemType::TwoFloats), attribute(3, 8, VertexElemType::TwoFloats)]),
        &module(),
    )
    .unwrap_err();
    assert_eq!(err, WgpuLayoutRejection::DuplicateShaderLocation { location: 3 });
}

#[test]
fn an_attribute_may_not_run_past_the_stream() {
    let offset = (stride() - 8) as u8;
    let err = plan_gpu_pass_bindings(&abi(vec![attribute(0, offset, VertexElemType::QuadFloat)]), &module())
        .unwrap_err();
    assert_eq!(
        err,
        WgpuLayoutRejection::AttrOverrunsStride {
            stream: 0,
            offset,
            size: 16,
            stride_bytes: stride(),
        }
    );
}

#[test]
fn a_stream_with_no_extent_is_refused() {
    let mut odd = attribute(0, 0, VertexElemType::TwoFloats);
    odd.layout.stream = 14;
    let err = plan_gpu_pass_bindings(&abi(vec![odd]), &module()).unwrap_err();
    assert_eq!(err, WgpuLayoutRejection::StreamExtentAbsent { vertex_format_code: VERTEX_TYPE, stream: 14 });
}

// ---- retained draws -------------------------------------------------------

fn item(key: u64, rank: u32) -> HeldDrawItem {
    HeldDrawItem {
        material_key: None,
        key,
        material_order_no: rank,
        kind: HeldDrawKind::world(0),
        facet_samplers: FacetSamplerInputs::default(),
        camera_zone: None,
    }
}

#[test]
fn the_host_sort_key_is_light_then_rank_then_low_key_bits() {
    let it = item((0xabc_u64 << 44) | 0xdead_beef, 77);
    assert_eq!(it.submission_order_key(), (0xabc, 77, 0xdead_beef));
    let high = item(u64::MAX, 0);
    assert_eq!(high.submission_order_key(), ((u64::MAX >> 44) as u32, 0, u32::MAX));
}

#[test]
fn a_plain_world_draw_is_one_surface_at_the_origin() {
    match HeldDrawKind::world(41) {
        HeldDrawKind::World { surf, run, run_start_offset, world_tree_camera_kind, level_tree_run_first, model_to_world: model_matrix } => {
            assert_eq!((surf, run, run_start_offset), (41, 1, 0));
            assert!(world_tree_camera_kind.is_none() && level_tree_run_first.is_none());
            assert_eq!(model_matrix, Mat4::IDENTITY);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn a_posed_world_draw_keeps_its_matrix() {
    let pose = Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0));
    match HeldDrawKind::posed_face(3, pose) {
        HeldDrawKind::World { surf, model_to_world: mesh_to_world, .. } => {
            assert_eq!(surf, 3);
            assert_eq!(mesh_to_world, pose);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn a_bsp_run_records_its_lane_and_first_surface() {
    match HeldDrawKind::map_geometry_kind(7, 5, 30, LevelViewLane::Decal, 6) {
        HeldDrawKind::World { surf, run, run_start_offset, world_tree_camera_kind, level_tree_run_first, model_to_world: world_from_local } => {
            assert_eq!((surf, run, run_start_offset), (7, 5, 30));
            assert_eq!(world_tree_camera_kind, Some(LevelViewLane::Decal));
            assert_eq!(level_tree_run_first, Some(6));
            assert_eq!(world_from_local, Mat4::IDENTITY);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn only_the_viewmodel_object_gets_the_depth_hack() {
    assert_eq!(viewmodel_fx_flags(MODEL_OBJECT_ID_FIRST_PERSON_WEAPON), RENDER_EFFECT_DEPTH_HACK);
    assert_eq!(viewmodel_fx_flags(0), 0);
    assert_eq!(viewmodel_fx_flags(2), 0);
    assert_eq!((MODEL_OBJECT_ID_FIRST_PERSON_WEAPON, RENDER_EFFECT_DEPTH_HACK), (1, 2));
}

// ---- texture bind identities ---------------------------------------------

fn port() -> ShaderPortKey {
    ShaderPortKey {
        vertex_shader_digest: 1,
        fragment_shader_digest: 2,
        vertex_format_slot: 3,
        vertex_format_code: 4,
        sampler_custom_bits: 5,
        argument_digest: 6,
    }
}

fn lane(register: u16, index: u32, state: u8, image: Option<u32>) -> CodeSamplerChannel {
    CodeSamplerChannel { sampler_reg: register, index, texture_sampling_mode: state, image: image.map(LiveImageId) }
}

fn identity(lanes: &[CodeSamplerChannel]) -> TextureBindingKey {
    TextureBindingKey::assemble_from_channels(port(), None, lanes, FacetSamplerInputs::default())
}

#[test]
fn equal_sampler_lanes_make_equal_identities() {
    let a = [lane(1, 2, 3, Some(9)), lane(4, 5, 6, None)];
    let b = [lane(1, 2, 3, Some(9)), lane(4, 5, 6, None)];
    assert_eq!(identity(&a), identity(&b));
    assert_eq!(identity(&a).local, 0);
}

#[test]
fn any_change_to_a_lane_changes_the_identity() {
    let base = [lane(1, 2, 3, Some(9))];
    let variants = [
        [lane(2, 2, 3, Some(9))],
        [lane(1, 3, 3, Some(9))],
        [lane(1, 2, 4, Some(9))],
        [lane(1, 2, 3, Some(8))],
        [lane(1, 2, 3, None)],
    ];
    for variant in &variants {
        assert_ne!(identity(&base).code, identity(variant).code, "{variant:?}");
    }
    assert_ne!(identity(&base).code, identity(&[]).code);
    assert_ne!(
        identity(&[lane(1, 2, 3, None), lane(4, 5, 6, None)]).code,
        identity(&[lane(4, 5, 6, None), lane(1, 2, 3, None)]).code
    );
}

#[test]
fn local_samplers_enter_by_their_id() {
    let local = LocalSamplerBundle { id: 0x55, lanes: Vec::new() };
    let with = TextureBindingKey::assemble_from_channels(port(), Some(&local), &[], FacetSamplerInputs::default());
    assert_eq!(with.local, 0x55);
    assert_eq!(with.code, identity(&[]).code);
    assert_ne!(with, identity(&[]));
}

#[test]
fn the_surface_inputs_are_part_of_the_identity() {
    let lit = FacetSamplerInputs { primary_lightmap: Some(SurfaceBakedLightId(0)), ..Default::default() };
    assert_ne!(TextureBindingKey::assemble_from_channels(port(), None, &[], lit), identity(&[]));
}

#[test]
fn from_packed_uses_the_stored_code_id() {
    let code = render_material::CompactEngineSamplers { id: 0x1234, lanes: Vec::new() };
    let id = TextureBindingKey::unpack_from_bits(port(), None, &code, FacetSamplerInputs::default());
    assert_eq!((id.local, id.code), (0, 0x1234));
}
