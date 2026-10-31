use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::super::kept_lanes::{HeldDrawItem, HeldDrawKind};
use super::super::mesh_build::prop_mesh::{DetailRampArgs, PropGpuPlan, prop_camera_detail};
use super::super::mesh_build::figure_mesh::FigureFacetDraw;

/// The retained draws found by what they draw instead of by position: the
/// first item of each static model placement and surface, world surface and
/// animated model surface.
pub(super) struct RetainedLookup<'a> {
    items: &'a [HeldDrawItem],
    prop_set: HashMap<(u32, u32), usize>,
    world: HashMap<u16, usize>,
    animated: HashMap<(u32, u16), usize>,
}

impl<'a> RetainedLookup<'a> {
    pub(super) fn index(items: &'a [HeldDrawItem]) -> Self {
        let mut found = Self {
            items,
            prop_set: HashMap::new(),
            world: HashMap::new(),
            animated: HashMap::new(),
        };
        for (at, draw_entry) in items.iter().enumerate() {
            match draw_entry.kind {
                HeldDrawKind::Smodel {
                    placement: pose_at, surface, ..
                } => {
                    found.prop_set.entry((pose_at, surface)).or_insert(at);
                }
                HeldDrawKind::World { surf, .. } => {
                    found.world.entry(surf).or_insert(at);
                }
                HeldDrawKind::XModel { surface: facet, .. } => {
                    let object = dpvs_t6::SortedDraw::unpack_from_bits(draw_entry.key).object_tag();
                    found.animated.entry((facet, object)).or_insert(at);
                }
                _ => {}
            }
        }
        found
    }

    pub(super) fn has_animated(&self) -> bool {
        !self.animated.is_empty()
    }

    pub(super) fn animated(&self, draw: &FigureFacetDraw) -> Option<HeldDrawItem> {
        let at = self.animated.get(&(draw.surface, draw.object_tag))?;
        Some(self.items[*at])
    }

    pub(super) fn world_surfaces(&self, surfaces: &[u16]) -> Vec<HeldDrawItem> {
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
        plan: Option<&PropGpuPlan>,
        eye: Vec3,
        ramp: DetailRampArgs,
    ) -> Vec<HeldDrawItem> {
        let Some(plan) = plan else { return Vec::new() };
        let mut found = Vec::new();
        for &placement_id in placements {
            let Some(pose_at) = plan.placements.get(usize::from(placement_id)) else {
                continue;
            };
            let Some(mesh_h) = plan.meshes.get(pose_at.model_slot) else {
                continue;
            };
            let Some(detail) =
                prop_camera_detail(mesh_h.lod, pose_at.at_pos, pose_at.size_k, Some(eye), ramp)
            else {
                continue;
            };
            let Some(surfaces) = mesh_h.facets_by_detail.get(usize::from(detail)) else {
                continue;
            };
            for &(facet, _) in surfaces.iter() {
                if let Some(&at) = self
                    .prop_set
                    .get(&(u32::from(placement_id), facet))
                {
                    found.push(self.items[at]);
                }
            }
        }
        found
    }
}
