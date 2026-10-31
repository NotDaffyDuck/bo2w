//! Which vertex streams each of the 16 vertex types reads, and where the 13
//! routings of a vertex shader take their semantic from.
//!
//! A vertex type is a set of up to nine sources. Each source names the stream
//! it reads, the byte offset inside that stream's row and the element type.
//! The packed (1), world (2..=13, stream 0) and static-model cache (0xf)
//! types follow BO2's vertex rows as uploaded (vertex-format-t6.md): colour
//! R,G,B,A at 16 (cache: 12), half2 uv_semantic at 20, 10:10:10 normal and
//! tangent at 24 and 28, world lightmap coordinate at 32. The routing and the
//! remaining types are still the fallback family's.
//! TODO(fact): BO2's colour byte order (R,G,B,A kept, as the CPU had it).

pub const MIX_PATH_TOTAL: usize = 13;

pub const VERTEX_LAYOUT_KIND_TOTAL: usize = 16;

pub const VERTEX_STREAM_SLOT_COUNT: usize = 9;

pub const COMPRESSED_VERTEX_KIND: u8 = 1;

pub const WORLD_VERTEX_LAYOUT_ID: u8 = 2;

pub const POSITION_UV_VERTEX_KIND: u8 = 0xe;

pub const STATIC_CACHE_VERTEX_LAYOUT_ID: u8 = 0xf;

/// A technique with this flag takes its world vertex type from the surface.
pub const TECHNIQUE_FLAG_TAKES_SURFACE_VERTEX_LAYOUT: u16 = 0x20;

/// The world vertex type a technique draws a surface with: the plain one,
/// shifted by the surface's own format when the technique asks for that.
pub const fn world_decl_kind(tech_bits: u16, level_format: u8) -> u8 {
    let from_surface = tech_bits & TECHNIQUE_FLAG_TAKES_SURFACE_VERTEX_LAYOUT != 0;
    WORLD_VERTEX_LAYOUT_ID.saturating_add(if from_surface { level_format } else { 0 })
}

/// The vertex types a colour pass is tessellated with: world, then packed.
#[must_use]
pub fn colour_tess_vertex_types(tech_flags: u16, level_format: u8) -> [u8; 2] {
    [
        world_decl_kind(tech_flags, level_format),
        COMPRESSED_VERTEX_KIND,
    ]
}

/// The vertex types a colour pass has to compile for: the world type, its
/// layered twin (the next type up unless the surface already chose one),
/// packed, and the static-model cache.
#[must_use]
pub fn colour_tess_compile_vertex_types(tech_flags: u16, level_format: u8) -> [u8; 4] {
    let [world, packed] = colour_tess_vertex_types(tech_flags, level_format);
    let layered = match world {
        WORLD_VERTEX_LAYOUT_ID => WORLD_VERTEX_LAYOUT_ID.saturating_add(1),
        kept => kept,
    };
    [world, layered, packed, STATIC_CACHE_VERTEX_LAYOUT_ID]
}

/// The colour pass's compile set plus the position/uv_semantic type.
#[must_use]
pub fn emissive_tess_compile_vertex_types(tech_flags: u16, level_format: u8) -> [u8; 5] {
    let mut types = [POSITION_UV_VERTEX_KIND; 5];
    types[..4].copy_from_slice(&colour_tess_compile_vertex_types(
        tech_flags,
        level_format,
    ));
    types
}

pub use d3d9_decl::{
    VERTEX_USAGE_TINT, VERTEX_ROLE_DEPTH, VERTEX_ROLE_NORMAL, VERTEX_ROLE_POSITION,
    VERTEX_ROLE_UV, VertexElementFormat as VertexElemType, VertexInputRole,
};

/// Where one source of a vertex type reads its element from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertexStreamArrangement {
    pub stream: u8,
    pub offset: u8,
    pub decl_type: d3d9_decl::VertexElementFormat,
}

/// `feeds` with input `input` reading element type `raw_type` at `offset` of `stream_no`.
const fn with(mut feeds: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT], input: usize, stream_no: u8, offset: u8, raw_type: u8) -> [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] {
    feeds[input] = Some((stream_no, offset, raw_type));
    feeds
}

const NO_FEEDS: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] = [None; VERTEX_STREAM_SLOT_COUNT];

/// The five stream-0 sources every skinned/world row starts with; only the
/// uv_semantic element differs between the packed row and the world rows.
const fn model_row(texcoord_type: u8) -> [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] {
    let feeds = with(NO_FEEDS, 0, 0, 0x00, 0x03);
    let feeds = with(feeds, 1, 0, 0x10, 0x08);
    let feeds = with(feeds, 2, 0, 0x14, texcoord_type);
    let feeds = with(feeds, 3, 0, 0x18, 0x40);
    with(feeds, 4, 0, 0x1c, 0x40)
}

const WORLD: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] = model_row(0x42);
/// World rows whose lightmap coordinate lives in stream 1 as a float4.
const LIT_WIDE: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] = with(WORLD, 5, 1, 0x00, 0x03);
/// The wide-lightmap rows that go on with a second stream-1 element.
const LIT_WIDE_AND_TWO: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] = with(LIT_WIDE, 6, 1, 0x10, 0x01);
const LIT_WIDE_AND_FOUR: [Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT] = with(LIT_WIDE, 6, 1, 0x10, 0x03);

const FEEDS_BY_TYPE: [[Option<(u8, u8, u8)>; VERTEX_STREAM_SLOT_COUNT]; VERTEX_LAYOUT_KIND_TOTAL] = [
    with(
        with(
            with(with(NO_FEEDS, 0, 0, 0x00, 0x03), 1, 0, 0x10, 0x04),
            2,
            0,
            0x14,
            0x01,
        ),
        3,
        0,
        0x1c,
        0x05,
    ),
    model_row(0x41),
    WORLD,
    with(WORLD, 5, 1, 0x00, 0x01),
    with(with(WORLD, 5, 1, 0x00, 0x01), 7, 1, 0x08, 0x04),
    LIT_WIDE,
    with(LIT_WIDE, 7, 1, 0x10, 0x04),
    with(with(LIT_WIDE, 7, 1, 0x10, 0x04), 8, 1, 0x14, 0x04),
    LIT_WIDE_AND_TWO,
    with(LIT_WIDE_AND_TWO, 7, 1, 0x18, 0x04),
    with(with(LIT_WIDE_AND_TWO, 7, 1, 0x18, 0x04), 8, 1, 0x1c, 0x04),
    LIT_WIDE_AND_FOUR,
    with(LIT_WIDE_AND_FOUR, 7, 1, 0x20, 0x04),
    with(with(LIT_WIDE_AND_FOUR, 7, 1, 0x20, 0x04), 8, 1, 0x24, 0x04),
    with(with(NO_FEEDS, 0, 0, 0x00, 0x02), 2, 0, 0x0c, 0x01),
    {
        let feeds = with(NO_FEEDS, 0, 0, 0x00, 0x02);
        let feeds = with(feeds, 1, 0, 0x0c, 0x08);
        let feeds = with(feeds, 2, 0, 0x10, 0x41);
        let feeds = with(feeds, 3, 0, 0x14, 0x40);
        let feeds = with(feeds, 4, 0, 0x18, 0x40);
        with(feeds, 5, 0, 0x1c, 0x05)
    },
];

/// The (usage, usage index) a routing's vertex-shader input binds to:
/// routings 5 to 12 are the uv_semantic usage with consecutive indices.
pub const fn destination_usage(write_target: u8) -> Option<(u8, u8)> {
    Some(match write_target {
        0 => (0x00, 0),
        1 => (0x03, 0),
        2 => (0x0a, 0),
        3 => (0x0a, 1),
        4 => (0x0c, 0),
        5..=12 => (0x05, write_target - 5),
        _ => return None,
    })
}

pub const fn origin_layout(vertex_format_code: u8, source: u8) -> Option<VertexStreamArrangement> {
    if vertex_format_code as usize >= VERTEX_LAYOUT_KIND_TOTAL || source as usize >= VERTEX_STREAM_SLOT_COUNT {
        return None;
    }
    match FEEDS_BY_TYPE[vertex_format_code as usize][source as usize] {
        Some((stream, offset, raw_type)) => Some(VertexStreamArrangement {
            stream,
            offset,
            decl_type: VertexElemType::from_raw(raw_type),
        }),
        None => None,
    }
}

/// How many bytes of `wanted_stream`'s row the vertex type reads, or `None`
/// when it reads nothing from that stream.
pub const fn stream_extent(vertex_format_code: u8, wanted_stream: u8) -> Option<u16> {
    let mut extent = None;
    let mut source = 0;
    while source < VERTEX_STREAM_SLOT_COUNT as u8 {
        if let Some(layout) = origin_layout(vertex_format_code, source)
            && layout.stream == wanted_stream
            && let Some(len) = layout.decl_type.byte_len()
        {
            let reach = len as u16 + layout.offset as u16;
            if !matches!(extent, Some(seen) if seen >= reach) {
                extent = Some(reach);
            }
        }
        source += 1;
    }
    extent
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vertex-format-t6.md: the model row is 32 bytes, the world row 36
    /// (lightmap u16x2 @32), and stream 0 uploads them as they are.
    #[test]
    fn stream_zero_describes_bo2_rows() {
        assert_eq!(stream_extent(COMPRESSED_VERTEX_KIND, 0), Some(32));
        for vertex_format_code in WORLD_VERTEX_LAYOUT_ID..=13 {
            assert_eq!(stream_extent(vertex_format_code, 0), Some(36), "type {vertex_format_code}");
        }
        assert_eq!(stream_extent(STATIC_CACHE_VERTEX_LAYOUT_ID, 0), Some(32));
    }

    #[test]
    fn routings_five_to_twelve_count_up_from_the_texcoord_usage() {
        assert_eq!(destination_usage(5), Some((5, 0)));
        assert_eq!(destination_usage(12), Some((5, 7)));
        assert_eq!(destination_usage(13), None);
    }

    #[test]
    fn only_the_wide_lightmap_types_feed_from_stream_one_slot_five() {
        let wide = origin_layout(5, 5).unwrap();
        assert_eq!((wide.stream, wide.offset), (1, 0));
        assert!(origin_layout(2, 5).is_none());
        assert!(origin_layout(16, 0).is_none());
        assert!(origin_layout(2, 9).is_none());
    }

    #[test]
    fn the_compile_sets_extend_one_another() {
        let colour = colour_tess_compile_vertex_types(0, 0);
        assert_eq!(colour, [2, 3, 1, 0xf]);
        let emissive = emissive_tess_compile_vertex_types(0, 0);
        assert_eq!(emissive[..4], colour);
        assert_eq!(emissive[4], POSITION_UV_VERTEX_KIND);
        assert_eq!(
            colour_tess_compile_vertex_types(TECHNIQUE_FLAG_TAKES_SURFACE_VERTEX_LAYOUT, 3)[..2],
            [5, 5]
        );
    }
}
