//! The model-lighting atlas: how big it is, where each lit model's tile sits in
//! it, and how a colour set is unfolded into the tile that gets uploaded.
//!
//! The atlas is a 3D texture 256 texels wide, `image_height` tall and 4 deep.
//! A tile is a 4x4x4 block of it, so 64 tiles sit side by side in a row.

/// Texels along each edge of a tile.
pub const MODEL_LIGHTING_TILE_DIM: usize = 4;
/// Texels in a tile.
const TILE_TEXELS: usize =
    MODEL_LIGHTING_TILE_DIM * MODEL_LIGHTING_TILE_DIM * MODEL_LIGHTING_TILE_DIM;
/// Bytes in a tile: four bytes (R, G, B, weight) per texel.
pub const MODEL_LIGHTING_TILE_BYTES: usize = TILE_TEXELS * 4;
/// Texels on the outside of a tile (the inner 2x2x2 is not stored).
const SHELL_TEXELS: usize = TILE_TEXELS - 8;
/// Bytes in a colour set: the shell, three bytes a texel.
const COLORS_BYTES: usize = SHELL_TEXELS * 3;

/// Texels across the atlas.
pub const MODEL_LIGHTING_ATLAS_WIDTH: u32 = 256;
/// Texels through the atlas.
pub const MODEL_LIGHTING_ATLAS_DEPTH: u32 = 4;
/// Where the middle of a tile sits through the depth (the 0 to 1 range spans
/// all four slices; the tile fills the first).
pub const MODEL_LIGHTING_VOLUME_W: f32 = 0.5;
/// One over the atlas width, for turning texels into 0 to 1 coordinates.
pub const MODEL_LIGHTING_INV_ATLAS_WIDTH: f64 = 1.0 / 256.0;

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
pub const TEXTURE_SRC_CODE_MODEL_LIGHTING: u8 = 0x3;
/// Shader source slot of the light attenuation ramp.
pub const TEXTURE_SRC_CODE_LIGHT_ATTENUATION: u8 = 0xd;

/// The shell index of each corner of a tile, in the order the compressed
/// colour is averaged over.
pub const LIGHT_GRID_COMPRESS_DIRS: [usize; 8] = [0, 3, 12, 15, 40, 43, 52, 55];

/// For each texel of a tile (x fastest, then y, then z), which shell entry of
/// a colour set it is filled from. Shell entries are numbered in the same
/// order as the outer texels, so the first and last plane are used whole. The
/// inner 2x2x2 reuses its nearest corner.
const TEXEL_TO_SHELL: [u8; TILE_TEXELS] = build_texel_to_shell();

const fn build_texel_to_shell() -> [u8; TILE_TEXELS] {
    let dim = MODEL_LIGHTING_TILE_DIM;
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
pub fn light_grid_expand_shell_to_tile_rgba(colors: &[u8], weight: u8, out: &mut [u8]) -> bool {
    if colors.len() < COLORS_BYTES || out.len() < MODEL_LIGHTING_TILE_BYTES {
        return false;
    }
    for (texel, &shell) in TEXEL_TO_SHELL.iter().enumerate() {
        let from = usize::from(shell) * 3;
        let to = texel * 4;
        out[to..to + 4].copy_from_slice(&[
            colors[from],
            colors[from + 1],
            colors[from + 2],
            weight,
        ]);
    }
    true
}

/// How many tiles the atlas holds and how tall it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLightingAtlasDims {
    /// Tiles reserved for dynamic models.
    pub xmodel_entry_limit: u32,
    /// Tiles left over for static models.
    pub smodel_entry_limit: u32,
    /// Texels tall the atlas image is.
    pub image_height: u32,
}

/// Sizes the atlas for `max_client_views` views: 1024 dynamic tiles each, and
/// the total grown by doubling until at least 0x800 are left for static
/// models. `None` for no views, or if it would take more than 2^30 entries.
pub const fn model_lighting_atlas_dims(max_client_views: u32) -> Option<ModelLightingAtlasDims> {
    if max_client_views == 0 {
        return None;
    }
    let xmodel_entry_limit = max_client_views << 10;
    let mut total_bits = u32::BITS - xmodel_entry_limit.leading_zeros();
    loop {
        if total_bits > MAX_TOTAL_BITS {
            return None;
        }
        let smodel_entry_limit = (1u32 << total_bits).wrapping_sub(xmodel_entry_limit);
        if smodel_entry_limit >= SMODEL_ENTRY_FLOOR {
            // 64 tiles fill a band of tiles 4 texels tall, so the image needs
            // one texel row for every 16 entries.
            return Some(ModelLightingAtlasDims {
                xmodel_entry_limit,
                smodel_entry_limit,
                image_height: 1 << (total_bits - 4),
            });
        }
        total_bits += 1;
    }
}

/// One over the atlas image height, `None` for a zero height.
pub fn model_lighting_inv_image_height(image_height: u32) -> Option<f32> {
    if image_height == 0 {
        return None;
    }
    Some(1.0 / image_height as f32)
}

/// Where a tile sits in the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelLightingTileIndex {
    pub raw: u16,
}

impl ModelLightingTileIndex {
    /// The tile a lighting handle names. Handles count from 1; 0 is "none".
    pub fn from_handle(handle: u16) -> Option<Self> {
        handle.checked_sub(1).map(|raw| Self { raw })
    }

    /// The tile's corner in the atlas, in texels. Tiles run along a row of 64
    /// before starting the next.
    pub fn texel_origin(self) -> (u32, u32) {
        let dim = MODEL_LIGHTING_TILE_DIM as u32;
        let per_row = MODEL_LIGHTING_ATLAS_WIDTH / dim;
        let raw = u32::from(self.raw);
        (raw % per_row * dim, raw / per_row * dim)
    }
}

/// Where in the atlas a tile's middle is, as the shader wants it: `[u, v, w, q]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLightingCoords {
    pub u: f32,
    pub v: f32,
    pub w: f32,
    pub q: f32,
}

impl ModelLightingCoords {
    pub fn as_array(self) -> [f32; 4] {
        [self.u, self.v, self.w, self.q]
    }
}

/// The coordinates for the tile a handle names. `None` for handle 0.
pub fn model_lighting_coords_from_handle(
    handle: u16,
    inv_image_height: f32,
) -> Option<ModelLightingCoords> {
    let (x, y) = ModelLightingTileIndex::from_handle(handle)?.texel_origin();
    Some(ModelLightingCoords {
        u: (x as f32 + TILE_MIDDLE) * MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
        v: (y as f32 + TILE_MIDDLE) * inv_image_height,
        w: MODEL_LIGHTING_VOLUME_W,
        q: 1.0,
    })
}

/// How the shader scales a tile-local lookup into atlas coordinates:
/// `[u, v, w, q]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLightingLookupScale {
    pub u: f32,
    pub v: f32,
    pub w: f32,
    pub q: f32,
}

impl ModelLightingLookupScale {
    pub fn as_array(self) -> [f32; 4] {
        [self.u, self.v, self.w, self.q]
    }
}

/// A lookup reaches a texel and a half along each axis, so the scale is that
/// over the atlas's size on the axis.
pub fn model_lighting_lookup_scale(inv_image_height: f32) -> ModelLightingLookupScale {
    ModelLightingLookupScale {
        u: LOOKUP_SPAN * MODEL_LIGHTING_INV_ATLAS_WIDTH as f32,
        v: LOOKUP_SPAN * inv_image_height,
        w: LOOKUP_SPAN / MODEL_LIGHTING_ATLAS_DEPTH as f32,
        q: 0.0,
    }
}

/// Copies a tile into the atlas's bytes (RGBA, one slice per tile layer).
/// False if `atlas` is too small for an image `image_height` tall or the
/// tile lies below its bottom.
pub fn model_lighting_write_tile_to_atlas(
    atlas: &mut [u8],
    image_height: u32,
    entry: ModelLightingTileIndex,
    tile: &[u8; MODEL_LIGHTING_TILE_BYTES],
) -> bool {
    let dim = MODEL_LIGHTING_TILE_DIM;
    let row_pitch = MODEL_LIGHTING_ATLAS_WIDTH as usize * 4;
    let slice_pitch = row_pitch * image_height as usize;
    let depth = MODEL_LIGHTING_ATLAS_DEPTH as usize;
    let (x, y) = entry.texel_origin();
    let (x, y) = (x as usize, y as usize);
    if atlas.len() < slice_pitch * depth || y + dim > image_height as usize {
        return false;
    }
    let row_bytes = dim * 4;
    for z in 0..dim {
        for row in 0..dim {
            let from = (z * dim + row) * row_bytes;
            let to = z * slice_pitch + (y + row) * row_pitch + x * 4;
            atlas[to..to + row_bytes].copy_from_slice(&tile[from..from + row_bytes]);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_set() -> [u8; COLORS_BYTES] {
        let mut set = [0u8; COLORS_BYTES];
        for shell in 0..SHELL_TEXELS {
            set[shell * 3] = shell as u8;
            set[shell * 3 + 1] = 100 + shell as u8;
            set[shell * 3 + 2] = 200;
        }
        set
    }

    #[test]
    fn the_outer_texels_take_the_shell_in_order() {
        let mut seen = Vec::new();
        for texel in 0..TILE_TEXELS {
            let (x, y, z) = (texel % 4, texel / 4 % 4, texel / 16);
            if [x, y, z].iter().any(|&axis| axis == 0 || axis == 3) {
                seen.push(TEXEL_TO_SHELL[texel]);
            }
        }
        assert_eq!(seen, (0..SHELL_TEXELS as u8).collect::<Vec<_>>());
    }

    #[test]
    fn the_inner_texels_copy_their_nearest_corner() {
        // (x, y, z) of an inner texel, and the corner it sits nearest.
        for (texel, corner) in [
            (21, 0),
            (22, 3),
            (25, 12),
            (26, 15),
            (37, 40),
            (38, 43),
            (41, 52),
            (42, 55),
        ] {
            assert_eq!(usize::from(TEXEL_TO_SHELL[texel]), corner, "texel {texel}");
        }
    }

    #[test]
    fn the_compress_dirs_are_the_tile_corners() {
        let corner_texels = [0, 3, 12, 15, 48, 51, 60, 63];
        for (dir, texel) in LIGHT_GRID_COMPRESS_DIRS.iter().zip(corner_texels) {
            assert_eq!(usize::from(TEXEL_TO_SHELL[texel]), *dir);
        }
    }

    #[test]
    fn expanding_fills_every_texel_with_its_shell_colour_and_the_weight() {
        let set = colour_set();
        let mut tile = [0u8; MODEL_LIGHTING_TILE_BYTES];
        assert!(light_grid_expand_shell_to_tile_rgba(&set, 0x7f, &mut tile));
        // Texel 23 is the 22nd outer texel (shell entry 21).
        assert_eq!(&tile[23 * 4..23 * 4 + 4], &[21, 121, 200, 0x7f]);
        // Texel 21 is inside, so it repeats corner 0.
        assert_eq!(&tile[21 * 4..21 * 4 + 4], &[0, 100, 200, 0x7f]);
        assert!(tile.chunks_exact(4).all(|texel| texel[3] == 0x7f));
    }

    #[test]
    fn expanding_refuses_short_buffers() {
        let set = colour_set();
        let mut tile = [0u8; MODEL_LIGHTING_TILE_BYTES];
        assert!(!light_grid_expand_shell_to_tile_rgba(
            &set[..167],
            0,
            &mut tile
        ));
        assert!(!light_grid_expand_shell_to_tile_rgba(
            &set,
            0,
            &mut tile[..255]
        ));
    }

    #[test]
    fn one_view_gets_a_4096_entry_atlas() {
        assert_eq!(
            model_lighting_atlas_dims(1),
            Some(ModelLightingAtlasDims {
                xmodel_entry_limit: 1024,
                smodel_entry_limit: 3072,
                image_height: 256,
            })
        );
    }

    #[test]
    fn two_views_already_leave_enough_for_static_models() {
        let dims = model_lighting_atlas_dims(2).unwrap();
        assert_eq!(dims.xmodel_entry_limit, 2048);
        assert_eq!(dims.smodel_entry_limit, 2048);
        assert_eq!(dims.image_height, 256);
    }

    #[test]
    fn three_views_double_the_atlas_to_keep_the_static_floor() {
        let dims = model_lighting_atlas_dims(3).unwrap();
        assert_eq!(dims.xmodel_entry_limit, 3072);
        assert_eq!(dims.smodel_entry_limit, 8192 - 3072);
        assert_eq!(dims.image_height, 512);
    }

    #[test]
    fn no_views_or_too_many_give_no_atlas() {
        assert_eq!(model_lighting_atlas_dims(0), None);
        assert_eq!(model_lighting_atlas_dims(1 << 21), None);
    }

    #[test]
    fn inverse_height_is_one_over_it() {
        assert_eq!(model_lighting_inv_image_height(256), Some(0.00390625));
        assert_eq!(model_lighting_inv_image_height(0), None);
    }

    #[test]
    fn handles_count_from_one() {
        assert_eq!(ModelLightingTileIndex::from_handle(0), None);
        assert_eq!(
            ModelLightingTileIndex::from_handle(1),
            Some(ModelLightingTileIndex { raw: 0 })
        );
    }

    #[test]
    fn tiles_run_across_a_row_of_64_then_down() {
        let origin = |handle| {
            ModelLightingTileIndex::from_handle(handle)
                .unwrap()
                .texel_origin()
        };
        assert_eq!(origin(1), (0, 0));
        assert_eq!(origin(2), (4, 0));
        assert_eq!(origin(64), (252, 0));
        assert_eq!(origin(65), (0, 4));
        assert_eq!(origin(66), (4, 4));
    }

    #[test]
    fn coordinates_aim_at_the_middle_of_the_tile() {
        let first = model_lighting_coords_from_handle(1, 1.0 / 256.0).unwrap();
        assert_eq!(first.as_array(), [2.0 / 256.0, 2.0 / 256.0, 0.5, 1.0]);
        let later = model_lighting_coords_from_handle(66, 1.0 / 256.0).unwrap();
        assert_eq!(later.as_array(), [6.0 / 256.0, 6.0 / 256.0, 0.5, 1.0]);
        assert_eq!(model_lighting_coords_from_handle(0, 1.0), None);
    }

    #[test]
    fn the_lookup_scale_is_a_texel_and_a_half_per_axis() {
        let scale = model_lighting_lookup_scale(1.0 / 512.0);
        assert_eq!(scale.as_array(), [0.005859375, 1.5 / 512.0, 0.375, 0.0]);
    }

    fn patterned_tile() -> [u8; MODEL_LIGHTING_TILE_BYTES] {
        let mut tile = [0u8; MODEL_LIGHTING_TILE_BYTES];
        for (index, byte) in tile.iter_mut().enumerate() {
            *byte = (index % 250) as u8 + 1;
        }
        tile
    }

    #[test]
    fn a_tile_lands_in_its_block_of_every_slice() {
        let height = 8u32;
        let slice = 1024 * height as usize;
        let mut atlas = vec![0u8; slice * 4];
        let tile = patterned_tile();
        // Handle 66 is the tile at texel (4, 4).
        let entry = ModelLightingTileIndex::from_handle(66).unwrap();
        assert!(model_lighting_write_tile_to_atlas(
            &mut atlas, height, entry, &tile
        ));
        for z in 0..4 {
            for row in 0..4 {
                let at = z * slice + (4 + row) * 1024 + 4 * 4;
                let from = (z * 4 + row) * 16;
                assert_eq!(
                    &atlas[at..at + 16],
                    &tile[from..from + 16],
                    "z {z} row {row}"
                );
            }
        }
        assert_eq!(
            atlas.iter().filter(|&&byte| byte != 0).count(),
            MODEL_LIGHTING_TILE_BYTES
        );
    }

    #[test]
    fn a_tile_below_the_atlas_or_a_small_buffer_is_refused() {
        let tile = patterned_tile();
        let mut atlas = vec![0u8; 4096 * 8];
        // Handle 129 starts at texel row 8: past an 8-tall atlas.
        let below = ModelLightingTileIndex::from_handle(129).unwrap();
        assert!(!model_lighting_write_tile_to_atlas(
            &mut atlas, 8, below, &tile
        ));
        let entry = ModelLightingTileIndex::from_handle(1).unwrap();
        assert!(!model_lighting_write_tile_to_atlas(
            &mut atlas[..4096 * 8 - 1],
            8,
            entry,
            &tile
        ));
        assert!(atlas.iter().all(|&byte| byte == 0));
    }
}
