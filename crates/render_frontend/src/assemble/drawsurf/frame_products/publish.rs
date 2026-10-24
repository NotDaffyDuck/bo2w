//! Hands the frame's finished products to the published ring.
//!
//! This runs once per frame after every product system. Each product is
//! swapped into its bank slot (the bank gives the old buffers back for the
//! next frame to reuse), the focused-model record is attached, and the bank
//! is made live.

use bevy::prelude::*;
use render_anim::RenderFocus;

use super::super::retained::StaticDrawLane;
use super::bank::RenderFrameProducts;
use super::camera::CameraProducts;
use super::spot::SpotProduct;
use super::sun::SunProduct;
use super::{FrameAssemblyInputs, FrameProductKind};
use crate::prepare::scene::view_parms::PreparedSceneView;

/// The kinds whose draw counts add up to the per-frame draw counter.
const COUNTED: [FrameProductKind; 5] = [
    FrameProductKind::Colour,
    FrameProductKind::Light,
    FrameProductKind::Emissive,
    FrameProductKind::SunShadow,
    FrameProductKind::SpotShadow,
];

fn total_draws(frame: &RenderFrameProducts) -> usize {
    COUNTED
        .into_iter()
        .map(|kind| frame.product(kind).ordered_draws.len())
        .fold(0, usize::saturating_add)
}

pub(crate) fn publish_frame_products(
    inputs: Res<FrameAssemblyInputs>,
    retained: Res<StaticDrawLane>,
    focus: Option<Res<RenderFocus>>,
    prepared: Option<Res<PreparedSceneView>>,
    mut frame: ResMut<RenderFrameProducts>,
    mut camera: ResMut<CameraProducts>,
    mut sun: ResMut<SunProduct>,
    mut spot: ResMut<SpotProduct>,
) {
    let (_, _, _, _, world_run_surfs) = retained.live_for(inputs.world_generation);
    frame.frame_id = inputs.frame_id;
    frame.begin_fill();
    frame.set_world_run_surfs(world_run_surfs);

    let CameraProducts {
        colour,
        emissive,
        light,
        distortion,
        ..
    } = camera.as_mut();
    frame.adopt(&mut colour.product);
    frame.adopt(&mut light.product);
    frame.adopt(&mut emissive.product);
    frame.adopt(distortion);
    frame.adopt(&mut sun.slot.product);
    frame.adopt(&mut spot.slot.product);

    let focused = focus.and_then(|focus| focus.frame.clone());
    frame.set_focus(focused.clone());
    frame.finish_fill();

    if let Some(focused) = focused {
        perf::render_owner_plan(
            frame.frame_id,
            inputs.world_generation.0,
            "script_model",
            focused.owner_id,
            focused.model.as_deref(),
            focused.outcome,
            focused.object_id,
            prepared.map(|view| view.eye.to_array()),
            focused.camera_hidden,
            focused.lighting_handle,
            focused.planned_surfaces,
        );
    }

    perf::Counter::CounterDraws.emit(total_draws(&frame) as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_bank_has_no_draws() {
        assert_eq!(total_draws(&RenderFrameProducts::default()), 0);
    }

    #[test]
    fn only_the_listed_kinds_are_counted() {
        assert!(!COUNTED.contains(&FrameProductKind::Distortion));
    }
}
