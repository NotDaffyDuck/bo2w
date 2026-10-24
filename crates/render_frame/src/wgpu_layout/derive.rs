//! Layout derivation from a pass's declared vertex inputs.

use std::collections::{BTreeMap, BTreeSet};

use d3d9_sm3::PassWgsl;
use render_material::PassProgramAbi;
use render_material::decl_table::D3dDeclType;

use super::model::{
    WgpuBindLayoutEntry, WgpuBindingKind, WgpuLayoutRefusal, WgpuPassLayout, WgpuShaderVisibility,
    WgpuVertexAttribute, WgpuVertexBufferLayout, WgpuVertexFormat,
};
use super::table::texture_table_bind_entries;

/// Builds the vertex layouts (one per stream in use) and the bind entries (the
/// draw-data storage buffer at group 0, then the shared texture table).
///
/// Attributes are checked in declaration order, so the first problem found is
/// the one reported: a reused shader location, then an undecodable format, then
/// a stream with no known extent, then an attribute that overruns its stride.
pub fn derive_wgpu_pass_layout(
    abi: &PassProgramAbi,
    _module: &PassWgsl,
) -> Result<WgpuPassLayout, WgpuLayoutRefusal> {
    let mut streams: BTreeMap<u8, (u16, Vec<WgpuVertexAttribute>)> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    for attr in &abi.attributes {
        if !taken.insert(attr.location) {
            return Err(WgpuLayoutRefusal::DuplicateShaderLocation { location: attr.location });
        }
        let (format, byte_len) = wgpu_format(attr.source, attr.layout.decl_type)?;
        let stream = attr.layout.stream;
        let Some(stride) = abi.vertex_family.host_stream_stride(abi.vertex_type, stream) else {
            return Err(WgpuLayoutRefusal::MissingStreamExtent {
                vertex_type: abi.vertex_type,
                stream,
            });
        };
        let offset = attr.layout.offset;
        if u16::from(offset) + u16::from(byte_len) > stride {
            return Err(WgpuLayoutRefusal::VertexAttributePastStride {
                stream,
                offset,
                byte_len,
                stride,
            });
        }
        let (_, list) = streams.entry(stream).or_insert_with(|| (stride, Vec::new()));
        list.push(WgpuVertexAttribute {
            location: attr.location,
            offset: u64::from(offset),
            format,
        });
    }

    let vertex_buffers = streams
        .into_iter()
        .map(|(stream, (stride, mut attributes))| {
            attributes.sort_by_key(|a| (a.offset, a.location));
            WgpuVertexBufferLayout { stream, array_stride: u64::from(stride), attributes }
        })
        .collect();

    let draw_data = WgpuBindLayoutEntry {
        group: 0,
        binding: 0,
        visibility: WgpuShaderVisibility::VertexFragment,
        kind: WgpuBindingKind::ReadOnlyStorageBuffer,
        register_packed: None,
    };
    let bind_entries = std::iter::once(draw_data).chain(texture_table_bind_entries()).collect();
    Ok(WgpuPassLayout { vertex_buffers, bind_entries })
}

/// The GPU format for a declaration type, and its size in bytes.
fn wgpu_format(source: u8, decl: D3dDeclType) -> Result<(WgpuVertexFormat, u8), WgpuLayoutRefusal> {
    use WgpuVertexFormat as F;
    let format = match decl {
        D3dDeclType::Float2 => F::Float32x2,
        D3dDeclType::Float16x2 => F::Float16x2,
        D3dDeclType::Float3 => F::Float32x3,
        D3dDeclType::Float4 => F::Float32x4,
        D3dDeclType::D3dColor | D3dDeclType::UByte4N => F::Unorm8x4,
        D3dDeclType::UByte4 => F::Uint8x4,
        D3dDeclType::T6UnitVec | D3dDeclType::T6TexCoord => F::Uint32,
        D3dDeclType::T6WorldTexLmap => F::Uint32x4,
        D3dDeclType::Unknown(raw) => {
            return Err(WgpuLayoutRefusal::UnknownVertexFormat { source, raw });
        }
    };
    let size = decl.byte_len().expect("all accepted declaration types have a byte length");
    Ok((format, size))
}
