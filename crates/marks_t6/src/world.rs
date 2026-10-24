//! Staging a mark on the world: every surface its box reaches, triangle by triangle.

use crate::{
    FxMarkEmitRefuse, FxMarkStagingPoint, FxMarkStagingTri, FxWorldMarkPoint, GfxMarkContext,
    R_MARK_CHOP_MAX_POINTS, mark_chop_world_triangle_points, mark_emit_brush_fragment,
    mark_fragment_clip_planes, mark_is_triangle_rejected,
};

/// The most world surfaces one mark is cut against.
const SURFACE_LIMIT: u32 = 256;

/// Whether a sphere (`origin`, squared radius) reaches a box given by its middle and half
/// size.
pub fn mark_sphere_hits_bounds(
    origin: [f32; 3],
    radius_sq: f32,
    mid: [f32; 3],
    half: [f32; 3],
) -> bool {
    let gap = |k: usize| ((origin[k] - mid[k]).abs() - half[k]).max(0.0);
    let (x, y, z) = (gap(0), gap(1), gap(2));
    x * x + y * y + z * z <= radius_sq
}

/// How far staging a mark on the world got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MarkWorldStaging {
    pub used_tri: u32,
    pub used_point: u32,
    /// The staging ran out of room and the mark stopped there.
    pub overflow: bool,
}

/// Cuts the mark's box (`origin`, `radius`, `axis`; `axis[0]` is the way the mark faces)
/// against the world surfaces in order and stages every piece.
///
/// Surface `s` has bounds `mids[s]`/`halves[s]`, its triangles as the `(start, count)` run
/// `surface_index_ranges[s]` of `packed_indices` (three per triangle), and its context
/// `contexts[s]`. The corner numbers index `positions`, `lightmap_uvs` and `normals`. With
/// `retest_sphere` a surface the mark's sphere misses is passed over. At most 256 surfaces
/// are cut; a full staging stops everything and sets `overflow`.
#[allow(clippy::too_many_arguments)]
pub fn mark_stage_world_surfaces(
    origin: [f32; 3],
    radius: f32,
    axis: [[f32; 3]; 3],
    mids: &[[f32; 3]],
    halves: &[[f32; 3]],
    positions: &[[f32; 3]],
    lightmap_uvs: &[[f32; 2]],
    normals: &[[f32; 3]],
    packed_indices: &[u32],
    surface_index_ranges: &[(u32, u32)],
    contexts: &[GfxMarkContext],
    max_tris: u32,
    max_points: u32,
    tris: &mut [FxMarkStagingTri],
    points: &mut [FxMarkStagingPoint],
    retest_sphere: bool,
) -> MarkWorldStaging {
    let planes = mark_fragment_clip_planes(origin, axis, radius);
    let radius_sq = radius * radius;
    let mut staged = MarkWorldStaging::default();
    let mut piece = [FxWorldMarkPoint::ZERO; R_MARK_CHOP_MAX_POINTS];
    let mut taken = 0u32;
    let surfaces = mids
        .iter()
        .zip(halves)
        .zip(surface_index_ranges)
        .enumerate();
    for (s, ((&mid, &half), &(start, count))) in surfaces {
        if taken >= SURFACE_LIMIT {
            break;
        }
        if retest_sphere && !mark_sphere_hits_bounds(origin, radius_sq, mid, half) {
            continue;
        }
        taken += 1;
        let start = start as usize;
        if count < 3 || start >= packed_indices.len() {
            continue;
        }
        let end = start
            .saturating_add(count as usize)
            .min(packed_indices.len());
        let context = contexts.get(s).copied().unwrap_or(GfxMarkContext::ZERO);
        for corners in packed_indices[start..end].chunks_exact(3) {
            let ids = [corners[0], corners[1], corners[2]].map(|i| i as usize);
            let Some([v0, v1, v2]) = corner_values(positions, ids) else {
                continue;
            };
            if mark_is_triangle_rejected(axis[0], v0, v1, v2) {
                continue;
            }
            let n = mark_chop_world_triangle_points(&planes, v0, v1, v2, &mut piece) as usize;
            if n < 3 {
                continue;
            }
            let (Some(lmap), Some(normal)) = (
                corner_values(lightmap_uvs, ids),
                corner_values(normals, ids),
            ) else {
                continue;
            };
            match mark_emit_brush_fragment(
                staged.used_tri,
                staged.used_point,
                max_tris,
                max_points,
                &piece[..n],
                lmap[0],
                lmap[1],
                lmap[2],
                normal[0],
                normal[1],
                normal[2],
                context,
                tris,
                points,
            ) {
                Ok((used_tri, used_point)) => {
                    staged.used_tri = used_tri;
                    staged.used_point = used_point;
                }
                Err(FxMarkEmitRefuse::Overflow) => {
                    staged.overflow = true;
                    return staged;
                }
                Err(FxMarkEmitRefuse::FragmentTooSmall) => {}
            }
        }
    }
    staged
}

/// The three corners' entries of `values`, `None` when a number is past its end.
fn corner_values<T: Copy>(values: &[T], ids: [usize; 3]) -> Option<[T; 3]> {
    Some([
        *values.get(ids[0])?,
        *values.get(ids[1])?,
        *values.get(ids[2])?,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{R_MARK_FRAGMENTS_MAX_POINTS, R_MARK_FRAGMENTS_MAX_TRIS};

    #[test]
    fn sphere_against_box() {
        let (mid, half) = ([0.0; 3], [1.0, 2.0, 3.0]);
        assert!(mark_sphere_hits_bounds([0.5, 0.0, 0.0], 0.0, mid, half));
        assert!(mark_sphere_hits_bounds([4.0, 0.0, 0.0], 9.0, mid, half));
        assert!(!mark_sphere_hits_bounds([4.0, 0.0, 0.0], 8.9, mid, half));
        // Corner gaps add: 3 along x and 4 along y is 5 away.
        assert!(mark_sphere_hits_bounds([4.0, -6.0, 0.0], 25.0, mid, half));
        assert!(!mark_sphere_hits_bounds([4.0, -6.0, 0.0], 24.9, mid, half));
    }

    /// A floor of two triangles, 40 units square, facing +z.
    struct Floor {
        positions: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
        normals: [[f32; 3]; 4],
        indices: [u32; 6],
    }

    const FLOOR: Floor = Floor {
        positions: [
            [-20.0, -20.0, 0.0],
            [20.0, -20.0, 0.0],
            [20.0, 20.0, 0.0],
            [-20.0, 20.0, 0.0],
        ],
        uvs: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        normals: [[0.0, 0.0, 1.0]; 4],
        // Wound so (v2 - v0) × (v1 - v0) is +z.
        indices: [0, 2, 1, 0, 3, 2],
    };

    const DOWN_ONTO_FLOOR: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    fn stage(
        ranges: &[(u32, u32)],
        mids: &[[f32; 3]],
        max_tris: u32,
        retest: bool,
    ) -> (MarkWorldStaging, [FxMarkStagingTri; 255]) {
        let mut tris = [FxMarkStagingTri::ZERO; R_MARK_FRAGMENTS_MAX_TRIS as usize];
        let mut points = [FxMarkStagingPoint::ZERO; R_MARK_FRAGMENTS_MAX_POINTS as usize];
        let halves = [[20.0, 20.0, 0.0]; 4];
        let contexts = [
            GfxMarkContext::world_surface(1, 2, 3),
            GfxMarkContext::world_surface(4, 5, 6),
        ];
        let staged = mark_stage_world_surfaces(
            [0.0, 0.0, 1.0],
            8.0,
            DOWN_ONTO_FLOOR,
            mids,
            &halves[..mids.len()],
            &FLOOR.positions,
            &FLOOR.uvs,
            &FLOOR.normals,
            &FLOOR.indices,
            ranges,
            &contexts,
            max_tris,
            R_MARK_FRAGMENTS_MAX_POINTS,
            &mut tris,
            &mut points,
            retest,
        );
        (staged, tris)
    }

    #[test]
    fn a_mark_on_the_floor_covers_its_square() {
        let (staged, tris) = stage(&[(0, 6)], &[[0.0; 3]], R_MARK_FRAGMENTS_MAX_TRIS, true);
        assert!(!staged.overflow);
        // Each floor triangle keeps its half of the 16-unit square: one triangle, three points.
        assert_eq!((staged.used_tri, staged.used_point), (2, 6));
        assert_eq!(tris[0].context, GfxMarkContext::world_surface(1, 2, 3));
    }

    #[test]
    fn surfaces_the_sphere_misses_are_skipped_only_when_asked() {
        let far = [[0.0, 0.0, 500.0]];
        assert_eq!(
            stage(&[(0, 6)], &far, R_MARK_FRAGMENTS_MAX_TRIS, true).0,
            MarkWorldStaging::default()
        );
        assert!(
            stage(&[(0, 6)], &far, R_MARK_FRAGMENTS_MAX_TRIS, false)
                .0
                .used_tri
                > 0
        );
    }

    #[test]
    fn each_surface_uses_its_own_context_and_short_runs_are_nothing() {
        let (staged, tris) = stage(
            &[(0, 2), (3, 3)],
            &[[0.0; 3]; 2],
            R_MARK_FRAGMENTS_MAX_TRIS,
            false,
        );
        assert!(staged.used_tri > 0);
        let used = &tris[..staged.used_tri as usize];
        assert!(
            used.iter()
                .all(|t| t.context == GfxMarkContext::world_surface(4, 5, 6))
        );
    }

    #[test]
    fn a_full_staging_stops_the_mark() {
        let (staged, _) = stage(&[(0, 6)], &[[0.0; 3]], 3, false);
        assert!(staged.overflow);
    }

    #[test]
    fn corner_numbers_past_the_tables_skip_the_triangle() {
        assert_eq!(corner_values(&[1, 2, 3], [0, 2, 1]), Some([1, 3, 2]));
        assert_eq!(corner_values(&[1, 2, 3], [0, 3, 1]), None);
    }
}
