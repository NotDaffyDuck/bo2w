//! The spot-light shadow caster list.
//!
//! Spot shadows are planned per light, not per cascade, so the list has no
//! near/far split. Two systems run each frame:
//!
//! 1. [`bake_spot_shadow_casters`] asks the caster planner which lights get a
//!    shadow map and what casts into it, and publishes the light indices and
//!    receiver table the lit passes read;
//! 2. [`execute_spot_product`] collapses the planned casters into the
//!    spot-shadow product and hands the per-light slots to the product.
//!
//! The collapse is replayed rather than recomputed for as long as the
//! [`SpotStamp`] is unchanged.

use std::sync::Arc;

use bevy::prelude::*;
use render_frame::{FrameProductKind, PackedFrontendLists, ProductTarget};

use super::super::retained::StaticDrawLane;
use super::super::setup::TechType;
use super::super::spot_shadow_casters::fill_spot_shadow_caster_plan;
use super::super::tess::world::WorldDrawGpuPlan;
use super::super::{
    SUN_SHADOW_CASTER_TECH, SmodelGpuPlan, SpotShadowCasterPlan, SpotShadowMapLights, XModelDrawPlan,
};
use super::bank::SPOT_MISSING;
use super::slot::{Build, ListSlot};
use super::stamps::{SpotStamp, caster_membership};
use super::tech::TechRule;
use super::{FrameAssemblyInputs, MaterialFrameInputs, MaterialGeneration};
use crate::prepare::scene::gfx_scene::{HostGfxScene, SpotShadowSceneOccupancy};
use crate::prepare::scene::smodel_geom_cache::LodRampDvar;
use crate::prepare::scene::view_parms::{PreparedSceneView, SmEnableDvar, SmSunEnableDvar};
use crate::prepare::scene::world::WorldScene;

/// The spot-shadow product and the bookkeeping that lets its per-light
/// slots survive from one frame to the next.
#[derive(Resource, Debug)]
pub struct SpotProduct {
    pub(super) slot: ListSlot,
    last: Option<SpotStamp>,
    last_pack_id: Option<u64>,
    last_packed: Vec<Arc<PackedFrontendLists>>,
}

impl Default for SpotProduct {
    fn default() -> Self {
        Self {
            slot: ListSlot::new(FrameProductKind::SpotShadow),
            last: None,
            last_pack_id: None,
            last_packed: Vec::new(),
        }
    }
}

pub(crate) fn bake_spot_shadow_casters(
    inputs: Res<FrameAssemblyInputs>,
    retained: Res<StaticDrawLane>,
    scene: Option<Res<WorldScene>>,
    world_plan: Option<Res<WorldDrawGpuPlan>>,
    xmodel_plan: Option<Res<XModelDrawPlan>>,
    smodel: (Option<Res<SmodelGpuPlan>>, Res<LodRampDvar>),
    occupancy: Res<SpotShadowSceneOccupancy>,
    host_gfx: Option<Res<HostGfxScene>>,
    prepared: Option<Res<PreparedSceneView>>,
    switches: (Res<SmEnableDvar>, Res<SmSunEnableDvar>),
    mut casters: ResMut<SpotShadowCasterPlan>,
    mut lights: ResMut<SpotShadowMapLights>,
    mut frame: ResMut<MaterialFrameInputs>,
) {
    let (smodel_plan, lod_ramp) = smodel;
    let (sm_enable, sm_sun_enable) = switches;
    let Some(world) = scene.as_deref() else {
        casters.clear_frame();
        lights.0.clear();
        frame.spot_receivers.clear();
        return;
    };

    let (static_items, _, _, _, world_run_surfs) = retained.live_for(inputs.world_generation);
    let world_ranges = world_plan.as_ref().map_or(&[][..], |plan| plan.surface_ranges());
    let world_verts = world_plan
        .as_ref()
        .map_or(0, |plan| plan.decoded_vertices().len() as u32);
    let smodel_ranges = smodel_plan.as_ref().map_or(&[][..], |plan| plan.surface_ranges());

    fill_spot_shadow_caster_plan(
        &mut casters,
        &occupancy,
        host_gfx.as_ref().map(|gfx| &gfx.scene),
        world,
        prepared.as_deref(),
        xmodel_plan.as_deref(),
        static_items,
        world_run_surfs,
        world_ranges,
        world_verts,
        smodel_ranges,
        smodel_plan.as_deref(),
        lod_ramp.args(),
        inputs.frame_id as u32,
        sm_enable.enabled,
        sm_sun_enable.enabled,
    );
    lights.0 = casters.shadowed_light_indices();
    frame.spot_receivers = casters.receivers_by_light_index();
}

/// True when at least one planned light has a packed list the spot pass can
/// run (an empty plan has nothing to draw).
fn any_light_ready(packed: &[PackedFrontendLists]) -> bool {
    packed
        .iter()
        .any(|lists| lighting_t6::spot_shadow_packed_lists_ready(lists.work_entry_n()))
}

pub(crate) fn execute_spot_product(
    inputs: Res<FrameAssemblyInputs>,
    generation: Res<MaterialGeneration>,
    retained: Res<StaticDrawLane>,
    mut casters: ResMut<SpotShadowCasterPlan>,
    mut owner: ResMut<SpotProduct>,
) {
    let SpotProduct {
        slot,
        last,
        last_pack_id,
        last_packed,
    } = owner.as_mut();
    slot.begin(SPOT_MISSING);
    if !any_light_ready(&casters.packed) {
        return;
    }

    slot.product.ordered_draws = std::mem::take(&mut casters.items);
    let stamp = SpotStamp {
        membership: caster_membership(&slot.product.ordered_draws, 0),
        catalog: inputs.catalog_generation.0,
        slot_n: casters.packed.len(),
    };
    let build = Build {
        generation_id: retained.generation_id,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let tech = TechType(SUN_SHADOW_CASTER_TECH);
    let target = ProductTarget::SpotShadowMaps;

    if *last == Some(stamp) && slot.memo.replay(&mut slot.product) {
        slot.ready(&build, tech, target);
    } else {
        slot.collapse(&build, &TechRule::Single(tech), tech, target, false);
        *last = Some(stamp);
    }
    slot.product.spot_slots =
        casters.adopt_frame_slots(slot.memo.slot_of_input(), last_pack_id, last_packed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_plan_has_no_ready_light() {
        assert!(!any_light_ready(&[]));
    }

    #[test]
    fn a_new_product_waits_for_casters() {
        let owner = SpotProduct::default();
        assert!(owner.last.is_none());
        assert!(owner.last_packed.is_empty());
        assert!(owner.slot.product.ordered_draws.is_empty());
    }
}
