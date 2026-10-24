use bevy::prelude::*;

use crate::assemble::drawsurf::tess::smodel::{
    LodRampArgs, SmodelGpuPlan, SmodelPlacement, smodel_camera_lod,
};
use crate::prepare::scene::cull::smodel_cull_dist_skips_slot;
use crate::prepare::scene::smodel_geom_cache::WorldStaticModelCache;
use crate::prepare::scene::smodel_lighting::WorldSmodelLighting;

/// One static model placement the camera can see, at the detail level it draws with.
///
/// Two frames that choose the same models in the same order can keep last frame's model
/// items, which is why every field that reaches an item is kept here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ModelChoice {
    pub(super) placement: u32,
    pub(super) mesh: u32,
    pub(super) lod: u8,
    pub(super) cache_index: u16,
    pub(super) lighting_handle: u32,
    pub(super) packed_lighting: Option<[u8; 4]>,
}

/// The chosen models, and how many placements got past visibility and distance culling.
#[derive(Debug, Default)]
pub(super) struct ModelChoices {
    pub(super) list: Vec<ModelChoice>,
    pub(super) queried: u32,
}

/// The view inputs that change which detail level a model draws at. Compared bit for bit, so
/// any camera move at all counts as a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewKey {
    eye: Option<[u32; 3]>,
    ramp: [Option<u32>; 3],
}

impl ViewKey {
    pub(super) fn new(eye: Option<Vec3>, ramp: LodRampArgs) -> Self {
        Self {
            eye: eye.map(|at| at.to_array().map(f32::to_bits)),
            ramp: [ramp.scale_mid, ramp.bias_mid, ramp.scale_last].map(|v| v.map(f32::to_bits)),
        }
    }
}

/// What a placement needs from the frame to be chosen.
pub(super) struct ChoiceInputs<'a> {
    pub(super) vis: &'a [u8],
    pub(super) cull_dists: &'a [u16],
    pub(super) eye: Option<Vec3>,
    pub(super) ramp: LodRampArgs,
    pub(super) lighting: Option<&'a WorldSmodelLighting>,
    pub(super) cache: Option<&'a WorldStaticModelCache>,
}

/// Walks every placement in plan order. A placement is skipped when the visibility list
/// (when there is one) marks its slot hidden or when it is beyond its cull distance; the rest
/// count as queried, and those with something to draw become choices.
pub(super) fn choose_models(plan: &SmodelGpuPlan, inputs: &ChoiceInputs) -> ModelChoices {
    let mut choices = ModelChoices::default();
    for (index, placement) in plan.placements.iter().enumerate() {
        let slot = placement.lighting_slot.unwrap_or(index);
        let hidden = !inputs.vis.is_empty() && inputs.vis.get(slot).is_none_or(|v| *v == 0);
        if hidden
            || smodel_cull_dist_skips_slot(
                inputs.cull_dists,
                slot,
                placement.origin,
                inputs.eye,
                inputs.ramp.scale_last,
            )
        {
            continue;
        }
        choices.queried += 1;
        choices
            .list
            .extend(choose_one(plan, index, placement, slot, inputs));
    }
    choices
}

/// The choice for one placement, or `None` when it has nothing to draw: a lit placement
/// without a lighting handle, a mesh missing from the plan, no detail level for this view, an
/// empty detail level, or a slot too large for a 16-bit model index.
fn choose_one(
    plan: &SmodelGpuPlan,
    index: usize,
    placement: &SmodelPlacement,
    slot: usize,
    inputs: &ChoiceInputs,
) -> Option<ModelChoice> {
    let lighting_handle = match placement.lit {
        true => {
            let lighting_slot = placement.lighting_slot?;
            let handle = inputs
                .lighting
                .and_then(|lighting| lighting.handles.get(lighting_slot).copied())
                .unwrap_or(0);
            (handle != 0).then_some(u32::from(handle))?
        }
        false => 0,
    };
    let packed_lighting = placement.packed_lighting.or_else(|| {
        let lighting_slot = placement.lighting_slot?;
        inputs.lighting?.packed_lighting_for_slot(lighting_slot)
    });
    let mesh = plan.meshes.get(placement.mesh)?;
    let lod = smodel_camera_lod(
        mesh.lod,
        placement.origin,
        placement.scale,
        inputs.eye,
        inputs.ramp,
    )?;
    let has_surfaces = mesh
        .surfaces_by_lod
        .get(usize::from(lod))
        .is_some_and(|surfaces| !surfaces.is_empty());
    if !has_surfaces {
        return None;
    }
    u16::try_from(slot).ok()?;
    let cache_index = placement
        .lighting_slot
        .and_then(|lighting_slot| inputs.cache?.cache_index.get(lighting_slot))
        .and_then(|per_lod| per_lod.get(usize::from(lod)).copied())
        .unwrap_or(0);
    Some(ModelChoice {
        placement: index as u32,
        mesh: placement.mesh as u32,
        lod,
        cache_index,
        lighting_handle,
        packed_lighting,
    })
}
