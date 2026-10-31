//! Staging a mark on the world: every surface its box reaches, triangle by triangle.

use crate::{
    DecalEmitDenial, EffectStainStagingPoint, EffectStainStagingTri, WorldDecalVertex, GfxMarkContext,
    DECAL_CLIP_POLYGON_VERTEX_CEILING, mark_chop_world_triangle_points, mark_emit_brush_fragment,
    mark_fragment_clip_planes, mark_is_triangle_rejected,
};

/// The most world surfaces one mark is cut against.
const SURFACE_LIMIT: u32 = 256;

/// Whether a sphere (`centre`, squared radius) reaches a box given by its middle and half
/// size.
pub fn mark_sphere_hits_bounds(
    centre: [f32; 3],
    reach_squared: f32,
    mid: [f32; 3],
    half: [f32; 3],
) -> bool {
    let gap = |k: usize| ((centre[k] - mid[k]).abs() - half[k]).max(0.0);
    let (x, y, z) = (gap(0), gap(1), gap(2));
    x * x + y * y + z * z <= reach_squared
}

/// How far staging a mark on the world got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StainLevelStaging {
    pub triangles_consumed: u32,
    pub points_consumed: u32,
    /// The staging ran out of room and the mark stopped there.
    pub overflow: bool,
}

/// Cuts the mark's box (`hub_pt`, `reach`, `frame_axes`; `frame_axes[0]` is the way the mark faces)
/// against the world surfaces in order and stages every piece.
///
/// Surface `s` has bounds `mids[s]`/`halves[s]`, its triangles as the `(start, count)` run
/// `facet_index_ranges[s]` of `compact_indices` (three per triangle), and its context
/// `contexts[s]`. The corner numbers index `vert_pos`, `lightbake_uvs` and `surface_directions`. With
/// `recheck_ball` a surface the mark's sphere misses is passed over. At most 256 surfaces
/// are cut; a full staging stops everything and sets `overflow`.
#[allow(clippy::too_many_arguments)]
pub fn mark_stage_world_surfaces(
    hub_pt: [f32; 3],
    reach: f32,
    frame_axes: [[f32; 3]; 3],
    centres: &[[f32; 3]],
    extents: &[[f32; 3]],
    vert_pos: &[[f32; 3]],
    lightbake_uvs: &[[f32; 2]],
    surface_directions: &[[f32; 3]],
    compact_indices: &[u32],
    facet_index_ranges: &[(u32, u32)],
    contexts: &[GfxMarkContext],
    tri_cap: u32,
    point_cap: u32,
    triangle_list: &mut [EffectStainStagingTri],
    points: &mut [EffectStainStagingPoint],
    recheck_ball: bool,
) -> StainLevelStaging {
    let planes = mark_fragment_clip_planes(hub_pt, frame_axes, reach);
    let reach_squared = reach * reach;
    let mut staged = StainLevelStaging::default();
    let mut piece = [WorldDecalVertex::ZERO; DECAL_CLIP_POLYGON_VERTEX_CEILING];
    let mut taken = 0u32;
    let surfaces = centres
        .iter()
        .zip(extents)
        .zip(facet_index_ranges)
        .enumerate();
    for (s, ((&mid, &half), &(start, count))) in surfaces {
        if taken >= SURFACE_LIMIT {
            break;
        }
        if recheck_ball && !mark_sphere_hits_bounds(hub_pt, reach_squared, mid, half) {
            continue;
        }
        taken += 1;
        let run_at = start as usize;
        if count < 3 || run_at >= compact_indices.len() {
            continue;
        }
        let end = run_at
            .saturating_add(count as usize)
            .min(compact_indices.len());
        let context = contexts.get(s).copied().unwrap_or(GfxMarkContext::ZERO);
        for corners in compact_indices[run_at..end].chunks_exact(3) {
            let ids = [corners[0], corners[1], corners[2]].map(|i| i as usize);
            let Some([v0, v1, v2]) = corner_values(vert_pos, ids) else {
                continue;
            };
            if mark_is_triangle_rejected(frame_axes[0], v0, v1, v2) {
                continue;
            }
            let n = mark_chop_world_triangle_points(&planes, v0, v1, v2, &mut piece) as usize;
            if n < 3 {
                continue;
            }
            let (Some(baked_light_slot), Some(normal)) = (
                corner_values(lightbake_uvs, ids),
                corner_values(surface_directions, ids),
            ) else {
                continue;
            };
            match mark_emit_brush_fragment(
                staged.triangles_consumed,
                staged.points_consumed,
                tri_cap,
                point_cap,
                &piece[..n],
                baked_light_slot[0],
                baked_light_slot[1],
                baked_light_slot[2],
                normal[0],
                normal[1],
                normal[2],
                context,
                triangle_list,
                points,
            ) {
                Ok((triangles_consumed, points_consumed)) => {
                    staged.triangles_consumed = triangles_consumed;
                    staged.points_consumed = points_consumed;
                }
                Err(DecalEmitDenial::Overflow) => {
                    staged.overflow = true;
                    return staged;
                }
                Err(DecalEmitDenial::SliverFragment) => {}
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
    use crate::{DECAL_FRAGMENT_VERTEX_CEILING, DECAL_FRAGMENT_TRIANGLE_CEILING};

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
        texcoord_set: [[f32; 2]; 4],
        surface_directions: [[f32; 3]; 4],
        indices: [u32; 6],
    }

    const FLOOR: Floor = Floor {
        positions: [
            [-20.0, -20.0, 0.0],
            [20.0, -20.0, 0.0],
            [20.0, 20.0, 0.0],
            [-20.0, 20.0, 0.0],
        ],
        texcoord_set: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        surface_directions: [[0.0, 0.0, 1.0]; 4],
        // Wound so (v2 - v0) × (v1 - v0) is +z.
        indices: [0, 2, 1, 0, 3, 2],
    };

    const DOWN_ONTO_FLOOR: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    fn stage(
        spans: &[(u32, u32)],
        centre_pts: &[[f32; 3]],
        tri_room: u32,
        retest: bool,
    ) -> (StainLevelStaging, [EffectStainStagingTri; 255]) {
        let mut triangle_list = [EffectStainStagingTri::ZERO; DECAL_FRAGMENT_TRIANGLE_CEILING as usize];
        let mut points = [EffectStainStagingPoint::ZERO; DECAL_FRAGMENT_VERTEX_CEILING as usize];
        let halves = [[20.0, 20.0, 0.0]; 4];
        let contexts = [
            GfxMarkContext::world_surface(1, 2, 3),
            GfxMarkContext::world_surface(4, 5, 6),
        ];
        let staged = mark_stage_world_surfaces(
            [0.0, 0.0, 1.0],
            8.0,
            DOWN_ONTO_FLOOR,
            centre_pts,
            &halves[..centre_pts.len()],
            &FLOOR.positions,
            &FLOOR.texcoord_set,
            &FLOOR.surface_directions,
            &FLOOR.indices,
            spans,
            &contexts,
            tri_room,
            DECAL_FRAGMENT_VERTEX_CEILING,
            &mut triangle_list,
            &mut points,
            retest,
        );
        (staged, triangle_list)
    }

    #[test]
    fn a_mark_on_the_floor_covers_its_square() {
        let (staged, triangle_list) = stage(&[(0, 6)], &[[0.0; 3]], DECAL_FRAGMENT_TRIANGLE_CEILING, true);
        assert!(!staged.overflow);
        // Each floor triangle keeps its half of the 16-unit square: one triangle, three points.
        assert_eq!((staged.triangles_consumed, staged.points_consumed), (2, 6));
        assert_eq!(triangle_list[0].context, GfxMarkContext::world_surface(1, 2, 3));
    }

    #[test]
    fn surfaces_the_sphere_misses_are_skipped_only_when_asked() {
        let far = [[0.0, 0.0, 500.0]];
        assert_eq!(
            stage(&[(0, 6)], &far, DECAL_FRAGMENT_TRIANGLE_CEILING, true).0,
            StainLevelStaging::default()
        );
        assert!(
            stage(&[(0, 6)], &far, DECAL_FRAGMENT_TRIANGLE_CEILING, false)
                .0
                .triangles_consumed
                > 0
        );
    }

    #[test]
    fn each_surface_uses_its_own_context_and_short_runs_are_nothing() {
        let (staged, triangle_list) = stage(
            &[(0, 2), (3, 3)],
            &[[0.0; 3]; 2],
            DECAL_FRAGMENT_TRIANGLE_CEILING,
            false,
        );
        assert!(staged.triangles_consumed > 0);
        let used = &triangle_list[..staged.triangles_consumed as usize];
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
