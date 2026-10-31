//! Turning pooled marks into one frame's mark mesh: the frame's surface, vertex and index
//! budget, and each mark point as a BO2 vertex row.

use asset_t6::{T6_WORLD_VERTEX, T6PackedVertex, T6WorldVertex, f32_to_half, pack_unit_vec};

use crate::EffectStainStagingPoint;

/// Bytes per vertex row of the mark mesh. Every row has a world vertex's size; model
/// vertices leave the last four bytes zero.
pub const DECAL_MESH_VERTEX_BYTES: usize = T6_WORLD_VERTEX;

/// Draw surfaces per frame.
const SURF_LIMIT: u32 = 1536;
/// Vertices per frame.
const VERT_LIMIT: u32 = 6144;
/// Indices per frame.
const INDEX_LIMIT: u32 = 9216;

/// What one frame's mark mesh has used so far, and how many times each limit turned
/// something away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecalMeshAllowance {
    pub facet_sum: u32,
    pub vertex_amount: u32,
    pub index_tally: u32,
    pub surf_soft_cap: u32,
    pub vert_soft_cap: u32,
    pub too_many_indices_logged: u32,
}

/// Which limit turned a mark or surface away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalMeshDenial {
    SurfLimit,
    VertLimit,
    IndicesExhausted,
}

/// Rounded up to even: indices are reserved in pairs.
const fn even_up(n: u32) -> u32 {
    (n + 1) & !1
}

/// Reserves a mark's vertices and indices. Returns its first vertex and first index.
/// When the indices do not fit, its vertices stay reserved.
pub fn generate_mark_verts_begin(
    budget: &mut DecalMeshAllowance,
    vertex_total_in_mark: i16,
    tri_tally: u8,
) -> Result<(u16, u32), DecalMeshDenial> {
    let vertex_points = vertex_total_in_mark.max(0) as u32;
    if budget.vertex_amount + vertex_points > VERT_LIMIT {
        budget.vert_soft_cap += 1;
        return Err(DecalMeshDenial::VertLimit);
    }
    let first_vert = budget.vertex_amount as u16;
    budget.vertex_amount += vertex_points;
    let indices = even_up(budget.index_tally + 3 * u32::from(tri_tally)) - even_up(budget.index_tally);
    if budget.index_tally + indices > INDEX_LIMIT {
        budget.too_many_indices_logged += 1;
        return Err(DecalMeshDenial::IndicesExhausted);
    }
    let first_index = budget.index_tally;
    budget.index_tally += indices;
    Ok((first_vert, first_index))
}

/// Reserves one draw surface and returns its slot.
pub fn add_mark_mesh_draw_surf(budget: &mut DecalMeshAllowance) -> Result<u32, DecalMeshDenial> {
    if budget.facet_sum >= SURF_LIMIT {
        budget.surf_soft_cap += 1;
        return Err(DecalMeshDenial::SurfLimit);
    }
    budget.facet_sum += 1;
    Ok(budget.facet_sum - 1)
}

fn cross(ua: [f32; 3], rv: [f32; 3]) -> [f32; 3] {
    [
        ua[1] * rv[2] - ua[2] * rv[1],
        ua[2] * rv[0] - ua[0] * rv[2],
        ua[0] * rv[1] - ua[1] * rv[0],
    ]
}

fn dot3(pa: [f32; 3], q: [f32; 3]) -> f32 {
    pa[0] * q[0] + pa[1] * q[1] + pa[2] * q[2]
}

/// A mark point as a model vertex. The texture spans the mark's box: u runs along
/// `uv_projection_axis` and v along that axis crossed with the point's normal, 0.5 at the
/// mark's origin. `authored_tint` is the effect's colour as a little-endian B, G, R, A word.
fn mark_vertex(
    corner: &EffectStainStagingPoint,
    centre: [f32; 3],
    reach: f32,
    uv_projection_axis: [f32; 3],
    authored_tint: u32,
) -> T6PackedVertex {
    let [b, g, r, a] = authored_tint.to_le_bytes();
    let bitan = cross(uv_projection_axis, corner.normal);
    let inv_span = if reach == 0.0 { 0.0 } else { 0.5 / reach };
    let rel = [
        corner.spot[0] - centre[0],
        corner.spot[1] - centre[1],
        corner.spot[2] - centre[2],
    ];
    let u = dot3(rel, uv_projection_axis) * inv_span + 0.5;
    let v = dot3(rel, bitan) * inv_span + 0.5;
    T6PackedVertex {
        position: corner.spot,
        binormal_sign: -1.0,
        color: [r, g, b, a],
        tex_coord: u32::from(f32_to_half(u)) | u32::from(f32_to_half(v)) << 16,
        normal: pack_unit_vec(corner.normal),
        tangent: pack_unit_vec(uv_projection_axis),
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
    point: &EffectStainStagingPoint,
    at_pt: [f32; 3],
    span_r: f32,
    uv_projection_axis: [f32; 3],
    authored_tint: u32,
) -> [u8; DECAL_MESH_VERTEX_BYTES] {
    let [lu, lv] = point.baked_light_uv.map(lightmap_unorm);
    T6WorldVertex {
        base: mark_vertex(point, at_pt, span_r, uv_projection_axis, authored_tint),
        baked_light_uv: lu | lv << 16,
    }
    .to_bytes()
}

/// A mark point on a model as a model vertex, in a row of the mesh's stride.
pub fn pack_mark_model_vertex(
    point: &EffectStainStagingPoint,
    mid_pt: [f32; 3],
    mark_r: f32,
    uv_projection_axis: [f32; 3],
    authored_tint: u32,
) -> [u8; DECAL_MESH_VERTEX_BYTES] {
    let vertex = mark_vertex(point, mid_pt, mark_r, uv_projection_axis, authored_tint).to_bytes();
    let mut vert_bytes = [0; DECAL_MESH_VERTEX_BYTES];
    vert_bytes[..vertex.len()].copy_from_slice(&vertex);
    vert_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_reserved_in_pairs() {
        let mut budget = DecalMeshAllowance::default();
        assert_eq!(generate_mark_verts_begin(&mut budget, 5, 3), Ok((0, 0)));
        assert_eq!((budget.vertex_amount, budget.index_tally), (5, 10));
        assert_eq!(generate_mark_verts_begin(&mut budget, 3, 1), Ok((5, 10)));
        assert_eq!((budget.vertex_amount, budget.index_tally), (8, 14));
        assert_eq!(generate_mark_verts_begin(&mut budget, -2, 0), Ok((8, 14)));
        assert_eq!((budget.vertex_amount, budget.index_tally), (8, 14));
    }

    #[test]
    fn full_budgets_count_what_they_turn_away() {
        let mut budget = DecalMeshAllowance {
            vertex_amount: 6140,
            ..Default::default()
        };
        assert_eq!(
            generate_mark_verts_begin(&mut budget, 5, 1),
            Err(DecalMeshDenial::VertLimit)
        );
        assert_eq!((budget.vertex_amount, budget.vert_soft_cap), (6140, 1));
        // The vertices fit, the indices do not: the vertices stay taken.
        let mut budget = DecalMeshAllowance {
            index_tally: 9210,
            ..Default::default()
        };
        assert_eq!(
            generate_mark_verts_begin(&mut budget, 4, 3),
            Err(DecalMeshDenial::IndicesExhausted)
        );
        assert_eq!(
            (budget.vertex_amount, budget.index_tally, budget.too_many_indices_logged),
            (4, 9210, 1)
        );
        assert_eq!(generate_mark_verts_begin(&mut budget, 4, 2), Ok((4, 9210)));
    }

    #[test]
    fn draw_surfaces_run_out_at_1536() {
        let mut budget = DecalMeshAllowance {
            facet_sum: 1535,
            ..Default::default()
        };
        assert_eq!(add_mark_mesh_draw_surf(&mut budget), Ok(1535));
        assert_eq!(
            add_mark_mesh_draw_surf(&mut budget),
            Err(DecalMeshDenial::SurfLimit)
        );
        assert_eq!((budget.facet_sum, budget.surf_soft_cap), (1536, 1));
    }

    const UP: [f32; 3] = [0.0, 0.0, 1.0];
    const EAST: [f32; 3] = [1.0, 0.0, 0.0];
    const ORIGIN: [f32; 3] = [10.0, 20.0, 0.0];

    fn point(xyz: [f32; 3], baked_light_uv: [f32; 2]) -> EffectStainStagingPoint {
        EffectStainStagingPoint {
            spot: xyz,
            baked_light_uv,
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
        assert_eq!(vertex.baked_light_uv, 16384 | 65535 << 16);
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
