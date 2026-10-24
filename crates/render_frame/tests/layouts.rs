//! Vertex layouts, the texture-table binding contract, retained draw items,
//! texture-bind identities and the code-constant helpers.

use glam::{Mat4, Vec3};

use d3d9_sm3::PassWgsl;
use render_frame::code_math::{
    camera_relative_view_projection, code_transpose_matrix_row4, code_transpose_matrix_rows,
    float4_bits,
};
use render_frame::{
    BspCameraLane, DYNAMIC_INDEX_BUFFER_CAPACITY, RENDER_FX_DEPTH_HACK, RetainedDrawItem,
    RetainedDrawKind, SurfaceLightmapId, SurfaceReflectionProbeId, SurfaceSamplerInputs,
    TEXTURE_TABLE_2D_CAPACITY, TEXTURE_TABLE_3D_CAPACITY, TEXTURE_TABLE_CUBE_CAPACITY,
    TEXTURE_TABLE_SAMPLER_CAPACITY, TextureBindIdentity, WgpuBindingKind, WgpuLayoutRefusal,
    TextureTableDimension, WgpuShaderVisibility, WgpuVertexFormat, XMODEL_OBJECT_ID_VIEWMODEL, derive_wgpu_pass_layout,
    host_viewmodel_render_fx_flags, texture_table_bind_entries, xmodel_tess_info_packed_arm,
    xmodel_tess_info_vert_decl_type,
};
use render_material::decl_table::{D3dDeclType, StreamSourceLayout};
use render_material::{
    PackedCodeSamplerLane, PackedLocalSamplers, PassProgramAbi, PortId, RuntimeImageId,
    VertexAttribute, VertexLayoutFamily,
};

// ---- code constants -------------------------------------------------------

#[test]
fn float_bits_are_the_raw_ieee_bits() {
    assert_eq!(float4_bits([1.0, -0.0, 0.5, f32::INFINITY]), [
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
    let rows = code_transpose_matrix_row4(m);
    assert_eq!(rows[0], float4_bits([1.0, 5.0, 9.0, 13.0]));
    assert_eq!(rows[3], float4_bits([4.0, 8.0, 12.0, 16.0]));
    assert_eq!(code_transpose_matrix_rows(m), rows.to_vec());
}

#[test]
fn camera_relative_projection_undoes_the_view_origin() {
    let clip_from_world = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)) * Mat4::from_scale(Vec3::splat(2.0));
    let origin = Vec3::new(10.0, 0.0, -4.0);
    let relative = camera_relative_view_projection(clip_from_world, origin);
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
        assert_eq!(xmodel_tess_info_packed_arm(byte), packed, "tess info {byte}");
        assert_eq!(xmodel_tess_info_vert_decl_type(byte), if packed { 1 } else { 2 });
    }
}

#[test]
fn surface_sampler_inputs_start_unbound() {
    let none = SurfaceSamplerInputs::default();
    assert!(none.reflection_probe.is_none() && none.primary_lightmap.is_none());
    let bound = SurfaceSamplerInputs {
        reflection_probe: Some(SurfaceReflectionProbeId(2)),
        primary_lightmap: Some(SurfaceLightmapId(1)),
        secondary_lightmap: None,
    };
    assert_ne!(none, bound);
    assert_eq!(DYNAMIC_INDEX_BUFFER_CAPACITY, 1 << 20);
}

// ---- texture table --------------------------------------------------------

#[test]
fn the_texture_table_has_four_bindings_in_one_group() {
    let entries = texture_table_bind_entries();
    let dims = [
        (TextureTableDimension::D2, TEXTURE_TABLE_2D_CAPACITY),
        (TextureTableDimension::Cube, TEXTURE_TABLE_CUBE_CAPACITY),
        (TextureTableDimension::D3, TEXTURE_TABLE_3D_CAPACITY),
    ];
    for (entry, (dimension, count)) in entries.iter().zip(dims) {
        assert_eq!(entry.kind, WgpuBindingKind::TextureArray { dimension, count });
    }
    assert_eq!(
        entries[3].kind,
        WgpuBindingKind::SamplerArray { count: TEXTURE_TABLE_SAMPLER_CAPACITY }
    );
    assert!(entries.iter().all(|e| e.group == entries[0].group));
    assert!(entries.iter().all(|e| e.visibility == WgpuShaderVisibility::VertexFragment));
    assert!(entries.iter().all(|e| e.register_packed.is_none()));
    let mut bindings: Vec<u16> = entries.iter().map(|e| e.binding).collect();
    bindings.sort_unstable();
    bindings.dedup();
    assert_eq!(bindings.len(), 4);
}

// ---- pass layouts ---------------------------------------------------------

const VERTEX_TYPE: u8 = 2;

fn stride() -> u16 {
    VertexLayoutFamily::Fallback.host_stream_stride(VERTEX_TYPE, 0).expect("stream 0 has an extent")
}

fn attribute(location: u32, offset: u8, decl_type: D3dDeclType) -> VertexAttribute {
    VertexAttribute {
        source: 0,
        layout: StreamSourceLayout { stream: 0, offset, decl_type },
        location,
    }
}

fn abi(attributes: Vec<VertexAttribute>) -> PassProgramAbi {
    PassProgramAbi { vertex_family: VertexLayoutFamily::Fallback, vertex_type: VERTEX_TYPE, attributes }
}

fn module() -> PassWgsl {
    PassWgsl {
        source: String::new(),
        attribute_count: 0,
        varying_count: 0,
        vertex_constant_len: 0,
        pixel_constant_len: 0,
        sampler_count: 0,
    }
}

#[test]
fn a_pass_without_attributes_still_gets_the_constants_buffer_and_texture_table() {
    let layout = derive_wgpu_pass_layout(&abi(Vec::new()), &module()).unwrap();
    assert!(layout.vertex_buffers.is_empty());
    assert_eq!(layout.bind_entries.len(), 5);
    let first = layout.bind_entries[0];
    assert_eq!((first.group, first.binding), (0, 0));
    assert_eq!(first.kind, WgpuBindingKind::ReadOnlyStorageBuffer);
    assert_eq!(first.visibility, WgpuShaderVisibility::VertexFragment);
    assert_eq!(layout.bind_entries[1..], texture_table_bind_entries());
}

#[test]
fn attributes_are_grouped_by_stream_and_ordered_by_offset() {
    let layout = derive_wgpu_pass_layout(
        &abi(vec![
            attribute(5, 12, D3dDeclType::Float3),
            attribute(2, 0, D3dDeclType::Float4),
            attribute(9, 12, D3dDeclType::UByte4),
        ]),
        &module(),
    )
    .unwrap();
    assert_eq!(layout.vertex_buffers.len(), 1);
    let buffer = &layout.vertex_buffers[0];
    assert_eq!((buffer.stream, buffer.array_stride), (0, u64::from(stride())));
    let seen: Vec<(u32, u64, WgpuVertexFormat)> =
        buffer.attributes.iter().map(|a| (a.location, a.offset, a.format)).collect();
    assert_eq!(
        seen,
        [
            (2, 0, WgpuVertexFormat::Float32x4),
            (5, 12, WgpuVertexFormat::Float32x3),
            (9, 12, WgpuVertexFormat::Uint8x4),
        ]
    );
}

#[test]
fn every_declaration_type_maps_to_a_wgpu_format() {
    let cases = [
        (D3dDeclType::Float2, WgpuVertexFormat::Float32x2),
        (D3dDeclType::Float16x2, WgpuVertexFormat::Float16x2),
        (D3dDeclType::Float3, WgpuVertexFormat::Float32x3),
        (D3dDeclType::Float4, WgpuVertexFormat::Float32x4),
        (D3dDeclType::D3dColor, WgpuVertexFormat::Unorm8x4),
        (D3dDeclType::UByte4N, WgpuVertexFormat::Unorm8x4),
        (D3dDeclType::UByte4, WgpuVertexFormat::Uint8x4),
        (D3dDeclType::T6UnitVec, WgpuVertexFormat::Uint32),
        (D3dDeclType::T6TexCoord, WgpuVertexFormat::Uint32),
        (D3dDeclType::T6WorldTexLmap, WgpuVertexFormat::Uint32x4),
    ];
    for (decl, expected) in cases {
        let layout = derive_wgpu_pass_layout(&abi(vec![attribute(0, 0, decl)]), &module())
            .unwrap_or_else(|why| panic!("{decl:?} refused: {why:?}"));
        assert_eq!(layout.vertex_buffers[0].attributes[0].format, expected, "{decl:?}");
    }
}

#[test]
fn an_unknown_declaration_type_is_refused() {
    let err = derive_wgpu_pass_layout(&abi(vec![attribute(0, 0, D3dDeclType::Unknown(77))]), &module())
        .unwrap_err();
    assert_eq!(err, WgpuLayoutRefusal::UnknownVertexFormat { source: 0, raw: 77 });
}

#[test]
fn two_attributes_may_not_share_a_location() {
    let err = derive_wgpu_pass_layout(
        &abi(vec![attribute(3, 0, D3dDeclType::Float2), attribute(3, 8, D3dDeclType::Float2)]),
        &module(),
    )
    .unwrap_err();
    assert_eq!(err, WgpuLayoutRefusal::DuplicateShaderLocation { location: 3 });
}

#[test]
fn an_attribute_may_not_run_past_the_stream() {
    let offset = (stride() - 8) as u8;
    let err = derive_wgpu_pass_layout(&abi(vec![attribute(0, offset, D3dDeclType::Float4)]), &module())
        .unwrap_err();
    assert_eq!(
        err,
        WgpuLayoutRefusal::VertexAttributePastStride {
            stream: 0,
            offset,
            byte_len: 16,
            stride: stride(),
        }
    );
}

#[test]
fn a_stream_with_no_extent_is_refused() {
    let mut odd = attribute(0, 0, D3dDeclType::Float2);
    odd.layout.stream = 14;
    let err = derive_wgpu_pass_layout(&abi(vec![odd]), &module()).unwrap_err();
    assert_eq!(err, WgpuLayoutRefusal::MissingStreamExtent { vertex_type: VERTEX_TYPE, stream: 14 });
}

// ---- retained draws -------------------------------------------------------

fn item(key: u64, rank: u32) -> RetainedDrawItem {
    RetainedDrawItem {
        material_id: None,
        key,
        material_rank: rank,
        kind: RetainedDrawKind::world(0),
        surface_samplers: SurfaceSamplerInputs::default(),
        camera_region: None,
    }
}

#[test]
fn the_host_sort_key_is_light_then_rank_then_low_key_bits() {
    let it = item((0xabc_u64 << 44) | 0xdead_beef, 77);
    assert_eq!(it.host_sort_key(), (0xabc, 77, 0xdead_beef));
    let high = item(u64::MAX, 0);
    assert_eq!(high.host_sort_key(), ((u64::MAX >> 44) as u32, 0, u32::MAX));
}

#[test]
fn a_plain_world_draw_is_one_surface_at_the_origin() {
    match RetainedDrawKind::world(41) {
        RetainedDrawKind::World { surf, run, run_off, bsp_kind, bsp_run_first, world_from_local } => {
            assert_eq!((surf, run, run_off), (41, 1, 0));
            assert!(bsp_kind.is_none() && bsp_run_first.is_none());
            assert_eq!(world_from_local, Mat4::IDENTITY);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn a_posed_world_draw_keeps_its_matrix() {
    let pose = Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0));
    match RetainedDrawKind::world_with_pose(3, pose) {
        RetainedDrawKind::World { surf, world_from_local, .. } => {
            assert_eq!(surf, 3);
            assert_eq!(world_from_local, pose);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn a_bsp_run_records_its_lane_and_first_surface() {
    match RetainedDrawKind::bsp_world(7, 5, 30, BspCameraLane::Decal, 6) {
        RetainedDrawKind::World { surf, run, run_off, bsp_kind, bsp_run_first, world_from_local } => {
            assert_eq!((surf, run, run_off), (7, 5, 30));
            assert_eq!(bsp_kind, Some(BspCameraLane::Decal));
            assert_eq!(bsp_run_first, Some(6));
            assert_eq!(world_from_local, Mat4::IDENTITY);
        }
        other => panic!("not a world draw: {other:?}"),
    }
}

#[test]
fn only_the_viewmodel_object_gets_the_depth_hack() {
    assert_eq!(host_viewmodel_render_fx_flags(XMODEL_OBJECT_ID_VIEWMODEL), RENDER_FX_DEPTH_HACK);
    assert_eq!(host_viewmodel_render_fx_flags(0), 0);
    assert_eq!(host_viewmodel_render_fx_flags(2), 0);
    assert_eq!((XMODEL_OBJECT_ID_VIEWMODEL, RENDER_FX_DEPTH_HACK), (1, 2));
}

// ---- texture bind identities ---------------------------------------------

fn port() -> PortId {
    PortId {
        vertex_program_hash: 1,
        pixel_program_hash: 2,
        vertex_decl_slot: 3,
        vertex_type: 4,
        custom_sampler_flags: 5,
        arguments_hash: 6,
    }
}

fn lane(register: u16, index: u32, state: u8, image: Option<u32>) -> PackedCodeSamplerLane {
    PackedCodeSamplerLane { register, index, sampler_state: state, image: image.map(RuntimeImageId) }
}

fn identity(lanes: &[PackedCodeSamplerLane]) -> TextureBindIdentity {
    TextureBindIdentity::from_lanes(port(), None, lanes, SurfaceSamplerInputs::default())
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
    let local = PackedLocalSamplers { id: 0x55, lanes: Vec::new() };
    let with = TextureBindIdentity::from_lanes(port(), Some(&local), &[], SurfaceSamplerInputs::default());
    assert_eq!(with.local, 0x55);
    assert_eq!(with.code, identity(&[]).code);
    assert_ne!(with, identity(&[]));
}

#[test]
fn the_surface_inputs_are_part_of_the_identity() {
    let lit = SurfaceSamplerInputs { primary_lightmap: Some(SurfaceLightmapId(0)), ..Default::default() };
    assert_ne!(TextureBindIdentity::from_lanes(port(), None, &[], lit), identity(&[]));
}

#[test]
fn from_packed_uses_the_stored_code_id() {
    let code = render_material::PackedCodeSamplers { id: 0x1234, lanes: Vec::new() };
    let id = TextureBindIdentity::from_packed(port(), None, &code, SurfaceSamplerInputs::default());
    assert_eq!((id.local, id.code), (0, 0x1234));
}
