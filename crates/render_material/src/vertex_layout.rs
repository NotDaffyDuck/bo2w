use crate::decl_table as decl;
use fastfile_t5::vertex_decl as t5;

/// Which vertex format table a draw reads its layout from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VertexLayoutFamily {
    #[default]
    Fallback,
    T5,
}

/// Per-vertex bytes in the second stream of a T5 world surface.
pub const T5_WORLD_LAYER_HOST_STRIDE: usize = 40;

const T5_SOURCE_COUNT: usize = 10;

impl VertexLayoutFamily {
    pub fn source_count(self) -> usize {
        match self {
            Self::Fallback => decl::STREAM_SOURCE_COUNT,
            Self::T5 => T5_SOURCE_COUNT,
        }
    }

    pub fn destination_usage(self, dest: u8) -> Option<(u8, u8)> {
        match self {
            Self::Fallback => decl::destination_usage(dest),
            Self::T5 => {
                let &[usage, index] = t5::DESTINATION_USAGE_TABLE.get(usize::from(dest))?;
                Some((usage, index))
            }
        }
    }

    pub fn source_layout(self, vertex_type: u8, source: u8) -> Option<decl::StreamSourceLayout> {
        match self {
            Self::Fallback => decl::source_layout(vertex_type, source),
            Self::T5 => {
                let row = t5::SOURCE_LAYOUT_TABLE.get(usize::from(vertex_type))?;
                let &[stream, offset, raw] = row.get(usize::from(source))?;
                if stream == 255 {
                    return None;
                }
                Some(decl::StreamSourceLayout {
                    stream,
                    offset,
                    decl_type: decl::D3dDeclType::from_raw(raw),
                })
            }
        }
    }

    /// Bytes per vertex that the host keeps in `stream` for this vertex type.
    pub fn host_stream_stride(self, vertex_type: u8, stream: u8) -> Option<u16> {
        match self {
            Self::Fallback => decl::stream_extent(vertex_type, stream),
            Self::T5 if stream == 1 && (2..=13).contains(&vertex_type) => {
                Some(T5_WORLD_LAYER_HOST_STRIDE as u16)
            }
            Self::T5 => (0..T5_SOURCE_COUNT as u8)
                .filter_map(|source| self.source_layout(vertex_type, source))
                .filter(|layout| layout.stream == stream)
                .filter_map(|layout| {
                    let size = layout.decl_type.byte_len()?;
                    Some(u16::from(layout.offset) + u16::from(size))
                })
                .max(),
        }
    }
}
