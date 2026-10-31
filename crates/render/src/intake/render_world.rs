//! What the render world copies out of the main world each frame, apart from
//! the geometry: the sealed colour frame, image handles, post effects and
//! model lighting tiles.

use bevy::prelude::*;
use bevy::render::{Extract, MainWorld};
use render_frontend::assemble::draw_items::dof::{BokehFrame, BloomTunables};
use render_frontend::assemble::draw_items::{ColourPaintMethod, GradeEye, LiveImageHandles};
use render_gpu::{ExtractedPostEffect, ExtractedLiveImageHandles, PublishedRenderFrame};

use super::seal::install_empty;

/// Hands the sealed frame across, or an empty one when none was sealed.
pub fn pull_precise_colour(
    mut commands: Commands,
    sealed: Extract<Option<Res<PublishedRenderFrame>>>,
) {
    match sealed.as_deref() {
        Some(sealed) => {
            commands.insert_resource(sealed.world().clone());
            commands.insert_resource(sealed.clone());
        }
        None => install_empty(&mut commands),
    }
}

pub fn pull_image_handles_to_render_world(
    mut commands: Commands,
    source: Extract<Option<Res<LiveImageHandles>>>,
    held: Option<ResMut<ExtractedLiveImageHandles>>,
) {
    let Some(source) = source.as_deref() else {
        commands.insert_resource(ExtractedLiveImageHandles::default());
        return;
    };
    let handles = source.clone();
    match held {
        Some(mut held) => held.handles = handles,
        None => commands.insert_resource(ExtractedLiveImageHandles { handles }),
    }
}

pub fn pull_screen_effects_to_render_world(
    focus: Extract<Res<BokehFrame>>,
    grade: Extract<Res<GradeEye>>,
    glow_dvars: Extract<Res<BloomTunables>>,
    draw_method: Extract<Res<ColourPaintMethod>>,
    mut out: ResMut<ExtractedPostEffect>,
) {
    out.vision = grade.current;
    let fullbright = !grade.script_forced && matches!(**draw_method, ColourPaintMethod::Unshaded);
    let dof = &focus.focus;
    out.frame = render_gpu::BokehFrame {
        dof_vals: render_gpu::FocusBlur {
            hand_model_start: dof.hand_model_start,
            hand_model_end: dof.hand_model_end,
            near_from: dof.close_begin,
            near_to: dof.close_stop,
            far_from: dof.distant_begin,
            far_to: dof.distant_stop,
            near_soft: dof.close_soften,
            far_soft: dof.distant_soften,
        },
        blur: grade.blur,
        grading: grade.grading,
        bias: focus.bias,
        stage_near: focus.stage_near,
        hand_model_near: focus.hand_model_near,
        glow: render_gpu::BloomFrame {
            glow_on: glow_dvars.enable || grade.script_forced,
            fullbright_on: fullbright,
            ..render_gpu::BloomFrame::from_vision(out.vision)
        },
    };
}

/// Moves the model lighting atlas tiles written since the last frame over to
/// the uploads the GPU side drains.
pub fn extract_model_lighting_tiles(
    mut main_world: ResMut<MainWorld>,
    mut uploads: ResMut<render_gpu::ModelLightingTileUploads>,
) {
    let Some(mut writes) = main_world.get_resource_mut::<render_scene::ModelLightingAtlasTileWrites>()
    else {
        return;
    };
    if writes.tiles.is_empty() {
        return;
    }
    uploads
        .tiles
        .extend(writes.tiles.drain(..).map(|tile| render_gpu::ModelLightingTileUpload {
            image: tile.image,
            origin: tile.origin,
            texels: tile.texels,
        }));
}
