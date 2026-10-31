use bevy::prelude::*;

use crate::mesh_build::prop_mesh::{
    DetailRampArgs, PropGpuPlan, PropPlacement, prop_camera_detail,
};
use crate::prepare::scene::cull::prop_hide_dist_skips_slot;
use crate::prepare::scene::prop_geom_memo::LevelPropCache;
use crate::prepare::scene::prop_light_slots::LevelPropIllum;

/// One static model placement the camera can see, at the detail level it draws with.
///
/// Two frames that choose the same models in the same order can keep last frame's model
/// items, which is why every field that reaches an item is kept here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ModelChoice {
    pub(super) placement: u32,
    pub(super) mesh: u32,
    pub(super) lod: u8,
    pub(super) cache_slot_no: u16,
    pub(super) illum_handle: u32,
    pub(super) compact_illum: Option<[u8; 4]>,
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
    pub(super) fn new(eye: Option<Vec3>, ramp: DetailRampArgs) -> Self {
        Self {
            eye: eye.map(|at| at.to_array().map(f32::to_bits)),
            ramp: [ramp.mid_scale, ramp.mid_bias, ramp.prior_size].map(|v| v.map(f32::to_bits)),
        }
    }
}

/// What a placement needs from the frame to be chosen.
pub(super) struct ChoiceInputs<'a> {
    pub(super) vis: &'a [u8],
    pub(super) hide_dists: &'a [u16],
    pub(super) eye: Option<Vec3>,
    pub(super) ramp: DetailRampArgs,
    pub(super) lighting: Option<&'a LevelPropIllum>,
    pub(super) cache: Option<&'a LevelPropCache>,
}

/// Walks every placement in plan order. A placement is skipped when the visibility list
/// (when there is one) marks its slot hidden or when it is beyond its cull distance; the rest
/// count as queried, and those with something to draw become choices.
pub(super) fn choose_models(plan: &PropGpuPlan, inputs: &ChoiceInputs) -> ModelChoices {
    let mut choices = ModelChoices::default();
    for (index, pose_at) in plan.placements.iter().enumerate() {
        let slot = pose_at.illum_slot.unwrap_or(index);
        let hidden = !inputs.vis.is_empty() && inputs.vis.get(slot).is_none_or(|v| *v == 0);
        if hidden
            || prop_hide_dist_skips_slot(
                inputs.hide_dists,
                slot,
                pose_at.at_pos,
                inputs.eye,
                inputs.ramp.prior_size,
            )
        {
            continue;
        }
        choices.queried += 1;
        choices
            .list
            .extend(choose_one(plan, index, pose_at, slot, inputs));
    }
    choices
}

/// The choice for one placement, or `None` when it has nothing to draw: a lit placement
/// without a lighting handle, a mesh missing from the plan, no detail level for this view, an
/// empty detail level, or a slot too large for a 16-bit model index.
fn choose_one(
    plan: &PropGpuPlan,
    index: usize,
    pose_at: &PropPlacement,
    seat: usize,
    inputs: &ChoiceInputs,
) -> Option<ModelChoice> {
    let illum_handle = match pose_at.lit_list {
        true => {
            let illum_slot = pose_at.illum_slot?;
            let handle = inputs
                .lighting
                .and_then(|lighting| lighting.handle_list.get(illum_slot).copied())
                .unwrap_or(0);
            (handle != 0).then_some(u32::from(handle))?
        }
        false => 0,
    };
    let compact_illum = pose_at.compact_illum.or_else(|| {
        let illum_slot = pose_at.illum_slot?;
        inputs.lighting?.compact_illum_for_slot(illum_slot)
    });
    let mesh_h = plan.meshes.get(pose_at.model_slot)?;
    let lod = prop_camera_detail(
        mesh_h.lod,
        pose_at.at_pos,
        pose_at.size_k,
        inputs.eye,
        inputs.ramp,
    )?;
    let lod_ix = usize::from(lod);
    let has_surfaces = mesh_h
        .facets_by_detail
        .get(lod_ix)
        .is_some_and(|surfaces| !surfaces.is_empty());
    if !has_surfaces {
        return None;
    }
    u16::try_from(seat).ok()?;
    let cache_slot_no = pose_at
        .illum_slot
        .and_then(|illum_slot| inputs.cache?.cache_slot_no.get(illum_slot))
        .and_then(|per_lod| per_lod.get(usize::from(lod)).copied())
        .unwrap_or(0);
    Some(ModelChoice {
        placement: index as u32,
        mesh: pose_at.model_slot as u32,
        lod,
        cache_slot_no,
        illum_handle,
        compact_illum,
    })
}
