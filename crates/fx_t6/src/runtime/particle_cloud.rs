//! Particle clouds and spark clouds: the grid of cells they are drawn from,
//! the vertices and indices of one cell, and the shader constants a cloud
//! draw sets.
//!
//! A cloud is drawn as a template of cells, each a camera-facing quad (a
//! spark is a longer strip of eight vertices). The template is a grid, each
//! cell jittered inside its square by a fixed random sequence, and sorted
//! from the centre outward so a cloud can draw just its first cells.

use crate::runtime::quat::axis_to_quat;
use crate::runtime::rotation::FX_RECIP_255;
use crate::runtime::vec::vec3_normalize;

/// The size of one position-and-texture vertex in bytes.
pub const GFX_POS_TEX_VERTEX_STRIDE: usize = 0x14;

/// How many cells the template has along each axis.
pub const FX_PARTICLE_CLOUD_GRID_X: u32 = 8;
pub const FX_PARTICLE_CLOUD_GRID_Y: u32 = 8;
pub const FX_PARTICLE_CLOUD_GRID_Z: u32 = 16;

/// How many cells the template has in all.
pub const FX_PARTICLE_CLOUD_TEMPLATE_CELLS: usize = 1024;

/// Vertices and indices in one cloud cell.
pub const FX_PARTICLE_CLOUD_VERTS_PER_CELL: u32 = 4;
pub const FX_PARTICLE_CLOUD_INDICES_PER_CELL: u32 = 6;
const CLOUD_TRIS_PER_CELL: u32 = 2;

/// Vertices and indices in one spark cell.
pub const FX_PARTICLE_SPARK_VERTS_PER_CELL: u32 = 8;
pub const FX_PARTICLE_SPARK_INDICES_PER_CELL: u32 = 18;
const SPARK_TRIS_PER_CELL: u32 = 6;

/// The cloud flag that makes a cloud a spark cloud.
pub const FX_PARTICLE_CLOUD_FLAG_SPARK: u32 = 4;

/// The shader constants a cloud draw sets.
pub const FX_CODE_PARTICLE_CLOUD_MATRIX0: u8 = 0x41;
pub const FX_CODE_SPARK_COLOR0: u8 = 0x44;
pub const FX_CODE_PARTICLE_CLOUD_COLOR: u8 = 0x06;
pub const FX_CODE_FOUNTAIN_PARM0: u8 = 0x47;
pub const FX_CODE_FOUNTAIN_PARM1: u8 = 0x48;

/// The indices of one cloud cell's two triangles.
pub const FX_PARTICLE_CLOUD_QUAD_INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// The texture coordinates of one cloud cell's corners.
pub const FX_PARTICLE_CLOUD_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];

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
pub const MSVCRT_HOLDRAND_DEFAULT: u32 = 1;

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
pub struct GfxPosTexVertex {
    pub xyz: [f32; 3],
    pub tex_coord: [f32; 2],
}

/// One cloud as the renderer gets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxParticleCloud {
    pub quat: [f32; 4],
    pub pos: [f32; 3],
    pub placement_scale: f32,
    /// Where the cloud was just before, taken from its velocity.
    pub axis_or_vel: [f32; 3],
    /// Blue, green, red, alpha from the low byte up.
    pub color: u32,
    pub size0: f32,
    pub size1: f32,
    /// The template size in the low two bits.
    pub flags: u32,
    pub scale: f32,
}

/// How many template cells a cloud draws: all of them, or half, a quarter
/// or an eighth, by the cloud's two lowest flag bits.
pub const fn particle_cloud_draw_cell_count(cloud_flags: u32) -> usize {
    FX_PARTICLE_CLOUD_TEMPLATE_CELLS >> (cloud_flags & 3)
}

/// How many vertices and triangles a cloud draws.
pub const fn particle_cloud_draw_counts(spark: bool, cloud_flags: u32) -> (u32, u32) {
    let cells = particle_cloud_draw_cell_count(cloud_flags) as u32;
    if spark {
        (
            cells.wrapping_mul(FX_PARTICLE_SPARK_VERTS_PER_CELL),
            cells.wrapping_mul(SPARK_TRIS_PER_CELL),
        )
    } else {
        (
            cells.wrapping_mul(FX_PARTICLE_CLOUD_VERTS_PER_CELL),
            cells.wrapping_mul(CLOUD_TRIS_PER_CELL),
        )
    }
}

/// The id a spark particle gets from its cell in the grid.
pub const fn particle_cloud_particle_id(x: u32, y: u32, z: u32) -> u32 {
    z.wrapping_add(x.wrapping_shl(7))
        .wrapping_add(y.wrapping_mul(16))
}

/// The next number from the C runtime's random sequence, 0 to 32767.
fn msvcrt_rand(holdrand: &mut u32) -> u32 {
    *holdrand = holdrand.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    (*holdrand >> 16) & 0x7fff
}

/// The next number from the C runtime's random sequence as 0 to 1.
pub fn msvcrt_rand01(holdrand: &mut u32) -> f32 {
    msvcrt_rand(holdrand) as f32 / 32767.0
}

/// Where a template cell sits: its grid square plus a jitter, in a box
/// centred on the origin.
pub fn particle_cloud_cell_xyz(x: u32, y: u32, z: u32, rand01: [f32; 3]) -> [f32; 3] {
    [
        (rand01[0] + x as f32) * CELL_SCALE_XY - CELL_ORIGIN,
        (rand01[1] + y as f32) * CELL_SCALE_XY - CELL_ORIGIN,
        (rand01[2] + z as f32) * CELL_SCALE_Z - CELL_ORIGIN,
    ]
}

/// A cell's squared distance from the template's centre.
pub fn particle_cloud_cell_radius_sq(xyz: [f32; 3]) -> f32 {
    xyz[0] * xyz[0] + xyz[1] * xyz[1] + xyz[2] * xyz[2]
}

/// Orders two cells nearest the centre first: -1, 0 or 1.
pub fn particle_cloud_compare_cell_radius(a: [f32; 3], b: [f32; 3]) -> i32 {
    let ra = particle_cloud_cell_radius_sq(a);
    let rb = particle_cloud_cell_radius_sq(b);
    if ra < rb {
        -1
    } else if ra > rb {
        1
    } else {
        0
    }
}

/// The four vertices of a cloud cell: all at the cell's centre, told apart
/// by their texture coordinates; the shader spreads them into a quad.
pub fn particle_cloud_cell_verts(xyz: [f32; 3]) -> [GfxPosTexVertex; 4] {
    core::array::from_fn(|i| GfxPosTexVertex {
        xyz,
        tex_coord: FX_PARTICLE_CLOUD_UV[i],
    })
}

/// The indices of cloud cell number `cell`.
pub fn particle_cloud_cell_indices(cell: u32) -> [u16; 6] {
    let base = cell.wrapping_mul(FX_PARTICLE_CLOUD_VERTS_PER_CELL) as u16;
    FX_PARTICLE_CLOUD_QUAD_INDICES.map(|index| index.wrapping_add(base))
}

/// The eight vertices of a spark cell.
pub fn particle_spark_cell_verts(xyz: [f32; 3]) -> [GfxPosTexVertex; 8] {
    core::array::from_fn(|i| GfxPosTexVertex {
        xyz,
        tex_coord: SPARK_UV[i],
    })
}

/// The indices of the spark cell of particle `particle_id`.
pub fn particle_spark_cell_indices(particle_id: u32) -> [u16; 18] {
    let base = particle_id.wrapping_mul(FX_PARTICLE_SPARK_VERTS_PER_CELL) as u16;
    SPARK_INDICES.map(|index| index.wrapping_add(base))
}

/// A vertex as the 20 bytes the vertex buffer holds.
pub fn gfx_pos_tex_vertex_bytes(vert: GfxPosTexVertex) -> [u8; GFX_POS_TEX_VERTEX_STRIDE] {
    let mut out = [0u8; GFX_POS_TEX_VERTEX_STRIDE];
    let floats = vert.xyz.into_iter().chain(vert.tex_coord);
    for (slot, f) in out.chunks_exact_mut(4).zip(floats) {
        slot.copy_from_slice(&f.to_le_bytes());
    }
    out
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
pub fn particle_cloud_matrix_diag(size0: f32, size1: f32) -> [f32; 4] {
    [size0, 0.0, 0.0, size1]
}

/// A fountain's first shader parameter from its age in milliseconds.
pub fn particle_fountain_parm0(age_msec: f32) -> [f32; 4] {
    [
        (age_msec - FOUNTAIN_AGE_BIAS_MS) * FOUNTAIN_AGE_SCALE,
        age_msec * FOUNTAIN_AGE_SCALE,
        0.0,
        0.0,
    ]
}

/// A colour packed blue, green, red, alpha from the low byte up.
const fn pack_gfx_color(rgba: [u8; 4]) -> u32 {
    let [r, g, b, a] = rgba;
    u32::from_le_bytes([b, g, r, a])
}

/// The renderer's cloud for an element. Its velocity gives the direction
/// the cloud came from; the element's flags give the template size.
#[allow(clippy::too_many_arguments)]
pub fn build_cloud(
    world_origin: [f32; 3],
    elem_axis: [[f32; 3]; 3],
    size0: f32,
    size1: f32,
    placement_scale: f32,
    color_rgba: [u8; 4],
    elem_flags: i32,
    velocity: [f32; 3],
    time_offset: f32,
) -> GfxParticleCloud {
    let heading = vec3_normalize(velocity);
    GfxParticleCloud {
        quat: axis_to_quat(elem_axis),
        pos: world_origin,
        placement_scale,
        axis_or_vel: [
            world_origin[0] - heading[0],
            world_origin[1] - heading[1],
            world_origin[2] - heading[2],
        ],
        color: pack_gfx_color(color_rgba),
        size0,
        size1,
        flags: ((elem_flags as u32) >> 29) & 3,
        scale: time_offset,
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
            particle_cloud_draw_cell_count(FX_PARTICLE_CLOUD_FLAG_SPARK),
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
        let mut hold = MSVCRT_HOLDRAND_DEFAULT;
        assert_eq!(msvcrt_rand(&mut hold), 41);
        assert_eq!(msvcrt_rand(&mut hold), 18467);
        assert_eq!(msvcrt_rand(&mut hold), 6334);
    }

    #[test]
    fn the_unit_random_is_the_sequence_over_32767() {
        let mut hold = MSVCRT_HOLDRAND_DEFAULT;
        assert_eq!(msvcrt_rand01(&mut hold), 41.0 / 32767.0);
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
        let verts = particle_cloud_cell_verts([1.0, 2.0, 3.0]);
        assert!(verts.iter().all(|v| v.xyz == [1.0, 2.0, 3.0]));
        assert_eq!(verts[3].tex_coord, [1.0, 1.0]);
        assert_eq!(verts[1].tex_coord, [1.0, 0.0]);
    }

    #[test]
    fn cell_indices_step_four_vertices_per_cell() {
        assert_eq!(particle_cloud_cell_indices(0), [0, 1, 2, 2, 1, 3]);
        assert_eq!(particle_cloud_cell_indices(3), [12, 13, 14, 14, 13, 15]);
    }

    #[test]
    fn a_spark_cell_is_eight_vertices_up_a_strip() {
        let verts = particle_spark_cell_verts([0.0; 3]);
        assert_eq!(verts.len(), 8);
        assert_eq!(verts[0].tex_coord, [0.0, 0.0]);
        assert_eq!(verts[7].tex_coord, [1.0, 1.0]);
        let thirds = [verts[2].tex_coord[1], verts[4].tex_coord[1]];
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
        let bytes = gfx_pos_tex_vertex_bytes(GfxPosTexVertex {
            xyz: [1.0, 2.0, 3.0],
            tex_coord: [0.5, 0.25],
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
        assert_eq!(cloud.pos, [1.0, 2.0, 3.0]);
        assert_eq!(cloud.axis_or_vel, [1.0, 2.0, 2.0]);
        assert_eq!(cloud.color, 0x80ff_0033);
        assert_eq!((cloud.size0, cloud.size1, cloud.scale), (5.0, 6.0, 7.0));
        assert_eq!(cloud.placement_scale, 0.5);
        assert_eq!(cloud.flags, 3);
        assert!((cloud.quat[3].abs() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn only_the_top_bits_of_the_element_flags_pick_the_template_size() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let flags_of = |elem_flags| {
            build_cloud(
                [0.0; 3],
                identity,
                0.0,
                0.0,
                1.0,
                [0; 4],
                elem_flags,
                [1.0, 0.0, 0.0],
                0.0,
            )
            .flags
        };
        assert_eq!(flags_of(0x1fff_ffff), 0);
        assert_eq!(flags_of(1 << 29), 1);
        assert_eq!(flags_of(2 << 29), 2);
    }
}
