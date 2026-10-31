//! Cutting a triangle down to the part inside a mark's box, and turning that piece into
//! staged triangles and points.

use crate::{EffectStainStagingPoint, EffectStainStagingTri, GfxMarkContext};

/// The most points a cut triangle keeps.
pub const DECAL_CLIP_POLYGON_VERTEX_CEILING: usize = 9;

/// A point this close to a plane (either way) counts as lying on it.
const ON_PLANE: f32 = 0.5;

/// A triangle is skipped when its normal leans more than 60 degrees from the mark's
/// direction: the normal's squared length times cos² 60° must not pass the squared dot.
const LEAN_COS_SQ: f32 = 0.25;

/// A point of a cut triangle: where it is and how much of each of the triangle's three
/// corners it is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldDecalVertex {
    pub spot: [f32; 3],
    pub blend_parts: [f32; 3],
}

impl WorldDecalVertex {
    pub const ZERO: Self = Self {
        spot: [0.0; 3],
        blend_parts: [0.0; 3],
    };

    /// The point `t` of the way from `self` to `to`.
    fn toward(self, to: Self, t: f32) -> Self {
        let mix = |a: [f32; 3], b: [f32; 3]| core::array::from_fn(|k| a[k] + t * (b[k] - a[k]));
        Self {
            spot: mix(self.spot, to.spot),
            blend_parts: mix(self.blend_parts, to.blend_parts),
        }
    }
}

fn dot3(lv: [f32; 3], q: [f32; 3]) -> f32 {
    lv[0] * q[0] + lv[1] * q[1] + lv[2] * q[2]
}

/// The six faces of a mark's box. Each is a normal pointing into the box and a distance:
/// a point is inside a face when `normal · point - distance` is above zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkClipPlaneSet {
    faces: [([f32; 3], f32); 6],
}

/// The box around `centre` whose sides lie `reach` out along each of the mark's axes.
pub fn mark_fragment_clip_planes(
    centre: [f32; 3],
    frame_axes: [[f32; 3]; 3],
    reach: f32,
) -> MarkClipPlaneSet {
    let mut faces = [([0.0; 3], 0.0); 6];
    for (k, normal) in frame_axes.into_iter().enumerate() {
        let along = dot3(normal, centre);
        faces[2 * k] = (normal, along - reach);
        faces[2 * k + 1] = (normal.map(|c| -c), -along - reach);
    }
    MarkClipPlaneSet { faces }
}

/// A triangle the mark skips: one facing away from the mark's direction `dir`, or leaning
/// too far from it. The normal is (v2 - v0) × (v1 - v0).
pub fn mark_is_triangle_rejected(dir: [f32; 3], v0: [f32; 3], v1: [f32; 3], v2: [f32; 3]) -> bool {
    let a = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
    let b = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
    let face_n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let facing = dot3(face_n, dir);
    facing < 0.0 || dot3(face_n, face_n) * LEAN_COS_SQ > facing * facing
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Inside,
    Outside,
    On,
}

/// The part of `poly` inside `face`, written to `out`, and its point count. Nothing inside
/// gives 0; nothing outside gives the polygon unchanged. A point on the face is kept and
/// never starts a new crossing point.
fn keep_inside(
    poly: &[WorldDecalVertex],
    (normal, distance): ([f32; 3], f32),
    out: &mut [WorldDecalVertex; DECAL_CLIP_POLYGON_VERTEX_CEILING],
) -> usize {
    let mut side_d = [0.0f32; DECAL_CLIP_POLYGON_VERTEX_CEILING];
    let mut side = [Side::On; DECAL_CLIP_POLYGON_VERTEX_CEILING];
    for (k, p) in poly.iter().enumerate() {
        side_d[k] = dot3(p.spot, normal) - distance;
        side[k] = if side_d[k] > ON_PLANE {
            Side::Inside
        } else if side_d[k] < -ON_PLANE {
            Side::Outside
        } else {
            Side::On
        };
    }
    let sides = &side[..poly.len()];
    if !sides.contains(&Side::Inside) {
        return 0;
    }
    if !sides.contains(&Side::Outside) {
        out[..poly.len()].copy_from_slice(poly);
        return poly.len();
    }
    let mut n = 0;
    let mut keep = |p: WorldDecalVertex| {
        if n < DECAL_CLIP_POLYGON_VERTEX_CEILING {
            out[n] = p;
            n += 1;
        }
    };
    for k in 0..poly.len() {
        let next = (k + 1) % poly.len();
        if side[k] != Side::Outside {
            keep(poly[k]);
        }
        let crosses = side[k] != Side::On && side[next] != Side::On && side[next] != side[k];
        if crosses {
            keep(poly[k].toward(poly[next], side_d[k] / (side_d[k] - side_d[next])));
        }
    }
    n
}

/// Cuts the triangle `v0 v1 v2` to the inside of the box, writing the piece's points
/// (each with its share of the three corners) to `out`. Returns how many it wrote: 0 when
/// none of the triangle is inside, at most [`DECAL_CLIP_POLYGON_VERTEX_CEILING`].
pub fn mark_chop_world_triangle_points(
    planes: &MarkClipPlaneSet,
    v0: [f32; 3],
    v1: [f32; 3],
    v2: [f32; 3],
    out: &mut [WorldDecalVertex],
) -> u32 {
    let mut poly = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
    poly[0] = WorldDecalVertex {
        spot: v0,
        blend_parts: [1.0, 0.0, 0.0],
    };
    poly[1] = WorldDecalVertex {
        spot: v1,
        blend_parts: [0.0, 1.0, 0.0],
    };
    poly[2] = WorldDecalVertex {
        spot: v2,
        blend_parts: [0.0, 0.0, 1.0],
    };
    let mut n = 3;
    let mut cut = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
    for &face in &planes.faces {
        n = keep_inside(&poly[..n], face, &mut cut);
        if n == 0 {
            return 0;
        }
        poly = cut;
    }
    let n = n.min(out.len());
    out[..n].copy_from_slice(&poly[..n]);
    n as u32
}

/// Why a cut piece was not staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalEmitDenial {
    /// The staging has no room: the mark stops taking pieces.
    Overflow,
    /// Fewer than three points, nothing to draw.
    SliverFragment,
}

/// `w`-weighted sum of the three corners' values.
fn blend<const N: usize>(w: [f32; 3], corners: [[f32; N]; 3]) -> [f32; N] {
    core::array::from_fn(|k| w[0] * corners[0][k] + w[1] * corners[1][k] + w[2] * corners[2][k])
}

/// Stages a cut piece as a fan of triangles around its first point, after the `triangles_consumed`
/// triangles and `points_consumed` points already staged. Each point gets the lightmap spot and
/// normal its corner weights give. Returns the new used counts.
///
/// BO2 asks for room for three times the fan's triangles before it stages any, so a mark
/// near the triangle limit stops early.
#[allow(clippy::too_many_arguments)]
pub fn mark_emit_brush_fragment(
    triangles_consumed: u32,
    points_consumed: u32,
    tri_cap: u32,
    point_cap: u32,
    fragment: &[WorldDecalVertex],
    lm_a: [f32; 2],
    lm_b: [f32; 2],
    lm_c: [f32; 2],
    v0_normal: [f32; 3],
    v1_normal: [f32; 3],
    v2_normal: [f32; 3],
    context: GfxMarkContext,
    triangle_list: &mut [EffectStainStagingTri],
    points: &mut [EffectStainStagingPoint],
) -> Result<(u32, u32), DecalEmitDenial> {
    let n = fragment.len();
    if n < 3 {
        return Err(DecalEmitDenial::SliverFragment);
    }
    let fan = n as u32 - 2;
    let point_room = i64::from(point_cap) - i64::from(points_consumed);
    let tri_room = i64::from(tri_cap) - i64::from(triangles_consumed);
    if point_room < n as i64 || tri_room < 3 * i64::from(fan) {
        return Err(DecalEmitDenial::Overflow);
    }
    let tris_after = triangles_consumed.saturating_add(fan);
    let point_range_stop = points_consumed.saturating_add(n as u32);
    let (Some(new_tris), Some(new_points)) = (
        triangle_list.get_mut(triangles_consumed as usize..tris_after as usize),
        points.get_mut(points_consumed as usize..point_range_stop as usize),
    ) else {
        return Err(DecalEmitDenial::Overflow);
    };
    let first = points_consumed as u16;
    for (k, tri) in new_tris.iter_mut().enumerate() {
        let b = first.wrapping_add(k as u16 + 1);
        *tri = EffectStainStagingTri {
            indices: [b, b.wrapping_add(1), first],
            context,
            tail_fill: 0,
        };
    }
    for (p, staged) in fragment.iter().zip(new_points) {
        *staged = EffectStainStagingPoint {
            spot: p.spot,
            baked_light_uv: blend(p.blend_parts, [lm_a, lm_b, lm_c]),
            normal: blend(p.blend_parts, [v0_normal, v1_normal, v2_normal]),
        };
    }
    Ok((tris_after, point_range_stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    #[test]
    fn box_faces_point_inward() {
        let set = mark_fragment_clip_planes([10.0, 20.0, 30.0], UP, 4.0);
        assert_eq!(set.faces[0], ([0.0, 0.0, 1.0], 26.0));
        assert_eq!(set.faces[1], ([-0.0, -0.0, -1.0], -34.0));
        assert_eq!(set.faces[2], ([1.0, 0.0, 0.0], 6.0));
        assert_eq!(set.faces[5], ([-0.0, -1.0, -0.0], -24.0));
    }

    #[test]
    fn triangles_facing_away_or_leaning_are_skipped() {
        // (v2 - v0) × (v1 - v0) is +z for this winding.
        let (v0, v1, v2) = ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        assert!(!mark_is_triangle_rejected([0.0, 0.0, 1.0], v0, v1, v2));
        assert!(mark_is_triangle_rejected([0.0, 0.0, -1.0], v0, v1, v2));
        // 59 degrees off passes, 61 does not.
        assert!(!mark_is_triangle_rejected(
            [0.857_167_3, 0.0, 0.515_038_1],
            v0,
            v1,
            v2
        ));
        assert!(mark_is_triangle_rejected(
            [0.874_619_7, 0.0, 0.484_809_6],
            v0,
            v1,
            v2
        ));
    }

    #[test]
    fn a_triangle_inside_the_box_comes_through_whole() {
        let set = mark_fragment_clip_planes([0.0; 3], UP, 10.0);
        let mut out = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
        let n = mark_chop_world_triangle_points(
            &set,
            [0.0; 3],
            [0.0, 2.0, 0.0],
            [2.0, 0.0, 0.0],
            &mut out,
        );
        assert_eq!(n, 3);
        assert_eq!(
            out[1],
            WorldDecalVertex {
                spot: [0.0, 2.0, 0.0],
                blend_parts: [0.0, 1.0, 0.0]
            }
        );
    }

    #[test]
    fn a_triangle_outside_the_box_is_gone() {
        let set = mark_fragment_clip_planes([0.0; 3], UP, 10.0);
        let mut out = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
        let far = |x: f32| [x, 0.0, 0.0];
        assert_eq!(
            mark_chop_world_triangle_points(&set, far(20.0), far(30.0), [20.0, 5.0, 0.0], &mut out),
            0
        );
    }

    #[test]
    fn a_big_triangle_is_cut_to_the_box_with_corner_shares() {
        let set = mark_fragment_clip_planes([0.0; 3], UP, 10.0);
        let mut out = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
        let n = mark_chop_world_triangle_points(
            &set,
            [-100.0, -100.0, 0.0],
            [-100.0, 300.0, 0.0],
            [300.0, -100.0, 0.0],
            &mut out,
        ) as usize;
        // The square face of the box is all inside this triangle.
        assert_eq!(n, 4);
        for p in &out[..n] {
            assert!(
                p.spot[0].abs() <= 10.0 + 1e-3 && p.spot[1].abs() <= 10.0 + 1e-3,
                "{p:?}"
            );
            let total: f32 = p.blend_parts.iter().sum();
            assert!((total - 1.0).abs() < 1e-5, "{p:?}");
            let back = blend(
                p.blend_parts,
                [[-100.0, -100.0], [-100.0, 300.0], [300.0, -100.0]],
            );
            assert!((back[0] - p.spot[0]).abs() < 1e-3 && (back[1] - p.spot[1]).abs() < 1e-3);
        }
    }

    #[test]
    fn points_near_a_face_count_as_on_it() {
        let set = mark_fragment_clip_planes([0.0; 3], UP, 10.0);
        let mut out = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
        let mut chop = |x: f32| {
            mark_chop_world_triangle_points(
                &set,
                [5.0, 0.0, 0.0],
                [5.0, 1.0, 0.0],
                [x, 0.0, 0.0],
                &mut out,
            )
        };
        // A corner 0.3 past the side is on it and stays; 0.6 past is cut off.
        assert_eq!(chop(10.3), 3);
        assert_eq!(chop(10.6), 4);
        // A triangle lying all on a side has nothing inside.
        let flat = mark_fragment_clip_planes([0.0; 3], UP, 0.2);
        assert_eq!(
            mark_chop_world_triangle_points(
                &flat,
                [0.0; 3],
                [0.0, 0.1, 0.0],
                [0.1, 0.0, 0.0],
                &mut out
            ),
            0
        );
    }

    fn piece(n: usize) -> [WorldDecalVertex; DECAL_CLIP_POLYGON_VERTEX_CEILING] {
        let mut p = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
        for (k, pt) in p.iter_mut().enumerate().take(n) {
            pt.spot = [k as f32, 0.0, 0.0];
            pt.blend_parts = [0.5, 0.25, 0.25];
        }
        p
    }

    #[test]
    fn a_piece_fans_around_its_first_point() {
        let ctx = GfxMarkContext::world_surface(3, 4, 5);
        let mut triangle_list = [EffectStainStagingTri::ZERO; 8];
        let mut points = [EffectStainStagingPoint::ZERO; 8];
        let frag = piece(5);
        let got = mark_emit_brush_fragment(
            1,
            2,
            255,
            765,
            &frag[..5],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            ctx,
            &mut triangle_list,
            &mut points,
        );
        assert_eq!(got, Ok((4, 7)));
        assert_eq!(triangle_list[0], EffectStainStagingTri::ZERO);
        assert_eq!(triangle_list[1].indices, [3, 4, 2]);
        assert_eq!(triangle_list[2].indices, [4, 5, 2]);
        assert_eq!(triangle_list[3].indices, [5, 6, 2]);
        assert_eq!(triangle_list[3].context, ctx);
        assert_eq!(points[6].spot, [4.0, 0.0, 0.0]);
        assert_eq!(points[6].baked_light_uv, [0.25, 0.25]);
        assert_eq!(points[6].normal, [0.0, 0.25, 0.75]);
    }

    #[test]
    fn a_piece_needs_room_for_three_fans() {
        let mut triangle_list = [EffectStainStagingTri::ZERO; 16];
        let mut points = [EffectStainStagingPoint::ZERO; 16];
        let frag = piece(4);
        let emit = |triangles_consumed,
                    max_tris,
                    triangle_list: &mut [EffectStainStagingTri],
                    points: &mut [EffectStainStagingPoint]| {
            mark_emit_brush_fragment(
                triangles_consumed,
                0,
                max_tris,
                16,
                &frag[..4],
                [0.0; 2],
                [0.0; 2],
                [0.0; 2],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                GfxMarkContext::ZERO,
                triangle_list,
                points,
            )
        };
        // Two triangles in the fan need six free.
        assert_eq!(
            emit(10, 15, &mut triangle_list, &mut points),
            Err(DecalEmitDenial::Overflow)
        );
        assert_eq!(emit(10, 16, &mut triangle_list, &mut points), Ok((12, 4)));
        // The slices must hold them too.
        assert_eq!(
            emit(15, 255, &mut triangle_list, &mut points),
            Err(DecalEmitDenial::Overflow)
        );
        assert_eq!(
            emit(0, 255, &mut triangle_list, &mut points[..3]),
            Err(DecalEmitDenial::Overflow)
        );
        let two = &frag[..2];
        assert_eq!(
            mark_emit_brush_fragment(
                0,
                0,
                255,
                765,
                two,
                [0.0; 2],
                [0.0; 2],
                [0.0; 2],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                GfxMarkContext::ZERO,
                &mut triangle_list,
                &mut points,
            ),
            Err(DecalEmitDenial::SliverFragment)
        );
    }
}
