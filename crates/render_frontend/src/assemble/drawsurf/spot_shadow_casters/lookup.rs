use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::super::retained::{RetainedDrawItem, RetainedDrawKind};
use super::super::tess::smodel::{LodRampArgs, SmodelGpuPlan, smodel_camera_lod};
use super::super::tess::xmodel::XModelSurfaceDraw;

/// The retained draws found by what they draw instead of by position: the
/// first item of each static model placement and surface, world surface and
/// animated model surface.
pub(super) struct RetainedLookup<'a> {
    items: &'a [RetainedDrawItem],
    static_models: HashMap<(u32, u32), usize>,
    world: HashMap<u16, usize>,
    animated: HashMap<(u32, u16), usize>,
}

impl<'a> RetainedLookup<'a> {
    pub(super) fn index(items: &'a [RetainedDrawItem]) -> Self {
        let mut found = Self {
            items,
            static_models: HashMap::new(),
            world: HashMap::new(),
            animated: HashMap::new(),
        };
        for (at, item) in items.iter().enumerate() {
            match item.kind {
                RetainedDrawKind::Smodel {
                    placement, surface, ..
                } => {
                    found.static_models.entry((placement, surface)).or_insert(at);
                }
                RetainedDrawKind::World { surf, .. } => {
                    found.world.entry(surf).or_insert(at);
                }
                RetainedDrawKind::XModel { surface, .. } => {
                    let object = dpvs_t6::GfxDrawSurf::from_packed(item.key).object_id();
                    found.animated.entry((surface, object)).or_insert(at);
                }
                _ => {}
            }
        }
        found
    }

    pub(super) fn has_animated(&self) -> bool {
        !self.animated.is_empty()
    }

    pub(super) fn animated(&self, draw: &XModelSurfaceDraw) -> Option<RetainedDrawItem> {
        let at = self.animated.get(&(draw.surface, draw.object_id))?;
        Some(self.items[*at])
    }

    pub(super) fn world_surfaces(&self, surfaces: &[u16]) -> Vec<RetainedDrawItem> {
        surfaces
            .iter()
            .filter_map(|surf| self.world.get(surf))
            .map(|&at| self.items[at])
            .collect()
    }

    /// The static model draws for these placements at the level of detail the
    /// eye would see them at.
    pub(super) fn static_models_at_lod(
        &self,
        placements: &[u16],
        plan: Option<&SmodelGpuPlan>,
        eye: Vec3,
        ramp: LodRampArgs,
    ) -> Vec<RetainedDrawItem> {
        let Some(plan) = plan else { return Vec::new() };
        let mut found = Vec::new();
        for &placement_id in placements {
            let Some(placement) = plan.placements.get(usize::from(placement_id)) else {
                continue;
            };
            let Some(mesh) = plan.meshes.get(placement.mesh) else {
                continue;
            };
            let Some(lod) =
                smodel_camera_lod(mesh.lod, placement.origin, placement.scale, Some(eye), ramp)
            else {
                continue;
            };
            let Some(surfaces) = mesh.surfaces_by_lod.get(usize::from(lod)) else {
                continue;
            };
            for &(surface, _) in surfaces.iter() {
                if let Some(&at) = self
                    .static_models
                    .get(&(u32::from(placement_id), surface))
                {
                    found.push(self.items[at]);
                }
            }
        }
        found
    }
}
