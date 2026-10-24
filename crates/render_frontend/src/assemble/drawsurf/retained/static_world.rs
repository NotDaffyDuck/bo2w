use asset_world::CameraRangeKind;
use bevy::prelude::*;
use dpvs_t6::GfxDrawSurf;
use render_frame::SurfaceSamplerInputs;

use super::{BspCameraLane, RetainedDrawItem, RetainedDrawKind, catalog_item};
use crate::assemble::drawsurf::list::DrawSurfList;
use crate::assemble::drawsurf::material_runtime::RuntimeMaterialCatalog;
use crate::assemble::drawsurf::tess::world::WorldDrawGpuPlan;
use crate::prepare::scene::world::{WorldDrawItemKind, WorldScene};

/// Where world items come from: this frame's world draw list, the surfaces' sampler inputs,
/// and the scene (for brush model poses).
pub(super) struct WorldSource<'a> {
    pub(super) list: &'a DrawSurfList,
    pub(super) plan: Option<&'a WorldDrawGpuPlan>,
    pub(super) scene: Option<&'a WorldScene>,
}

impl WorldSource<'_> {
    fn samplers(&self, surf: u16) -> SurfaceSamplerInputs {
        self.plan
            .and_then(|plan| plan.surface_sampler_inputs.get(usize::from(surf)))
            .copied()
            .unwrap_or_default()
    }

    fn brush_model_pose(&self, surf: u16) -> Mat4 {
        let Some(cull) = self.scene.and_then(|scene| scene.cull.as_ref()) else {
            return Mat4::IDENTITY;
        };
        // Model 0 is the world itself; a surface belongs to the first brush model whose range holds it.
        let surf = usize::from(surf);
        cull.brush_models
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, model)| {
                let first = usize::from(model.start_surf);
                (first..first + usize::from(model.surface_count)).contains(&surf)
            })
            .and_then(|(index, _)| cull.bmodel_world_from_local.get(index).copied())
            .unwrap_or(Mat4::IDENTITY)
    }
}

/// Appends one item per brush model surface and one per stretch of a BSP run whose surfaces
/// share sampler inputs. A run's surfaces go to `members`, and its item's `run_off` points at
/// them there.
///
/// Runs are split because the draw key carries no lightmap or reflection probe: each surface
/// binds its own, so one item can only cover surfaces that bind the same ones.
pub(super) fn emit_world_items(
    source: &WorldSource,
    catalog: &RuntimeMaterialCatalog,
    items: &mut Vec<RetainedDrawItem>,
    members: &mut Vec<u16>,
) {
    for entry in &source.list.items {
        let rank = u32::from(GfxDrawSurf::from_packed(entry.key).material_sorted_index());
        let camera_range = match entry.kind {
            WorldDrawItemKind::Bsp(camera_range) => camera_range,
            WorldDrawItemKind::BModel => {
                let kind = RetainedDrawKind::world_with_pose(
                    entry.surf,
                    source.brush_model_pose(entry.surf),
                );
                let samplers = source.samplers(entry.surf);
                items.push(catalog_item(entry.key, rank, kind, samplers, catalog));
                continue;
            }
        };
        let first = entry.list_start as usize;
        let run = source
            .list
            .surfs
            .get(first..first + usize::from(entry.run))
            .unwrap_or_else(|| panic!("world draw run is outside its surface list"));
        for stretch in run.chunk_by(|&a, &b| source.samplers(a) == source.samplers(b)) {
            let kind = RetainedDrawKind::bsp_world(
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

const fn camera_lane(range: CameraRangeKind) -> BspCameraLane {
    match range {
        CameraRangeKind::LitOpaque => BspCameraLane::LitOpaque,
        CameraRangeKind::LitTrans => BspCameraLane::LitTrans,
        CameraRangeKind::Decal => BspCameraLane::Decal,
        CameraRangeKind::Emissive => BspCameraLane::Emissive,
    }
}

/// Copies every world item's run into `table` in item order and points the item at its copy,
/// so the drawn list reads its runs front to back. Items of other kinds are left alone.
pub(super) fn lay_out_runs(items: &mut [RetainedDrawItem], members: &[u16], table: &mut Vec<u16>) {
    table.clear();
    for item in items {
        let RetainedDrawKind::World {
            surf,
            run,
            run_off,
            bsp_kind,
            bsp_run_first,
            world_from_local,
        } = item.kind
        else {
            continue;
        };
        let run = run.max(1);
        let copied_at = table.len() as u32;
        match run {
            1 => table.push(surf),
            _ => {
                let from = run_off as usize;
                let run_members = members
                    .get(from..from + usize::from(run))
                    .unwrap_or_else(|| panic!("world draw run is outside its surface list"));
                table.extend_from_slice(run_members);
            }
        }
        item.kind = RetainedDrawKind::World {
            surf,
            run,
            run_off: copied_at,
            bsp_kind,
            bsp_run_first,
            world_from_local,
        };
    }
}
