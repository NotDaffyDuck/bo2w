//! The frame's mark geometry (bullet holes, blood, scorch) as the mark mesh
//! plan: the shared vertex block, plus index runs ordered the way
//! Black Ops II orders its mark draw surfaces.

use std::sync::Arc;

use dpvs_t6::pack_mark_mesh_draw_surf;
use fx::{FxSystemHost, GfxMarkMeshCensus};

use super::mesh_bind::mark_mesh_bind;
use crate::assemble::drawsurf::{GfxMarkMeshPlan, GfxMarkSubKey, MaterialGeneration};

/// A mark surface whose material was found, waiting to be sorted.
struct Queued {
    sort_key: u8,
    ordinal: u32,
    sub_key: GfxMarkSubKey,
    first_index: usize,
    index_count: usize,
}

/// Replaces the mark plan with this frame's geometry; with none, the plan
/// is published as it stands.
pub(super) fn fill_mark_mesh_plan(
    host: &mut FxSystemHost,
    mesh: Option<GfxMarkMeshCensus>,
    plan: &mut GfxMarkMeshPlan,
    runtime: &MaterialGeneration,
) {
    host.gfx_mark_draw_n = mesh.map(|mesh| {
        let marks = &host.marks;
        build_mark_plan(mesh, plan, |slot| {
            let name = marks.material_name(slot).ok_or("no_material")?;
            mark_mesh_bind(name, runtime).ok_or("no_ordinal")
        });
        plan.draws.len() as u32
    });
    plan.bump();
    plan.publish_share();
}

/// `bind` gives a mark slot's `(sort key, material ordinal)`, or the reason
/// it has none. The last reason seen is kept on the plan.
fn build_mark_plan(
    mesh: GfxMarkMeshCensus,
    plan: &mut GfxMarkMeshPlan,
    bind: impl Fn(u16) -> Result<(u8, u32), &'static str>,
) {
    let GfxMarkMeshCensus {
        budget,
        packed,
        indices,
        surfs,
        ..
    } = mesh;
    plan.vertices = Arc::new(packed);
    Arc::make_mut(&mut plan.indices).reserve(indices.len());

    let mut refusal = None;
    let mut queue: Vec<(u64, Queued)> = Vec::with_capacity(surfs.len());
    for surf in surfs.iter().filter(|surf| surf.index_count > 0) {
        let (sort_key, ordinal) = match bind(surf.mark_slot) {
            Ok(bound) => bound,
            Err(why) => {
                refusal = Some(why);
                continue;
            }
        };
        let sub_key = GfxMarkSubKey::from_context(&surf.context);
        let order = pack_mark_mesh_draw_surf(
            sort_key,
            render_material::sort_band(ordinal),
            0,
            sub_key.lmap,
            sub_key.primary_light,
            sub_key.probe,
        )
        .packed;
        queue.push((
            order,
            Queued {
                sort_key,
                ordinal,
                sub_key,
                first_index: surf.index_start as usize,
                index_count: surf.index_count as usize,
            },
        ));
    }
    queue.sort_by_key(|(order, _)| *order);

    for (_, run) in queue {
        let end = run.first_index.saturating_add(run.index_count);
        match indices.get(run.first_index..end) {
            Some(slice) => {
                plan.append_run_indices(run.sort_key, Some(run.ordinal), run.sub_key, slice);
            }
            None => refusal = Some("range_out_of_mesh"),
        }
    }
    if budget.surf_n > 0 && plan.draws.is_empty() && refusal.is_none() {
        refusal = Some("empty_range");
    }
    plan.skip_why = refusal;
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx::GfxMarkMeshSurf;

    fn surf(mark_slot: u16, index_start: u32, index_count: u32) -> GfxMarkMeshSurf {
        GfxMarkMeshSurf {
            index_start,
            index_count,
            mark_slot,
            ..Default::default()
        }
    }

    fn census(indices: Vec<u16>, surfs: Vec<GfxMarkMeshSurf>) -> GfxMarkMeshCensus {
        let mut census = GfxMarkMeshCensus {
            indices,
            surfs,
            ..Default::default()
        };
        census.budget.surf_n = census.surfs.len() as _;
        census
    }

    /// Slot 1 and 2 are materials with different sort keys; slot 9 has none.
    fn bind(slot: u16) -> Result<(u8, u32), &'static str> {
        match slot {
            1 => Ok((3, 10)),
            2 => Ok((5, 11)),
            3 => Err("no_ordinal"),
            _ => Err("no_material"),
        }
    }

    fn draw_ranges(plan: &GfxMarkMeshPlan) -> Vec<(u32, u32)> {
        plan.draws.iter().map(|d| (d.index_start, d.index_count)).collect()
    }

    #[test]
    fn neighbouring_surfaces_of_one_material_share_a_draw() {
        let mut plan = GfxMarkMeshPlan::default();
        let mesh = census((0..12).collect(), vec![surf(1, 0, 3), surf(1, 3, 3), surf(1, 6, 6)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(draw_ranges(&plan), vec![(0, 12)]);
        assert_eq!(plan.skip_why, None);
        assert_eq!(plan.indices.len(), 12);
    }

    #[test]
    fn materials_are_drawn_in_sort_order_whatever_the_surface_order() {
        let mut plan = GfxMarkMeshPlan::default();
        // The sort-key-5 surface comes first in the mesh, the key-3 one second.
        let mesh = census((0..6).collect(), vec![surf(2, 0, 3), surf(1, 3, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        let keys: Vec<u8> = plan
            .draws
            .iter()
            .map(|d| plan.materials[d.material as usize].sort_key)
            .collect();
        assert_eq!(keys, vec![3, 5]);
        // The runs are appended in that order, so the key-3 indices come first.
        assert_eq!(plan.indices.as_slice(), &[3, 4, 5, 0, 1, 2]);
    }

    #[test]
    fn surfaces_without_indices_are_ignored() {
        let mut plan = GfxMarkMeshPlan::default();
        let mesh = census(vec![0, 1, 2], vec![surf(1, 0, 0), surf(1, 0, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(draw_ranges(&plan), vec![(0, 3)]);
        assert_eq!(plan.skip_why, None);
    }

    #[test]
    fn the_last_unbound_reason_is_kept() {
        let mut plan = GfxMarkMeshPlan::default();
        let mesh = census((0..9).collect(), vec![surf(9, 0, 3), surf(3, 3, 3), surf(1, 6, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.skip_why, Some("no_ordinal"));
        assert_eq!(draw_ranges(&plan), vec![(0, 3)]);
    }

    #[test]
    fn a_run_past_the_end_of_the_mesh_is_refused() {
        let mut plan = GfxMarkMeshPlan::default();
        let mesh = census(vec![0, 1, 2], vec![surf(1, 0, 3), surf(2, 2, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.skip_why, Some("range_out_of_mesh"));
        assert_eq!(draw_ranges(&plan), vec![(0, 3)]);
    }

    #[test]
    fn surfaces_that_all_come_out_empty_are_reported() {
        let mut plan = GfxMarkMeshPlan::default();
        let mesh = census(Vec::new(), vec![surf(1, 0, 0)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.skip_why, Some("empty_range"));
        assert!(plan.draws.is_empty());
    }

    #[test]
    fn a_frame_without_surfaces_has_nothing_to_report() {
        let mut plan = GfxMarkMeshPlan::default();
        build_mark_plan(census(Vec::new(), Vec::new()), &mut plan, bind);
        assert_eq!(plan.skip_why, None);
    }
}
