//! Layout derivation from a pass's declared vertex inputs.

use std::collections::{BTreeMap, BTreeSet};

use d3d9_sm3::PassShaderText;
use render_material::PassShaderContract;
use render_material::decl_table::VertexElemType;

use super::model::{
    GpuBindSlotDescriptor, GpuBindResourceKind, WgpuLayoutRejection, GpuPassBindingPlan, GpuStageVisibility,
    GpuVertexField, GpuVertexStreamLayout, GpuVertexElementKind,
};
use super::table::table_bind_entries;

/// Builds the vertex layouts (one per stream in use) and the bind entries (the
/// draw-data storage buffer at group 0, then the shared texture table).
///
/// Attributes are checked in declaration order, so the first problem found is
/// the one reported: a reused shader location, then an undecodable format, then
/// a stream with no known extent, then an attribute that overruns its stride.
pub fn plan_gpu_pass_bindings(
    abi: &PassShaderContract,
    _module: &PassShaderText,
) -> Result<GpuPassBindingPlan, WgpuLayoutRejection> {
    let mut streams: BTreeMap<u8, (u16, Vec<GpuVertexField>)> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for attr in &abi.attributes {
        if !taken.insert(attr.location) {
            return Err(WgpuLayoutRejection::DuplicateShaderLocation { location: attr.location });
        }
        let (wgpu_fmt, byte_len) = wgpu_format(attr.source, attr.layout.decl_type)?;
        let stream = attr.layout.stream;
        let Some(stride) = abi.vertex_family.host_stride_bytes(abi.vertex_format_code, stream) else {
            return Err(WgpuLayoutRejection::StreamExtentAbsent {
                vertex_format_code: abi.vertex_format_code,
                stream,
            });
        };
        let offset = attr.layout.offset;
        if u16::from(offset) + u16::from(byte_len) > stride {
            return Err(WgpuLayoutRejection::AttrOverrunsStride {
                stream,
                offset,
                size: byte_len,
                stride_bytes: stride,
            });
        }
        let (_, attrs_for) = streams.entry(stream).or_insert_with(|| (stride, Vec::new()));
        attrs_for.push(GpuVertexField {
            location: attr.location,
            offset: u64::from(offset),
            format: wgpu_fmt,
        });
    }

    let stream_layouts = streams
        .into_iter()
        .map(|(stream_no, (stride, mut attributes))| {
            attributes.sort_by_key(|a| (a.offset, a.location));
            GpuVertexStreamLayout { slot: stream_no, array_stride: u64::from(stride), attributes }
        })
        .collect();

    let draw_data = GpuBindSlotDescriptor {
        band: 0,
        binding: 0,
        visibility: GpuStageVisibility::VertexAndPixelStage,
        kind: GpuBindResourceKind::ReadableDataBuffer,
        register_packed: None,
    };
    let slot_list = std::iter::once(draw_data).chain(table_bind_entries()).collect();
    Ok(GpuPassBindingPlan { vertex_buffers: stream_layouts, bind_entries: slot_list })
}

/// The GPU format for a declaration type, and its size in bytes.
fn wgpu_format(stream_slot: u8, decl: VertexElemType) -> Result<(GpuVertexElementKind, u8), WgpuLayoutRejection> {
    let format = match decl {
        VertexElemType::TwoFloats => GpuVertexElementKind::Float32x2,
        VertexElemType::Float16x2 => GpuVertexElementKind::Float16x2,
        VertexElemType::ThreeFloats => GpuVertexElementKind::Float32x3,
        VertexElemType::QuadFloat => GpuVertexElementKind::Float32x4,
        VertexElemType::PackedColorBytes | VertexElemType::FourBytesNormalised => GpuVertexElementKind::Unorm8x4,
        VertexElemType::FourBytesRaw => GpuVertexElementKind::Uint8x4,
        VertexElemType::T6UnitVec | VertexElemType::T6TexCoord => GpuVertexElementKind::Uint32,
        VertexElemType::T6WorldTexLmap => GpuVertexElementKind::Uint32x4,
        VertexElemType::Unknown(decl_code) => {
            return Err(WgpuLayoutRejection::UnlistedVertexFormat { stream_slot, decl_code });
        }
    };
    let size = decl.byte_len().expect("all accepted declaration types have a byte length");
    Ok((format, size))
}
