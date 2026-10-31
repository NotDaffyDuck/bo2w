//! The frame's mark geometry (bullet holes, blood, scorch) as the mark mesh
//! plan: the shared vertex block, plus index runs ordered the way
//! Black Ops II orders its mark draw surfaces.

use std::sync::Arc;

use dpvs_t6::pack_stain_mesh_facet_draw;
use fx::{EffectSystemHost, DecalMeshTally};

use super::mesh_bind::mark_mesh_bind;
use crate::assemble::draw_items::{VisualStainMeshPlan, DecalSubmeshKey, MaterialEpoch};

/// A mark surface whose material was found, waiting to be sorted: where it
/// sorts, which material it uses, and the stretch of indices it draws.
struct Queued {
    order: u64,
    material: (u8, u32),
    light: DecalSubmeshKey,
    span: std::ops::Range<usize>,
}

/// The draw-surface sort word of one mark surface.
fn sort_word(sort_key: u8, ord: u32, light: &DecalSubmeshKey) -> u64 {
    pack_stain_mesh_facet_draw(
        sort_key,
        render_material::sort_band(ord),
        0,
        light.baked_light_slot,
        light.dominant_light,
        light.probe,
    )
    .packed
}

/// Replaces the mark plan with this frame's geometry; with none, the plan
/// is published as it stands.
pub(super) fn fill_mark_mesh_plan(
    host: &mut EffectSystemHost,
    mesh: Option<DecalMeshTally>,
    draw_plan: &mut VisualStainMeshPlan,
    runtime: &MaterialEpoch,
) {
    host.stain_draws = mesh.map(|mesh| {
        let marks = &host.marks;
        build_mark_plan(mesh, draw_plan, |slot| {
            let name = marks.surface_title(slot).ok_or(BindMiss::NoMaterial)?;
            mark_mesh_bind(name, runtime).ok_or(BindMiss::NoOrdinal)
        });
        draw_plan.draws.len() as u32
    });
    draw_plan.next_edit();
    draw_plan.release_shared_copy();
}


/// Why a slot could not be bound to a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BindMiss {
    NoMaterial,
    NoOrdinal,
}

impl BindMiss {
    /// The reason kept on the plan.
    fn cause(self) -> &'static str {
        match self {
            BindMiss::NoMaterial => "no_material",
            BindMiss::NoOrdinal => "no_ordinal",
        }
    }
}

/// `bind` gives a mark slot's `(sort key, material ordinal)`, or the reason
/// it has none. The last reason seen is kept on the plan.
fn build_mark_plan(
    mesh: DecalMeshTally,
    plan: &mut VisualStainMeshPlan,
    bind: impl Fn(u16) -> Result<(u8, u32), BindMiss>,
) {
    let DecalMeshTally {
        budget,
        vert_blob: packed,
        index_list: indices,
        surfs,
        ..
    } = mesh;
    plan.vertices = Arc::new(packed);
    Arc::make_mut(&mut plan.indices).reserve(indices.len());

    let mut refusal = None;
    let mut queue = Vec::with_capacity(surfs.len());
    for surf in surfs.iter().filter(|surf| surf.index_count > 0) {
        match bind(surf.stain_cell) {
            Ok((sort_key, ordinal)) => {
                let light = DecalSubmeshKey::from_context(&surf.context);
                let first = surf.first_triangle_slot as usize;
                queue.push(Queued {
                    order: sort_word(sort_key, ordinal, &light),
                    material: (sort_key, ordinal),
                    light,
                    span: first..first.saturating_add(surf.index_count as usize),
                });
            }
            Err(miss) => refusal = Some(miss.cause()),
        }
    }
    queue.sort_by_key(|run| run.order);

    for Queued { material, light, span, .. } in queue {
        match indices.get(span) {
            Some(slice) => plan.extend_with_run_indices(material.0, Some(material.1), light, slice),
            None => refusal = Some("range_out_of_mesh"),
        }
    }
    if budget.facet_sum > 0 && plan.draws.is_empty() && refusal.is_none() {
        refusal = Some("empty_range");
    }
    plan.bypass_cause = refusal;
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx::DecalMeshPatch;

    fn surf(mark_slot: u16, first_triangle_slot: u32, idx_count: u32) -> DecalMeshPatch {
        DecalMeshPatch {
            first_triangle_slot,
            index_count: idx_count,
            stain_cell: mark_slot,
            ..Default::default()
        }
    }

    fn census(indices: Vec<u16>, surfs: Vec<DecalMeshPatch>) -> DecalMeshTally {
        let mut census = DecalMeshTally {
            index_list: indices,
            surfs,
            ..Default::default()
        };
        census.budget.facet_sum = census.surfs.len() as _;
        census
    }

    /// Slot 1 and 2 are materials with different sort keys; slot 9 has none.
    fn bind(slot: u16) -> Result<(u8, u32), BindMiss> {
        match slot {
            1 => Ok((3, 10)),
            2 => Ok((5, 11)),
            3 => Err(BindMiss::NoOrdinal),
            _ => Err(BindMiss::NoMaterial),
        }
    }

    fn draw_spans(plan: &VisualStainMeshPlan) -> Vec<(u32, u32)> {
        plan.draws.iter().map(|d| (d.first_triangle_slot, d.index_count)).collect()
    }

    #[test]
    fn neighbouring_surfaces_of_one_material_share_a_draw() {
        let mut plan = VisualStainMeshPlan::default();
        let mesh = census((0..12).collect(), vec![surf(1, 0, 3), surf(1, 3, 3), surf(1, 6, 6)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(draw_spans(&plan), vec![(0, 12)]);
        assert_eq!(plan.bypass_cause, None);
        assert_eq!(plan.indices.len(), 12);
    }

    #[test]
    fn materials_are_drawn_in_sort_order_whatever_the_surface_order() {
        let mut plan = VisualStainMeshPlan::default();
        // The sort-key-5 surface comes first in the mesh, the key-3 one second.
        let mesh = census((0..6).collect(), vec![surf(2, 0, 3), surf(1, 3, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        let keys: Vec<u8> = plan
            .draws
            .iter()
            .map(|d| plan.mat_list[d.shader as usize].bin)
            .collect();
        assert_eq!(keys, vec![3, 5]);
        // The runs are appended in that order, so the key-3 indices come first.
        assert_eq!(plan.indices.as_slice(), &[3, 4, 5, 0, 1, 2]);
    }

    #[test]
    fn surfaces_without_indices_are_ignored() {
        let mut plan = VisualStainMeshPlan::default();
        let mesh = census(vec![0, 1, 2], vec![surf(1, 0, 0), surf(1, 0, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(draw_spans(&plan), vec![(0, 3)]);
        assert_eq!(plan.bypass_cause, None);
    }

    #[test]
    fn the_last_unbound_reason_is_kept() {
        let mut plan = VisualStainMeshPlan::default();
        let mesh = census((0..9).collect(), vec![surf(9, 0, 3), surf(3, 3, 3), surf(1, 6, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.bypass_cause, Some("no_ordinal"));
        assert_eq!(draw_spans(&plan), vec![(0, 3)]);
    }

    #[test]
    fn a_run_past_the_end_of_the_mesh_is_refused() {
        let mut plan = VisualStainMeshPlan::default();
        let mesh = census(vec![0, 1, 2], vec![surf(1, 0, 3), surf(2, 2, 3)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.bypass_cause, Some("range_out_of_mesh"));
        assert_eq!(draw_spans(&plan), vec![(0, 3)]);
    }

    #[test]
    fn surfaces_that_all_come_out_empty_are_reported() {
        let mut plan = VisualStainMeshPlan::default();
        let mesh = census(Vec::new(), vec![surf(1, 0, 0)]);
        build_mark_plan(mesh, &mut plan, bind);
        assert_eq!(plan.bypass_cause, Some("empty_range"));
        assert!(plan.draws.is_empty());
    }

    #[test]
    fn a_frame_without_surfaces_has_nothing_to_report() {
        let mut plan = VisualStainMeshPlan::default();
        build_mark_plan(census(vec![], vec![]), &mut plan, bind);
        assert_eq!(plan.bypass_cause, None);
    }
}
