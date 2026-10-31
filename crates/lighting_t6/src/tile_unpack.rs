//! The model-lighting atlas: how big it is, where each lit model's tile sits in
//! it, and how a colour set is unfolded into the tile that gets uploaded.
//!
//! The atlas is a 3D texture 256 texels wide, `image_height` tall and 4 deep.
//! A tile is a 4x4x4 block of it, so 64 tiles sit side by side in a row.

/// Texels along each edge of a tile.
pub const ILLUM_TILE_SIDE: usize = 4;
/// Texels in a tile.
const TILE_TEXELS: usize =
    ILLUM_TILE_SIDE * ILLUM_TILE_SIDE * ILLUM_TILE_SIDE;
/// Bytes in a tile: four bytes (R, G, B, weight) per texel.
pub const ILLUM_TILE_LEN: usize = TILE_TEXELS * 4;
/// Texels on the outside of a tile (the inner 2x2x2 is not stored).
const SHELL_TEXELS: usize = TILE_TEXELS - 8;
/// Bytes in a colour set: the shell, three bytes a texel.
const COLORS_BYTES: usize = SHELL_TEXELS * 3;

/// Texels across the atlas.
pub const MODEL_LIGHT_ATLAS_TEXELS_ACROSS: u32 = 256;
/// Texels through the atlas.
pub const MODEL_LIGHT_ATLAS_LAYERS: u32 = 4;
/// Where the middle of a tile sits through the depth (the 0 to 1 range spans
/// all four slices; the tile fills the first).
pub const MODEL_LIGHT_VOLUME_DEPTH_SPAN: f32 = 0.5;
/// One over the atlas width, for turning texels into 0 to 1 coordinates.
pub const ATLAS_INV_WIDTH: f64 = 1.0 / 256.0;

/// Tiles reserved for static models are never fewer than this.
const SMODEL_ENTRY_FLOOR: u32 = 0x800;
/// The atlas is never given more than 2^30 entries.
const MAX_TOTAL_BITS: u32 = 30;

/// A texel and a half: how far a lookup reaches into a tile (see
/// [`model_lighting_lookup_scale`]).
pub(crate) const LOOKUP_SPAN: f32 = 1.5;
/// Half a tile edge: the offset from a tile's corner to its middle.
const TILE_MIDDLE: f32 = 2.0;

/// Shader source slot of the model-lighting atlas.
pub const TEX_SLOT_ILLUM_ATLAS: u8 = 0x3;
/// Shader source slot of the light attenuation ramp.
pub const TEX_SLOT_LAMP_FALLOFF: u8 = 0xd;

/// The shell index of each corner of a tile, in the order the compressed
/// colour is averaged over.
pub const GRID_PACKED_DIRECTION_CELLS: [usize; 8] = [0, 3, 12, 15, 40, 43, 52, 55];

/// For each texel of a tile (x fastest, then y, then z), which shell entry of
/// a colour set it is filled from. Shell entries are numbered in the same
/// order as the outer texels, so the first and last plane are used whole. The
/// inner 2x2x2 reuses its nearest corner.
const TEXEL_TO_SHELL: [u8; TILE_TEXELS] = build_texel_to_shell();

const fn build_texel_to_shell() -> [u8; TILE_TEXELS] {
    let dim = ILLUM_TILE_SIDE;
    let mut table = [0u8; TILE_TEXELS];
    let mut next_shell = 0u8;
    let mut texel = 0;
    while texel < TILE_TEXELS {
        let x = texel % dim;
        let y = texel / dim % dim;
        let z = texel / (dim * dim);
        let outer = x % (dim - 1) == 0 || y % (dim - 1) == 0 || z % (dim - 1) == 0;
        if outer {
            table[texel] = next_shell;
            next_shell += 1;
        } else {
            // The corner on the near side of the middle along each axis.
            let corner_x = if x < dim / 2 { 0 } else { dim - 1 };
            let corner_y = if y < dim / 2 { 0 } else { dim - 1 };
            let plane_start = if z < dim / 2 {
                0
            } else {
                SHELL_TEXELS - dim * dim
            };
            table[texel] = (plane_start + corner_y * dim + corner_x) as u8;
        }
        texel += 1;
    }
    table
}

/// Unfolds a colour set into a tile's texels as `[r, g, b, weight]`. False if
/// `colors` is shorter than a colour set or `out` shorter than a tile.
pub fn widen_shell_into_tile(colors: &[u8], share: u8, dest: &mut [u8]) -> bool {
    let (Some(shell), Some(tile)) = (colors.get(..COLORS_BYTES), dest.get_mut(..ILLUM_TILE_LEN)) else {
        return false;
    };
    for (texel, &from) in tile.chunks_exact_mut(4).zip(&TEXEL_TO_SHELL) {
        let rgb = &shell[usize::from(from) * 3..][..3];
        texel[..3].copy_from_slice(rgb);
        texel[3] = share;
    }
    true
}
/// How many tiles the atlas holds and how tall it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelIllumAtlasDims {
    /// Tiles reserved for dynamic models.
    pub figure_entry_limit: u32,
    /// Tiles left over for static models.
    pub prop_entry_limit: u32,
    /// Texels tall the atlas image is.
    pub rows_tall: u32,
}

/// Bits needed to count to `n`: the position of its top set bit, plus one.
const fn bit_length(n: u32) -> u32 {
    u32::BITS - n.leading_zeros()
}

/// Sizes the atlas for `max_simultaneous_views` views: 1024 dynamic tiles each, and
/// the total the smallest power of two above them that still leaves at least
/// 0x800 for static models. `None` for no views, or if it would take more than
/// 2^30 entries.
pub const fn illum_atlas_dims(max_simultaneous_views: u32) -> Option<ModelIllumAtlasDims> {
    const DYNAMIC_PER_VIEW_SHIFT: u32 = 10;
    if max_simultaneous_views == 0 {
        return None;
    }
    let dynamic = max_simultaneous_views << DYNAMIC_PER_VIEW_SHIFT;
    let above_dynamic = bit_length(dynamic);
    if above_dynamic > MAX_TOTAL_BITS {
        return None;
    }
    // The smallest power of two holding the dynamic tiles plus the static floor.
    let with_floor = bit_length(dynamic + SMODEL_ENTRY_FLOOR - 1);
    let bits = if with_floor > above_dynamic { with_floor } else { above_dynamic };
    if bits > MAX_TOTAL_BITS {
        return None;
    }
    // 64 tiles fill a band of tiles 4 texels tall, so the image needs one
    // texel row for every 16 entries.
    Some(ModelIllumAtlasDims {
        figure_entry_limit: dynamic,
        prop_entry_limit: (1u32 << bits).wrapping_sub(dynamic),
        rows_tall: (1u32 << bits) / 16,
    })
}

/// One over the atlas image height, `None` for a zero height.
pub fn light_atlas_reciprocal_height(image_height: u32) -> Option<f32> {
    (image_height != 0).then(|| 1.0 / image_height as f32)
}

/// Where a tile sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllumTileNo {
    pub raw: u16,
}

/// Tiles side by side across the atlas.
const TILES_PER_ROW: u32 = MODEL_LIGHT_ATLAS_TEXELS_ACROSS / ILLUM_TILE_SIDE as u32;

impl IllumTileNo {
    /// The tile a lighting handle names. Handles count from 1; 0 is "none".
    pub fn from_handle(handle: u16) -> Option<Self> {
        let raw = handle.checked_sub(1)?;
        Some(IllumTileNo { raw })
    }

    /// The tile's corner in the atlas, in texels. Tiles run along a row of 64
    /// before starting the next.
    pub fn texel_origin(self) -> (u32, u32) {
        let edge = ILLUM_TILE_SIDE as u32;
        let (row, column) = (u32::from(self.raw) / TILES_PER_ROW, u32::from(self.raw) % TILES_PER_ROW);
        (edge * column, edge * row)
    }
}

/// Where in the atlas a tile's middle is, as the shader wants it: `[u, v, w, q]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelIlluminationCoords3d {
    pub u: f32,
    pub v: f32,
    pub w: f32,
    pub q: f32,
}

impl ModelIlluminationCoords3d {
    pub fn as_array(self) -> [f32; 4] {
        let Self { u, v, w, q } = self;
        [u, v, w, q]
    }
}

/// The coordinates for the tile a handle names. `None` for handle 0.
pub fn light_atlas_coords_for_handle(
    handle: u16,
    recip_image_height: f32,
) -> Option<ModelIlluminationCoords3d> {
    let tile = IllumTileNo::from_handle(handle)?;
    let (x, y) = tile.texel_origin();
    let middle = |corner: u32, inverse: f32| (corner as f32 + TILE_MIDDLE) * inverse;
    let inv_width = ATLAS_INV_WIDTH as f32;
    Some(ModelIlluminationCoords3d { u: middle(x, inv_width), v: middle(y, recip_image_height), w: MODEL_LIGHT_VOLUME_DEPTH_SPAN, q: 1.0 })
}

/// How the shader scales a tile-local lookup into atlas coordinates:
/// `[u, v, w, q]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelIlluminationLookupFactor {
    pub u: f32,
    pub v: f32,
    pub w: f32,
    pub q: f32,
}

impl ModelIlluminationLookupFactor {
    pub fn as_array(self) -> [f32; 4] {
        let Self { u, v, w, q } = self;
        [u, v, w, q]
    }
}

/// A lookup reaches a texel and a half along each axis, so the scale is that
/// over the atlas's size on the axis.
pub fn atlas_lookup_scale(recip_image_height: f32) -> ModelIlluminationLookupFactor {
    let depth = MODEL_LIGHT_ATLAS_LAYERS as f32;
    let across = LOOKUP_SPAN * ATLAS_INV_WIDTH as f32;
    ModelIlluminationLookupFactor { u: across, v: LOOKUP_SPAN * recip_image_height, w: LOOKUP_SPAN / depth, q: 0.0 }
}

/// Copies a tile into the atlas's bytes (RGBA, one slice per tile layer).
/// False if `atlas` is too small for an image `atlas_rows` tall or the
/// tile lies below its bottom.
pub fn blit_illum_tile(
    atlas: &mut [u8],
    atlas_rows: u32,
    entry: IllumTileNo,
    tile: &[u8; ILLUM_TILE_LEN],
) -> bool {
    const BYTES_PER_TEXEL: usize = 4;
    let edge = ILLUM_TILE_SIDE;
    let atlas_row = MODEL_LIGHT_ATLAS_TEXELS_ACROSS as usize * BYTES_PER_TEXEL;
    let atlas_slice = atlas_row * atlas_rows as usize;
    let (col0, row0) = entry.texel_origin();
    let (col0, row0) = (col0 as usize, row0 as usize);
    let fits = row0 + edge <= atlas_rows as usize
        && atlas.len() >= atlas_slice * MODEL_LIGHT_ATLAS_LAYERS as usize;
    if !fits {
        return false;
    }
    // The tile's rows run x fastest, then y, then z: one atlas row each.
    let tile_row = edge * BYTES_PER_TEXEL;
    for (n, source) in tile.chunks_exact(tile_row).enumerate() {
        let (layer, line) = (n / edge, n % edge);
        let start = layer * atlas_slice + (row0 + line) * atlas_row + col0 * BYTES_PER_TEXEL;
        atlas[start..start + tile_row].copy_from_slice(source);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_set() -> [u8; COLORS_BYTES] {
        let mut set = [0u8; COLORS_BYTES];
        for (shell, rgb) in set.chunks_exact_mut(3).enumerate() {
            rgb.copy_from_slice(&[shell as u8, 100 + shell as u8, 200]);
        }
        set
    }

    fn xyz(texel: usize) -> [usize; 3] {
        [texel % 4, texel / 4 % 4, texel / 16]
    }

    #[test]
    fn the_outer_texels_take_the_shell_in_order() {
        let outer: Vec<u8> = (0..TILE_TEXELS)
            .filter(|&t| xyz(t).iter().any(|&a| a == 0 || a == 3))
            .map(|t| TEXEL_TO_SHELL[t])
            .collect();
        assert_eq!(outer, (0..SHELL_TEXELS as u8).collect::<Vec<_>>());
    }

    #[test]
    fn the_inner_texels_copy_their_nearest_corner() {
        // An inner texel, and the corner it sits nearest.
        let pairs = [(21, 0), (22, 3), (25, 12), (26, 15), (37, 40), (38, 43), (41, 52), (42, 55)];
        for (texel, corner) in pairs {
            assert_eq!(usize::from(TEXEL_TO_SHELL[texel]), corner, "texel {texel}");
        }
    }

    #[test]
    fn the_compress_dirs_are_the_tile_corners() {
        let corner_shells = [0, 3, 12, 15, 48, 51, 60, 63].map(|t| usize::from(TEXEL_TO_SHELL[t]));
        assert_eq!(corner_shells, GRID_PACKED_DIRECTION_CELLS);
    }

    #[test]
    fn expanding_fills_every_texel_with_its_shell_colour_and_the_weight() {
        let mut texels = [0u8; ILLUM_TILE_LEN];
        assert!(widen_shell_into_tile(&colour_set(), 0x7f, &mut texels));
        let texel = |n: usize| &texels[n * 4..n * 4 + 4];
        // Texel 23 is the 22nd outer texel (shell entry 21).
        assert_eq!(texel(23), &[21, 121, 200, 0x7f]);
        // Texel 21 is inside, so it repeats corner 0.
        assert_eq!(texel(21), &[0, 100, 200, 0x7f]);
        assert!(texels.chunks_exact(4).all(|t| t[3] == 0x7f));
    }

    #[test]
    fn expanding_refuses_short_buffers() {
        let set = colour_set();
        let mut texels = [0u8; ILLUM_TILE_LEN];
        assert!(!widen_shell_into_tile(&set[..167], 0, &mut texels));
        assert!(!widen_shell_into_tile(&set, 0, &mut texels[..255]));
    }

    fn dims(views: u32) -> (u32, u32, u32) {
        let d = illum_atlas_dims(views).unwrap();
        (d.figure_entry_limit, d.prop_entry_limit, d.rows_tall)
    }

    #[test]
    fn the_atlas_grows_to_keep_the_static_floor() {
        // One view: a 4096 entry atlas. Two already leave enough; three double it.
        assert_eq!(dims(1), (1024, 3072, 256));
        assert_eq!(dims(2), (2048, 2048, 256));
        assert_eq!(dims(3), (3072, 8192 - 3072, 512));
    }

    #[test]
    fn no_views_or_too_many_give_no_atlas() {
        assert_eq!(illum_atlas_dims(0), None);
        assert_eq!(illum_atlas_dims(1 << 21), None);
        assert_eq!(illum_atlas_dims(1 << 19).map(|d| d.rows_tall), Some(1 << 26));
    }

    #[test]
    fn inverse_height_is_one_over_it() {
        assert_eq!([256, 0].map(light_atlas_reciprocal_height), [Some(0.00390625), None]);
    }

    #[test]
    fn handles_count_from_one() {
        assert_eq!(IllumTileNo::from_handle(0), None);
        assert_eq!(IllumTileNo::from_handle(1), Some(IllumTileNo { raw: 0 }));
    }

    #[test]
    fn tiles_run_across_a_row_of_64_then_down() {
        let origin = |handle| IllumTileNo::from_handle(handle).unwrap().texel_origin();
        let got = [1, 2, 64, 65, 66].map(origin);
        assert_eq!(got, [(0, 0), (4, 0), (252, 0), (0, 4), (4, 4)]);
    }

    #[test]
    fn coordinates_aim_at_the_middle_of_the_tile() {
        let at = |handle| light_atlas_coords_for_handle(handle, 1.0 / 256.0).map(ModelIlluminationCoords3d::as_array);
        assert_eq!(at(1), Some([2.0 / 256.0, 2.0 / 256.0, 0.5, 1.0]));
        assert_eq!(at(66), Some([6.0 / 256.0, 6.0 / 256.0, 0.5, 1.0]));
        assert_eq!(light_atlas_coords_for_handle(0, 1.0), None);
    }

    #[test]
    fn the_lookup_scale_is_a_texel_and_a_half_per_axis() {
        assert_eq!(atlas_lookup_scale(1.0 / 512.0).as_array(), [0.005859375, 1.5 / 512.0, 0.375, 0.0]);
    }

    fn patterned_tile() -> [u8; ILLUM_TILE_LEN] {
        core::array::from_fn(|index| (index % 250) as u8 + 1)
    }

    #[test]
    fn a_tile_lands_in_its_block_of_every_slice() {
        let height = 8u32;
        let slice = 1024 * height as usize;
        let mut atlas = vec![0u8; slice * 4];
        let tile = patterned_tile();
        // Handle 66 is the tile at texel (4, 4).
        let entry = IllumTileNo::from_handle(66).unwrap();
        assert!(blit_illum_tile(&mut atlas, height, entry, &tile));
        for (layer, line) in (0..4).flat_map(|z| (0..4).map(move |r| (z, r))) {
            let at = layer * slice + (4 + line) * 1024 + 16;
            let from = (layer * 4 + line) * 16;
            assert_eq!(&atlas[at..at + 16], &tile[from..from + 16], "z {layer} row {line}");
        }
        assert_eq!(atlas.iter().filter(|&&b| b != 0).count(), ILLUM_TILE_LEN);
    }

    #[test]
    fn a_tile_below_the_atlas_or_a_small_buffer_is_refused() {
        let tile = patterned_tile();
        let mut atlas = vec![0u8; 4096 * 8];
        // Handle 129 starts at texel row 8: past an 8-tall atlas.
        let below = IllumTileNo::from_handle(129).unwrap();
        assert!(!blit_illum_tile(&mut atlas, 8, below, &tile));
        let first = IllumTileNo::from_handle(1).unwrap();
        assert!(!blit_illum_tile(&mut atlas[..4096 * 8 - 1], 8, first, &tile));
        assert!(atlas.iter().all(|&b| b == 0));
    }
}
