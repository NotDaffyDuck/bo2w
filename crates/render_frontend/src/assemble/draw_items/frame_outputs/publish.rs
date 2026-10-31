//! Hands the frame's finished products to the published ring.
//!
//! This runs once per frame after every product system. Each product is
//! swapped into its bank slot (the bank gives the old buffers back for the
//! next frame to reuse), the focused-model record is attached, and the bank
//! is made live.

use bevy::prelude::*;
use render_anim::DrawFocus;

use super::super::kept_lanes::StaticDrawTrack;
use super::bank::FrameRenderOutputs;
use super::cam_xf::CameraOutputs;
use super::spot::ConeOutput;
use super::daylight::DaylightOutput;
use crate::assemble::draw_items::frame_outputs::{FrameBuildFeed, FrameOutputKind};
use crate::prepare::scene::eye_parms::StagedStageEye;

/// The kinds whose draw counts add up to the per-frame draw counter.
const COUNTED: [FrameOutputKind; 5] = [
    FrameOutputKind::Colour,
    FrameOutputKind::Light,
    FrameOutputKind::Emissive,
    FrameOutputKind::SunDepth,
    FrameOutputKind::SpotLightShadowPass,
];

/// The lane name the layout mark carries for the focused draw.
const FOCUS_LANE: &str = "script_model";

fn total_draws(frame: &FrameRenderOutputs) -> usize {
    COUNTED
        .into_iter()
        .map(|kind| frame.product(kind).draw_sequence.len())
        .fold(0, usize::saturating_add)
}

pub(crate) fn publish_frame_outputs(
    inputs: Res<FrameBuildFeed>,
    retained: Res<StaticDrawTrack>,
    focus: Option<Res<DrawFocus>>,
    prepared: Option<Res<StagedStageEye>>,
    mut products: ResMut<FrameRenderOutputs>,
    mut camera: ResMut<CameraOutputs>,
    mut sun: ResMut<DaylightOutput>,
    mut spot: ResMut<ConeOutput>,
) {
    let (_, _, _, _, level_run_facets) = retained.alive_for(inputs.level_generation);
    products.frame_seq_no = inputs.frame_seq_no;
    products.start_fill();
    products.set_world_run_surfs(level_run_facets);

    let CameraOutputs {
        colour,
        emissive,
        light,
        distortion: warp_pass,
        ..
    } = camera.as_mut();
    products.adopt(&mut colour.product);
    products.adopt(&mut light.product);
    products.adopt(&mut emissive.product);
    products.adopt(warp_pass);
    products.adopt(&mut sun.slot.product);
    products.adopt(&mut spot.slot.product);

    let in_focus = focus.and_then(|focus| focus.frame.clone());
    products.set_focus(in_focus.clone());
    products.complete_fill();

    if let Some(focus_on) = in_focus {
        perf::draw_thread_layout_mark(
            products.frame_seq_no,
            inputs.level_generation.0,
            FOCUS_LANE,
            focus_on.owner_no,
            focus_on.model.as_deref(),
            focus_on.outcome,
            focus_on.object_tag,
            prepared.map(|view| view.eye.to_array()),
            focus_on.hidden_from_viewer,
            focus_on.illum_handle,
            focus_on.planned_facets,
        );
    }

    perf::Counter::TallyDrawCalls.emit(total_draws(&products) as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_bank_has_no_draws() {
        assert_eq!(total_draws(&FrameRenderOutputs::default()), 0);
    }

    #[test]
    fn only_the_listed_kinds_are_counted() {
        assert!(!COUNTED.contains(&FrameOutputKind::RefractionPass));
    }
}
