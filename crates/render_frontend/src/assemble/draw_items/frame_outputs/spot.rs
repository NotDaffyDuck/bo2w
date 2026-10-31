//! The spot-light shadow caster list.
//!
//! Spot shadows are planned per light, not per cascade, so the list has no
//! near/far split. Two systems run each frame:
//!
//! 1. [`bake_cone_shadow_casters`] asks the caster planner which lights get a
//!    shadow map and what casts into it, and publishes the light indices and
//!    receiver table the lit passes read;
//! 2. [`execute_cone_output`] collapses the planned casters into the
//!    spot-shadow product and hands the per-light slots to the product.
//!
//! The collapse is replayed rather than recomputed for as long as the
//! [`SpotStamp`] is unchanged.

use std::sync::Arc;

use bevy::prelude::*;
use render_frame::{FrameOutputKind, CompactFrontendLists, OutputTarget};

use super::super::kept_lanes::StaticDrawTrack;
use super::super::material_setup::MethodType;
use super::super::spot_casters::fill_cone_shadow_caster_plan;
use super::super::mesh_build::world::WorldGpuSubmission;
use super::super::{
    DAYLIGHT_CASTER_TECH, PropGpuPlan, ConeShadowCasterPlan, ConeShadowMapLights, FigureDrawPlan,
};
use super::bank::SPOT_MISSING;
use super::seat::{Build, ListSlot};
use super::stamps::{SpotStamp, caster_membership};
use super::technique_ix::TechRule;
use crate::assemble::draw_items::frame_outputs::{FrameBuildFeed, MaterialFrameFeed, MaterialEpoch};
use crate::prepare::scene::visual_stage::{HostVisualStage, ConeShadowStageOccupancy};
use crate::prepare::scene::prop_geom_memo::DetailRampTunable;
use crate::prepare::scene::eye_parms::StagedStageEye;
use crate::prepare::scene::eye_parms::SmEnableDvar;
use crate::prepare::scene::eye_parms::SmSunEnableDvar;
use crate::prepare::scene::world::LevelStage;

/// The spot-shadow product and the bookkeeping that lets its per-light
/// slots survive from one frame to the next.
#[derive(Resource, Debug)]
pub struct ConeOutput {
    pub(super) slot: ListSlot,
    last: Option<SpotStamp>,
    prior_bundle_id: Option<u64>,
    last_compact: Vec<Arc<CompactFrontendLists>>,
}

impl Default for ConeOutput {
    fn default() -> Self {
        Self {
            slot: ListSlot::new(FrameOutputKind::SpotLightShadowPass),
            last: None,
            prior_bundle_id: None,
            last_compact: Vec::new(),
        }
    }
}

pub(crate) fn bake_cone_shadow_casters(
    inputs: Res<FrameBuildFeed>,
    retained: Res<StaticDrawTrack>,
    stage: Option<Res<LevelStage>>,
    level_plan: Option<Res<WorldGpuSubmission>>,
    figure_plan: Option<Res<FigureDrawPlan>>,
    smodel: (Option<Res<PropGpuPlan>>, Res<DetailRampTunable>),
    occupancy: Res<ConeShadowStageOccupancy>,
    host_gfx: Option<Res<HostVisualStage>>,
    ready_set: Option<Res<StagedStageEye>>,
    switches: (Res<SmEnableDvar>, Res<SmSunEnableDvar>),
    mut casters: ResMut<ConeShadowCasterPlan>,
    mut lights: ResMut<ConeShadowMapLights>,
    mut frame: ResMut<MaterialFrameFeed>,
) {
    let (prop_plan, detail_ramp) = smodel;
    let (cone_switch, sun_switch) = switches;
    let Some(level) = stage.as_deref() else {
        casters.wipe_frame();
        lights.0.clear();
        frame.cone_receivers.clear();
        return;
    };

    let (static_items, _, _, _, level_run_facets) = retained.alive_for(inputs.level_generation);
    let level_ranges = level_plan.as_ref().map_or(&[][..], |plan| plan.facet_ranges());
    let level_vertices = level_plan
        .as_ref()
        .map_or(0, |plan| plan.decoded_vertices().len() as u32);
    let prop_ranges = prop_plan.as_ref().map_or(&[][..], |plan| plan.facet_ranges());

    fill_cone_shadow_caster_plan(
        &mut casters,
        &occupancy,
        host_gfx.as_ref().map(|gfx| &gfx.scene),
        level,
        ready_set.as_deref(),
        figure_plan.as_deref(),
        static_items,
        level_run_facets,
        level_ranges,
        level_vertices,
        prop_ranges,
        prop_plan.as_deref(),
        detail_ramp.args(),
        inputs.frame_seq_no as u32,
        cone_switch.enabled,
        sun_switch.enabled,
    );
    lights.0 = casters.shadow_light_slots();
    frame.cone_receivers = casters.receivers_by_light_index();
}

/// True when at least one planned light has a packed list the spot pass can
/// run (an empty plan has nothing to draw).
fn any_light_ready(packed: &[CompactFrontendLists]) -> bool {
    packed
        .iter()
        .any(|lists| lighting_t6::cone_lists_ready(lists.work_entry_tally()))
}

pub(crate) fn execute_cone_output(
    inputs: Res<FrameBuildFeed>,
    generation: Res<MaterialEpoch>,
    retained: Res<StaticDrawTrack>,
    mut casters: ResMut<ConeShadowCasterPlan>,
    mut holder: ResMut<ConeOutput>,
) {
    let ConeOutput {
        slot,
        last,
        prior_bundle_id,
        last_compact,
    } = holder.as_mut();
    slot.begin(SPOT_MISSING);
    if !any_light_ready(&casters.packed) {
        return;
    }

    slot.product.draw_sequence = std::mem::take(&mut casters.items);
    let stamp = SpotStamp {
        membership: caster_membership(&slot.product.draw_sequence, 0),
        catalog: inputs.ledger_generation.0,
        slot_n: casters.packed.len(),
    };
    let build = Build {
        revision_stamp: retained.revision_stamp,
        digest: stamp.digest(),
        catalog: &generation.catalog,
        prepared: &generation.prepared,
    };
    let tech = MethodType(DAYLIGHT_CASTER_TECH);
    let target = OutputTarget::SpotLightShadowAtlas;

    if *last == Some(stamp) && slot.memo.replay(&mut slot.product) {
        slot.ready(&build, tech, target);
    } else {
        slot.collapse(&build, &TechRule::Single(tech), tech, target, false);
        *last = Some(stamp);
    }
    slot.product.spot_light_frame_slots =
        casters.take_frame_slots(slot.memo.slot_of_input(), prior_bundle_id, last_compact);
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
        let owner = ConeOutput::default();
        assert!(owner.last.is_none());
        assert!(owner.last_compact.is_empty());
        assert!(owner.slot.product.draw_sequence.is_empty());
    }
}
