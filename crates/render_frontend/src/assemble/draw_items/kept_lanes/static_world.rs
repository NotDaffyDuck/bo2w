use asset_world::CameraSpanKind;
use bevy::prelude::*;
use dpvs_t6::SortedDraw;
use render_frame::FacetSamplerInputs;

use crate::assemble::draw_items::kept_lanes::{LevelViewLane, HeldDrawItem, HeldDrawKind, catalog_item};
use crate::assemble::draw_items::list::FacetDrawList;
use crate::assemble::draw_items::material_live::LiveMaterialLedger;
use crate::assemble::draw_items::mesh_build::world::WorldGpuSubmission;
use crate::prepare::scene::world::LevelDrawItemKind;
use crate::prepare::scene::world::LevelStage;

/// Where world items come from: this frame's world draw list, the surfaces' sampler inputs,
/// and the scene (for brush model poses).
pub(super) struct WorldSource<'a> {
    pub(super) list: &'a FacetDrawList,
    pub(super) plan: Option<&'a WorldGpuSubmission>,
    pub(super) scene: Option<&'a LevelStage>,
}

impl WorldSource<'_> {
    fn samplers(&self, surf: u16) -> FacetSamplerInputs {
        self.plan
            .and_then(|plan| plan.facet_sampler_inputs.get(usize::from(surf)))
            .copied()
            .unwrap_or_default()
    }

    fn brush_model_pose(&self, surf: u16) -> Mat4 {
        let Some(cull) = self.scene.and_then(|scene| scene.culling.as_ref()) else {
            return Mat4::IDENTITY;
        };
        // Model 0 is the world itself; a surface belongs to the first brush model whose range holds it.
        let surf_ix = usize::from(surf);
        cull.solid_models
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, model)| {
                let first = usize::from(model.start_facet);
                (first..first + usize::from(model.facet_tally)).contains(&surf_ix)
            })
            .and_then(|(index, _)| cull.solid_model_level_from_local.get(index).copied())
            .unwrap_or(Mat4::IDENTITY)
    }
}

/// Appends one item per brush model surface and one per stretch of a BSP run whose surfaces
/// share sampler inputs. A run's surfaces go to `members`, and its item's `run_start_offset` points at
/// them there.
///
/// Runs are split because the draw key carries no lightmap or reflection probe: each surface
/// binds its own, so one item can only cover surfaces that bind the same ones.
pub(super) fn emit_world_items(
    source: &WorldSource,
    catalog: &LiveMaterialLedger,
    items: &mut Vec<HeldDrawItem>,
    members: &mut Vec<u16>,
) {
    for entry in &source.list.items {
        let rank = u32::from(SortedDraw::unpack_from_bits(entry.key).material_draw_rank_slot());
        let camera_range = match entry.kind {
            LevelDrawItemKind::Bsp(camera_range) => camera_range,
            LevelDrawItemKind::BModel => {
                let kind = HeldDrawKind::posed_face(
                    entry.facet_ix,
                    source.brush_model_pose(entry.facet_ix),
                );
                let samplers = source.samplers(entry.facet_ix);
                items.push(catalog_item(entry.key, rank, kind, samplers, catalog));
                continue;
            }
        };
        let first = entry.list_start as usize;
        let run = source
            .list
            .surfs
            .get(first..first + usize::from(entry.span_run))
            .unwrap_or_else(|| panic!("world draw run is outside its surface list"));
        for stretch in run.chunk_by(|&a, &b| source.samplers(a) == source.samplers(b)) {
            let kind = HeldDrawKind::map_geometry_kind(
                stretch[0],
                stretch.len() as u16,
                members.len() as u32,
                camera_lane(camera_range),
                entry.entry_first,
            );
            members.extend_from_slice(stretch);
            let samplers = source.samplers(stretch[0]);
            items.push(catalog_item(entry.key, rank, kind, samplers, catalog));
        }
    }
}

const fn camera_lane(range: CameraSpanKind) -> LevelViewLane {
    match range {
        CameraSpanKind::LitOpaque => LevelViewLane::LitOpaque,
        CameraSpanKind::LitTrans => LevelViewLane::LitTrans,
        CameraSpanKind::Decal => LevelViewLane::Decal,
        CameraSpanKind::Emissive => LevelViewLane::Emissive,
    }
}

/// Copies every world item's run into `table` in item order and points the item at its copy,
/// so the drawn list reads its runs front to back. Items of other kinds are left alone.
pub(super) fn lay_out_runs(items: &mut [HeldDrawItem], members: &[u16], table: &mut Vec<u16>) {
    table.clear();
    for draw_entry in items {
        let HeldDrawKind::World {
            surf,
            run,
            run_start_offset,
            world_tree_camera_kind,
            level_tree_run_first,
            model_to_world: local_to_world,
        } = draw_entry.kind
        else {
            continue;
        };
        let span_run = run.max(1);
        let copied_at = table.len() as u32;
        match span_run {
            1 => table.push(surf),
            _ => {
                let from = run_start_offset as usize;
                let run_members = members
                    .get(from..from + usize::from(span_run))
                    .unwrap_or_else(|| panic!("world draw run is outside its surface list"));
                table.extend_from_slice(run_members);
            }
        }
        draw_entry.kind = HeldDrawKind::World {
            surf,
            run: span_run,
            run_start_offset: copied_at,
            world_tree_camera_kind,
            level_tree_run_first,
            model_to_world: local_to_world,
        };
    }
}
