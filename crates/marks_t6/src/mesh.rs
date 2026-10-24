//! Turning pooled marks into one frame's mark mesh: the frame's surface, vertex and index
//! budget, and each mark point as a BO2 vertex row.

use asset_t6::{T6_WORLD_VERTEX, T6PackedVertex, T6WorldVertex, f32_to_half, pack_unit_vec};

use crate::FxMarkStagingPoint;

/// Bytes per vertex row of the mark mesh. Every row has a world vertex's size; model
/// vertices leave the last four bytes zero.
pub const GFX_MARK_MESH_VERTEX_STRIDE: usize = T6_WORLD_VERTEX;

/// Draw surfaces per frame.
const SURF_LIMIT: u32 = 1536;
/// Vertices per frame.
const VERT_LIMIT: u32 = 6144;
/// Indices per frame.
const INDEX_LIMIT: u32 = 9216;

/// What one frame's mark mesh has used so far, and how many times each limit turned
/// something away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxMarkMeshBudget {
    pub surf_n: u32,
    pub vert_n: u32,
    pub index_n: u32,
    pub surf_warn: u32,
    pub vert_warn: u32,
    pub index_warn: u32,
}

/// Which limit turned a mark or surface away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GfxMarkMeshRefuse {
    SurfLimit,
    VertLimit,
    IndexLimit,
}

/// Rounded up to even: indices are reserved in pairs.
const fn even_up(n: u32) -> u32 {
    (n + 1) & !1
}

/// Reserves a mark's vertices and indices. Returns its first vertex and first index.
/// When the indices do not fit, its vertices stay reserved.
pub fn generate_mark_verts_begin(
    budget: &mut GfxMarkMeshBudget,
    point_count: i16,
    tri_count: u8,
) -> Result<(u16, u32), GfxMarkMeshRefuse> {
    let verts = point_count.max(0) as u32;
    if budget.vert_n + verts > VERT_LIMIT {
        budget.vert_warn += 1;
        return Err(GfxMarkMeshRefuse::VertLimit);
    }
    let first_vert = budget.vert_n as u16;
    budget.vert_n += verts;
    let indices = even_up(budget.index_n + 3 * u32::from(tri_count)) - even_up(budget.index_n);
    if budget.index_n + indices > INDEX_LIMIT {
        budget.index_warn += 1;
        return Err(GfxMarkMeshRefuse::IndexLimit);
    }
    let first_index = budget.index_n;
    budget.index_n += indices;
    Ok((first_vert, first_index))
}

/// Reserves one draw surface and returns its slot.
pub fn add_mark_mesh_draw_surf(budget: &mut GfxMarkMeshBudget) -> Result<u32, GfxMarkMeshRefuse> {
    if budget.surf_n >= SURF_LIMIT {
        budget.surf_warn += 1;
        return Err(GfxMarkMeshRefuse::SurfLimit);
    }
    budget.surf_n += 1;
    Ok(budget.surf_n - 1)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A mark point as a model vertex. The texture spans the mark's box: u runs along
/// `tex_coord_axis` and v along that axis crossed with the point's normal, 0.5 at the
/// mark's origin. `native_color` is the effect's colour as a little-endian B, G, R, A word.
fn mark_vertex(
    point: &FxMarkStagingPoint,
    origin: [f32; 3],
    radius: f32,
    tex_coord_axis: [f32; 3],
    native_color: u32,
) -> T6PackedVertex {
    let [b, g, r, a] = native_color.to_le_bytes();
    let binormal = cross(tex_coord_axis, point.normal);
    let scale = if radius == 0.0 { 0.0 } else { 0.5 / radius };
    let offset = [
        point.xyz[0] - origin[0],
        point.xyz[1] - origin[1],
        point.xyz[2] - origin[2],
    ];
    let u = dot(offset, tex_coord_axis) * scale + 0.5;
    let v = dot(offset, binormal) * scale + 0.5;
    T6PackedVertex {
        position: point.xyz,
        binormal_sign: -1.0,
        color: [r, g, b, a],
        tex_coord: u32::from(f32_to_half(u)) | u32::from(f32_to_half(v)) << 16,
        normal: pack_unit_vec(point.normal),
        tangent: pack_unit_vec(tex_coord_axis),
    }
}

/// A lightmap coordinate as BO2 stores it, a u16 over 65535.
///
/// TODO(fact): BO2's rounding here is open (QUESTIONS.md, 10-21 Clean1); nearest is used.
fn lightmap_unorm(c: f32) -> u32 {
    (c.clamp(0.0, 1.0) * 65535.0).round() as u32
}

/// A mark point on a lightmapped surface as a world vertex row.
pub fn pack_mark_world_vertex(
    point: &FxMarkStagingPoint,
    origin: [f32; 3],
    radius: f32,
    tex_coord_axis: [f32; 3],
    native_color: u32,
) -> [u8; GFX_MARK_MESH_VERTEX_STRIDE] {
    let [lu, lv] = point.lmap_coord.map(lightmap_unorm);
    T6WorldVertex {
        base: mark_vertex(point, origin, radius, tex_coord_axis, native_color),
        lmap_coord: lu | lv << 16,
    }
    .to_bytes()
}

/// A mark point on a model as a model vertex, in a row of the mesh's stride.
pub fn pack_mark_model_vertex(
    point: &FxMarkStagingPoint,
    origin: [f32; 3],
    radius: f32,
    tex_coord_axis: [f32; 3],
    native_color: u32,
) -> [u8; GFX_MARK_MESH_VERTEX_STRIDE] {
    let vertex = mark_vertex(point, origin, radius, tex_coord_axis, native_color).to_bytes();
    let mut row = [0; GFX_MARK_MESH_VERTEX_STRIDE];
    row[..vertex.len()].copy_from_slice(&vertex);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_reserved_in_pairs() {
        let mut budget = GfxMarkMeshBudget::default();
        assert_eq!(generate_mark_verts_begin(&mut budget, 5, 3), Ok((0, 0)));
        assert_eq!((budget.vert_n, budget.index_n), (5, 10));
        assert_eq!(generate_mark_verts_begin(&mut budget, 3, 1), Ok((5, 10)));
        assert_eq!((budget.vert_n, budget.index_n), (8, 14));
        assert_eq!(generate_mark_verts_begin(&mut budget, -2, 0), Ok((8, 14)));
        assert_eq!((budget.vert_n, budget.index_n), (8, 14));
    }

    #[test]
    fn full_budgets_count_what_they_turn_away() {
        let mut budget = GfxMarkMeshBudget {
            vert_n: 6140,
            ..Default::default()
        };
        assert_eq!(
            generate_mark_verts_begin(&mut budget, 5, 1),
            Err(GfxMarkMeshRefuse::VertLimit)
        );
        assert_eq!((budget.vert_n, budget.vert_warn), (6140, 1));
        // The vertices fit, the indices do not: the vertices stay taken.
        let mut budget = GfxMarkMeshBudget {
            index_n: 9210,
            ..Default::default()
        };
        assert_eq!(
            generate_mark_verts_begin(&mut budget, 4, 3),
            Err(GfxMarkMeshRefuse::IndexLimit)
        );
        assert_eq!(
            (budget.vert_n, budget.index_n, budget.index_warn),
            (4, 9210, 1)
        );
        assert_eq!(generate_mark_verts_begin(&mut budget, 4, 2), Ok((4, 9210)));
    }

    #[test]
    fn draw_surfaces_run_out_at_1536() {
        let mut budget = GfxMarkMeshBudget {
            surf_n: 1535,
            ..Default::default()
        };
        assert_eq!(add_mark_mesh_draw_surf(&mut budget), Ok(1535));
        assert_eq!(
            add_mark_mesh_draw_surf(&mut budget),
            Err(GfxMarkMeshRefuse::SurfLimit)
        );
        assert_eq!((budget.surf_n, budget.surf_warn), (1536, 1));
    }

    const UP: [f32; 3] = [0.0, 0.0, 1.0];
    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const ORIGIN: [f32; 3] = [10.0, 20.0, 0.0];

    fn point(xyz: [f32; 3], lmap_coord: [f32; 2]) -> FxMarkStagingPoint {
        FxMarkStagingPoint {
            xyz,
            lmap_coord,
            normal: UP,
        }
    }

    #[test]
    fn the_texture_spans_the_mark() {
        let row = |xyz| pack_mark_world_vertex(&point(xyz, [0.0; 2]), ORIGIN, 8.0, EAST, 0);
        let uv = |xyz| T6WorldVertex::read(&row(xyz)).unwrap().base.uv();
        assert_eq!(uv(ORIGIN), [0.5, 0.5]);
        assert_eq!(uv([18.0, 20.0, 0.0]), [1.0, 0.5]);
        // v runs along east × up, which is south.
        assert_eq!(uv([10.0, 12.0, 0.0]), [0.5, 1.0]);
        assert_eq!(uv([6.0, 24.0, 0.0]), [0.25, 0.25]);
        let flat = pack_mark_world_vertex(&point([99.0, 0.0, 0.0], [0.0; 2]), ORIGIN, 0.0, EAST, 0);
        assert_eq!(T6WorldVertex::read(&flat).unwrap().base.uv(), [0.5, 0.5]);
    }

    #[test]
    fn a_world_vertex_carries_place_colour_normal_and_lightmap() {
        let row = pack_mark_world_vertex(
            &point([1.0, 2.0, 3.0], [0.25, 1.5]),
            ORIGIN,
            8.0,
            EAST,
            0x8040_20ff,
        );
        let vertex = T6WorldVertex::read(&row).unwrap();
        assert_eq!(vertex.base.position, [1.0, 2.0, 3.0]);
        assert_eq!(vertex.base.binormal_sign, -1.0);
        assert_eq!(vertex.base.color, [0x40, 0x20, 0xff, 0x80]);
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.01);
        assert!(near(vertex.base.unpacked_normal(), UP));
        assert!(near(vertex.base.unpacked_tangent(), EAST));
        assert_eq!(vertex.lmap_coord, 16384 | 65535 << 16);
    }

    #[test]
    fn a_model_vertex_is_the_same_row_without_a_lightmap() {
        let p = point([1.0, 2.0, 3.0], [0.5, 0.5]);
        let world = pack_mark_world_vertex(&p, ORIGIN, 8.0, EAST, 0x1234_5678);
        let model = pack_mark_model_vertex(&p, ORIGIN, 8.0, EAST, 0x1234_5678);
        assert_eq!(world[..32], model[..32]);
        assert_eq!(model[32..], [0; 4]);
        assert_eq!(
            T6PackedVertex::read(&model),
            T6WorldVertex::read(&world).map(|w| w.base)
        );
    }
}
