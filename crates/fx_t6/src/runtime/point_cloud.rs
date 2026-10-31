//! Particle clouds and spark clouds: the grid of cells they are drawn from,
//! the vertices and indices of one cell, and the shader constants a cloud
//! draw sets.
//!
//! A cloud is drawn as a template of cells, each a camera-facing quad (a
//! spark is a longer strip of eight vertices). The template is a grid, each
//! cell jittered inside its square by a fixed random sequence, and sorted
//! from the centre outward so a cloud can draw just its first cells.

use crate::runtime::quat::axis_to_quat;
use crate::runtime::angle_graph::FX_RECIP_255;
use crate::runtime::vec::vec3_normalize;

/// The size of one position-and-texture vertex in bytes.
pub const POINT_UV_CORNER_BYTES: usize = 0x14;

/// How many cells the template has along each axis.
pub const SWARM_LATTICE_ACROSS: u32 = 8;
pub const SWARM_LATTICE_DEEP: u32 = 8;
pub const SWARM_LATTICE_TALL: u32 = 16;

/// How many cells the template has in all.
pub const SWARM_STAMP_SPECKS: usize = 1024;

/// Vertices and indices in one cloud cell.
pub const SWARM_CORNERS_EACH_SPECK: u32 = 4;
pub const SWARM_TRIANGLE_SLOTS_EACH_SPECK: u32 = 6;
const CLOUD_TRIS_PER_CELL: u32 = 2;

/// Vertices and indices in one spark cell.
pub const EMBER_CORNERS_EACH_SPECK: u32 = 8;
pub const EMBER_TRIANGLE_SLOTS_EACH_SPECK: u32 = 18;
const SPARK_TRIS_PER_CELL: u32 = 6;

/// The cloud flag that makes a cloud a spark cloud.
pub const SWARM_IS_EMBERS: u32 = 4;

/// The shader constants a cloud draw sets.
pub const SWARM_SHADER_BASIS_ROW: u8 = 0x41;
pub const EMBER_SHADER_TINT: u8 = 0x44;
pub const SWARM_SHADER_TINT: u8 = 0x06;
pub const SPRAY_SHADER_KNOB_A: u8 = 0x47;
pub const SPRAY_SHADER_KNOB_B: u8 = 0x48;

/// The indices of one cloud cell's two triangles.
pub const SWARM_RECT_TRIANGLE_SLOTS: [u16; 6] = [0_u16, 1, 2, 2, 1, 3];

/// The texture coordinates of one cloud cell's corners.
pub const SWARM_CORNER_UVS: [[f32; 2]; 4] = [[0.0_f32, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];

/// The indices of one spark cell's six triangles.
const SPARK_INDICES: [u16; 18] = [0, 2, 1, 1, 2, 3, 2, 4, 3, 3, 4, 5, 4, 6, 5, 5, 6, 7];

/// Where the spark strip's two inner rows sit up the texture: close to a
/// third and two thirds, but as the game stores them.
const SPARK_V_THIRD: f32 = f32::from_bits(0x3eaa_a64c);
const SPARK_V_TWO_THIRDS: f32 = f32::from_bits(0x3f2a_acda);

/// The texture coordinates of one spark cell's eight vertices.
const SPARK_UV: [[f32; 2]; 8] = [
    [0.0, 0.0],
    [1.0, 0.0],
    [0.0, SPARK_V_THIRD],
    [1.0, SPARK_V_THIRD],
    [0.0, SPARK_V_TWO_THIRDS],
    [1.0, SPARK_V_TWO_THIRDS],
    [0.0, 1.0],
    [1.0, 1.0],
];

/// The seed of the C runtime's random sequence the template is built from.
pub const CLASSIC_DICE_START: u32 = 1;

/// A template cell's size: a quarter unit across, an eighth tall, with the
/// whole template centred on the origin.
const CELL_SCALE_XY: f32 = 0.25;
const CELL_SCALE_Z: f32 = 0.125;
const CELL_ORIGIN: f32 = 1.0;

/// A fountain's second parameter is the age, a thousandth per millisecond;
/// the first is the same, half a second earlier.
const FOUNTAIN_AGE_BIAS_MS: f32 = 500.0;
const FOUNTAIN_AGE_SCALE: f32 = 0.001;

/// One vertex of a cloud cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointUvCorner {
    pub spot: [f32; 3],
    pub sheet_uv: [f32; 2],
}

/// One cloud as the renderer gets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualParticleCloud {
    pub turn: [f32; 4],
    pub spot: [f32; 3],
    pub layout_size_ratio: f32,
    /// Where the cloud was just before, taken from its velocity.
    pub basis_or_speed: [f32; 3],
    /// Blue, green, red, alpha from the low byte up.
    pub color: u32,
    pub start_extent: f32,
    pub second_extent: f32,
    /// The template size in the low two bits.
    pub flags: u32,
    pub scale: f32,
}

/// How many template cells a cloud draws: all of them, or half, a quarter
/// or an eighth, by the cloud's two lowest flag bits.
pub const fn particle_cloud_draw_cell_count(cloud_flags: u32) -> usize {
    SWARM_STAMP_SPECKS >> (cloud_flags & 3)
}

/// How many vertices and triangles a cloud draws.
pub const fn particle_cloud_draw_counts(spark: bool, cloud_bits: u32) -> (u32, u32) {
    let cells = particle_cloud_draw_cell_count(cloud_bits) as u32;
    let (verts_each, tris_each) = if spark {
        (EMBER_CORNERS_EACH_SPECK, SPARK_TRIS_PER_CELL)
    } else {
        (SWARM_CORNERS_EACH_SPECK, CLOUD_TRIS_PER_CELL)
    };
    (cells.wrapping_mul(verts_each), cells.wrapping_mul(tris_each))
}

/// The id a spark particle gets from its cell in the grid.
pub const fn particle_cloud_particle_id(x: u32, y: u32, depth: u32) -> u32 {
    // Three fields of one running sum: z as is, y in sixteens, x in 128s.
    let mut id = depth;
    id = id.wrapping_add(y.wrapping_mul(16));
    id.wrapping_add(x.wrapping_shl(7))
}

/// The next number from the C runtime's random sequence, 0 to 32767.
fn msvcrt_rand(holdrand: &mut u32) -> u32 {
    // The classic LCG: state * 214013 + 2531011, top 15 bits of the low word out.
    *holdrand = holdrand.wrapping_mul(214_013).wrapping_add(2_531_011);
    (*holdrand >> 16) & 0x7fff
}

/// The next number from the C runtime's random sequence as 0 to 1.
pub fn classic_dice_unit(holdrand: &mut u32) -> f32 {
    msvcrt_rand(holdrand) as f32 / 32767.0
}

/// Where a template cell sits: its grid square plus a jitter, in a box
/// centred on the origin.
pub fn particle_cloud_cell_xyz(x: u32, y: u32, z: u32, roll: [f32; 3]) -> [f32; 3] {
    [
        (roll[0] + x as f32) * CELL_SCALE_XY - CELL_ORIGIN,
        (roll[1] + y as f32) * CELL_SCALE_XY - CELL_ORIGIN,
        (roll[2] + z as f32) * CELL_SCALE_Z - CELL_ORIGIN,
    ]
}

/// A cell's squared distance from the template's centre.
pub fn particle_cloud_cell_radius_sq(xyz: [f32; 3]) -> f32 {
    xyz.iter().map(|c| c * c).sum()
}

/// Orders two cells nearest the centre first: -1, 0 or 1.
pub fn particle_cloud_compare_cell_radius(a: [f32; 3], b: [f32; 3]) -> i32 {
    use core::cmp::Ordering;
    let radii = (
        particle_cloud_cell_radius_sq(a),
        particle_cloud_cell_radius_sq(b),
    );
    match radii.0.partial_cmp(&radii.1) {
        Some(Ordering::Less) => -1,
        Some(Ordering::Greater) => 1,
        _ => 0,
    }
}

/// The four vertices of a cloud cell: all at the cell's centre, told apart
/// by their texture coordinates; the shader spreads them into a quad.
pub fn particle_cloud_cell_verts(xyz: [f32; 3]) -> [PointUvCorner; 4] {
    verts_at(xyz, SWARM_CORNER_UVS)
}

/// One vertex per texture coordinate, all at `xyz`.
fn verts_at<const N: usize>(xyz: [f32; 3], texcoord_set: [[f32; 2]; N]) -> [PointUvCorner; N] {
    texcoord_set.map(|tex_coord| PointUvCorner { spot: xyz, sheet_uv: tex_coord })
}

/// The indices of cloud cell number `cell`.
pub fn particle_cloud_cell_indices(cell: u32) -> [u16; 6] {
    let base = cell.wrapping_mul(SWARM_CORNERS_EACH_SPECK) as u16;
    SWARM_RECT_TRIANGLE_SLOTS.map(|index| index.wrapping_add(base))
}

/// The eight vertices of a spark cell.
pub fn particle_spark_cell_verts(xyz: [f32; 3]) -> [PointUvCorner; 8] {
    verts_at(xyz, SPARK_UV)
}

/// The indices of the spark cell of particle `particle_id`.
pub fn particle_spark_cell_indices(particle_id: u32) -> [u16; 18] {
    let base = particle_id.wrapping_mul(EMBER_CORNERS_EACH_SPECK) as u16;
    SPARK_INDICES.map(|index| index.wrapping_add(base))
}

/// A vertex as the 20 bytes the vertex buffer holds.
pub fn point_uv_corner_bytes(vert: PointUvCorner) -> [u8; POINT_UV_CORNER_BYTES] {
    let [x, y, z] = vert.spot;
    let [u, v] = vert.sheet_uv;
    let words = [x, y, z, u, v].map(f32::to_le_bytes);
    core::array::from_fn(|byte| words[byte / 4][byte % 4])
}

/// A packed cloud colour as the red, green, blue, alpha shader constant.
pub fn particle_cloud_color_const(color: u32) -> [f32; 4] {
    let [b, g, r, a] = color.to_le_bytes();
    let step = FX_RECIP_255 as f32;
    [
        f32::from(r) * step,
        f32::from(g) * step,
        f32::from(b) * step,
        f32::from(a) * step,
    ]
}

/// The shader's size matrix: the two sizes on the diagonal.
pub fn particle_cloud_matrix_diag(start_extent: f32, second_extent: f32) -> [f32; 4] {
    // A 2x2 matrix laid out row by row: entries 0 and 3 are the diagonal.
    core::array::from_fn(|slot| match slot {
        0 => start_extent,
        3 => second_extent,
        _ => 0.0,
    })
}

/// A fountain's first shader parameter from its age in milliseconds.
pub fn particle_fountain_parm0(lived_ms: f32) -> [f32; 4] {
    [
        (lived_ms - FOUNTAIN_AGE_BIAS_MS) * FOUNTAIN_AGE_SCALE,
        lived_ms * FOUNTAIN_AGE_SCALE,
        0.0,
        0.0,
    ]
}

/// A colour packed blue, green, red, alpha from the low byte up.
const fn pack_gfx_color(rgba: [u8; 4]) -> u32 {
    u32::from_le_bytes([rgba[2], rgba[1], rgba[0], rgba[3]])
}

/// The renderer's cloud for an element. Its velocity gives the direction
/// the cloud came from; the element's flags give the template size.
#[allow(clippy::too_many_arguments)]
pub fn build_cloud(
    scene_anchor: [f32; 3],
    elem_turn: [[f32; 3]; 3],
    start_extent: f32,
    second_extent: f32,
    layout_size_ratio: f32,
    tint: [u8; 4],
    elem_bits: i32,
    speed_vec: [f32; 3],
    age_offset: f32,
) -> VisualParticleCloud {
    let heading = vec3_normalize(speed_vec);
    // One unit back along the velocity from where the cloud is now.
    let came_from = core::array::from_fn(|i| scene_anchor[i] - heading[i]);
    // The element's top two flag bits become the template-size bits.
    let size_bits = (elem_bits as u32) >> 29 & 3;
    VisualParticleCloud {
        turn: axis_to_quat(elem_turn),
        start_extent,
        second_extent,
        layout_size_ratio,
        spot: scene_anchor,
        basis_or_speed: came_from,
        color: pack_gfx_color(tint),
        flags: size_bits,
        scale: age_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_low_flag_bits_halve_the_cells_drawn() {
        assert_eq!(particle_cloud_draw_cell_count(0), 1024);
        assert_eq!(particle_cloud_draw_cell_count(1), 512);
        assert_eq!(particle_cloud_draw_cell_count(2), 256);
        assert_eq!(particle_cloud_draw_cell_count(3), 128);
        assert_eq!(
            particle_cloud_draw_cell_count(SWARM_IS_EMBERS),
            1024
        );
    }

    #[test]
    fn draw_counts_scale_with_the_cells_and_the_cloud_kind() {
        assert_eq!(particle_cloud_draw_counts(false, 0), (4096, 2048));
        assert_eq!(particle_cloud_draw_counts(true, 0), (8192, 6144));
        assert_eq!(particle_cloud_draw_counts(false, 3), (512, 256));
    }

    #[test]
    fn a_particle_id_packs_the_grid_position() {
        assert_eq!(particle_cloud_particle_id(0, 0, 0), 0);
        assert_eq!(particle_cloud_particle_id(1, 0, 0), 128);
        assert_eq!(particle_cloud_particle_id(0, 1, 0), 16);
        assert_eq!(particle_cloud_particle_id(2, 3, 5), 5 + 256 + 48);
    }

    #[test]
    fn the_c_runtime_sequence_starts_the_way_it_always_does() {
        let mut hold = CLASSIC_DICE_START;
        assert_eq!(msvcrt_rand(&mut hold), 41);
        assert_eq!(msvcrt_rand(&mut hold), 18467);
        assert_eq!(msvcrt_rand(&mut hold), 6334);
    }

    #[test]
    fn the_unit_random_is_the_sequence_over_32767() {
        let mut hold = CLASSIC_DICE_START;
        assert_eq!(classic_dice_unit(&mut hold), 41.0 / 32767.0);
    }

    #[test]
    fn a_cell_sits_in_its_grid_square_inside_the_centred_box() {
        assert_eq!(
            particle_cloud_cell_xyz(2, 3, 4, [0.5, 0.5, 0.5]),
            [-0.375, -0.125, -0.4375]
        );
        let low = particle_cloud_cell_xyz(0, 0, 0, [0.0; 3]);
        assert_eq!(low, [-1.0, -1.0, -1.0]);
    }

    #[test]
    fn cells_order_nearest_the_centre_first() {
        let near = [0.1, 0.0, 0.0];
        let far = [0.0, 0.0, 0.5];
        assert_eq!(particle_cloud_cell_radius_sq(far), 0.25);
        assert_eq!(particle_cloud_compare_cell_radius(near, far), -1);
        assert_eq!(particle_cloud_compare_cell_radius(far, near), 1);
        assert_eq!(particle_cloud_compare_cell_radius(near, [0.0, 0.1, 0.0]), 0);
    }

    #[test]
    fn a_cloud_cell_is_four_vertices_at_one_point() {
        let vertex_points = particle_cloud_cell_verts([1.0, 2.0, 3.0]);
        assert!(vertex_points.iter().all(|v| v.spot == [1.0, 2.0, 3.0]));
        assert_eq!(vertex_points[3].sheet_uv, [1.0, 1.0]);
        assert_eq!(vertex_points[1].sheet_uv, [1.0, 0.0]);
    }

    #[test]
    fn cell_indices_step_four_vertices_per_cell() {
        assert_eq!(particle_cloud_cell_indices(0), [0, 1, 2, 2, 1, 3]);
        assert_eq!(particle_cloud_cell_indices(3), [12, 13, 14, 14, 13, 15]);
    }

    #[test]
    fn a_spark_cell_is_eight_vertices_up_a_strip() {
        let vertex_points = particle_spark_cell_verts([0.0; 3]);
        assert_eq!(vertex_points.len(), 8);
        assert_eq!(vertex_points[0].sheet_uv, [0.0, 0.0]);
        assert_eq!(vertex_points[7].sheet_uv, [1.0, 1.0]);
        let thirds = [vertex_points[2].sheet_uv[1], vertex_points[4].sheet_uv[1]];
        assert!((thirds[0] - 1.0 / 3.0).abs() < 1e-3);
        assert!((thirds[1] - 2.0 / 3.0).abs() < 1e-3);
    }

    #[test]
    fn spark_indices_step_eight_vertices_per_particle() {
        let got = particle_spark_cell_indices(2);
        assert_eq!(got[..6], [16, 18, 17, 17, 18, 19]);
        assert_eq!(got[17], 23);
    }

    #[test]
    fn indices_wrap_at_sixteen_bits() {
        assert_eq!(particle_cloud_cell_indices(0x4000)[0], 0);
    }

    #[test]
    fn a_vertex_is_twenty_little_endian_bytes() {
        let bytes = point_uv_corner_bytes(PointUvCorner {
            spot: [1.0, 2.0, 3.0],
            sheet_uv: [0.5, 0.25],
        });
        assert_eq!(bytes[0..4], 1.0f32.to_le_bytes());
        assert_eq!(bytes[8..12], 3.0f32.to_le_bytes());
        assert_eq!(bytes[12..16], 0.5f32.to_le_bytes());
        assert_eq!(bytes[16..20], 0.25f32.to_le_bytes());
    }

    #[test]
    fn a_packed_colour_comes_back_out_red_first() {
        let packed = pack_gfx_color([255, 0, 51, 128]);
        assert_eq!(packed, 0x80ff_0033);
        let got = particle_cloud_color_const(packed);
        assert!((got[0] - 1.0).abs() < 1e-6);
        assert_eq!(got[1], 0.0);
        assert!((got[2] - 0.2).abs() < 1e-6);
        assert!((got[3] - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn the_size_matrix_has_the_sizes_on_the_diagonal() {
        assert_eq!(particle_cloud_matrix_diag(3.0, 4.0), [3.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn a_fountain_s_parameters_follow_its_age() {
        let parm = particle_fountain_parm0(1500.0);
        assert!((parm[0] - 1.0).abs() < 1e-6);
        assert!((parm[1] - 1.5).abs() < 1e-6);
        assert_eq!(parm[2..], [0.0, 0.0]);
        assert_eq!(particle_fountain_parm0(500.0)[0], 0.0);
    }

    #[test]
    fn a_built_cloud_carries_the_element_s_placement() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let cloud = build_cloud(
            [1.0, 2.0, 3.0],
            identity,
            5.0,
            6.0,
            0.5,
            [255, 0, 51, 128],
            (3u32 << 29) as i32,
            [0.0, 0.0, 2.0],
            7.0,
        );
        assert_eq!(cloud.spot, [1.0, 2.0, 3.0]);
        assert_eq!(cloud.basis_or_speed, [1.0, 2.0, 2.0]);
        assert_eq!(cloud.color, 0x80ff_0033);
        assert_eq!((cloud.start_extent, cloud.second_extent, cloud.scale), (5.0, 6.0, 7.0));
        assert_eq!(cloud.layout_size_ratio, 0.5);
        assert_eq!(cloud.flags, 3);
        assert!((cloud.turn[3].abs() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn only_the_top_bits_of_the_element_flags_pick_the_template_size() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        // Moving along the first axis; nothing else matters for the flags.
        let flags_of = |flag_bits| {
            let cloud = build_cloud([0.0; 3], identity, 0.0, 0.0, 1.0, [0; 4], flag_bits, identity[0], 0.0);
            cloud.flags
        };
        assert_eq!(flags_of(0x1fff_ffff), 0);
        assert_eq!(flags_of(1 << 29), 1);
        assert_eq!(flags_of(2 << 29), 2);
    }
}
