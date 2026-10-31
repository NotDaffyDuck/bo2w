use crate::decl_table::{VertexElemType, VERTEX_STREAM_SLOT_COUNT, VertexStreamArrangement, destination_usage, origin_layout, stream_extent};
use fastfile_t5::vtx_formats::STREAM_SLOT_PLANS;
use fastfile_t5::vtx_formats::TARGET_USE_LOOKUP;

/// Which vertex format table a draw reads its layout from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VertexFormatLineage {
    #[default]
    Fallback,
    T5,
}

/// Per-vertex bytes in the second stream of a T5 world surface.
pub const LEGACY_WORLD_LAYER_VERTEX_BYTES: usize = 40;

const T5_SOURCE_COUNT: usize = 10;

impl VertexFormatLineage {
    pub fn input_source_total(self) -> usize {
        match self {
            Self::Fallback => VERTEX_STREAM_SLOT_COUNT,
            Self::T5 => T5_SOURCE_COUNT,
        }
    }

    pub fn write_target_usage(self, write_target: u8) -> Option<(u8, u8)> {
        match self {
            Self::Fallback => destination_usage(write_target),
            VertexFormatLineage::T5 => {
                let &[usage, index] = TARGET_USE_LOOKUP.get(usize::from(write_target))?;
                Some((usage, index))
            }
        }
    }

    pub fn origin_layout(self, vertex_format_code: u8, source: u8) -> Option<VertexStreamArrangement> {
        match self {
            Self::Fallback => origin_layout(vertex_format_code, source),
            VertexFormatLineage::T5 => {
                let row = STREAM_SLOT_PLANS.get(usize::from(vertex_format_code))?;
                let &[strm, offset, raw] = row.get(usize::from(source))?;
                if strm == 255 {
                    return None;
                }
                Some(VertexStreamArrangement {
                    stream: strm,
                    offset,
                    decl_type: VertexElemType::from_raw(raw),
                })
            }
        }
    }

    /// Bytes per vertex that the host keeps in stream `lane_no` for this vertex type.
    pub fn host_stride_bytes(self, vertex_format_code: u8, lane_no: u8) -> Option<u16> {
        match self {
            Self::Fallback => stream_extent(vertex_format_code, lane_no),
            Self::T5 if lane_no == 1 && (2..=13).contains(&vertex_format_code) => {
                Some(LEGACY_WORLD_LAYER_VERTEX_BYTES as u16)
            }
            Self::T5 => (0..T5_SOURCE_COUNT as u8)
                .filter_map(|source| self.origin_layout(vertex_format_code, source))
                .filter(|layout| layout.stream == lane_no)
                .filter_map(|layout| {
                    let size = layout.decl_type.byte_len()?;
                    Some(u16::from(layout.offset) + u16::from(size))
                })
                .max(),
        }
    }
}
