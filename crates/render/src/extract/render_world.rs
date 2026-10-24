//! What the render world copies out of the main world each frame, apart from
//! the geometry: the sealed colour frame, image handles, post effects and
//! model lighting tiles.

use bevy::prelude::*;
use bevy::render::{Extract, MainWorld};
use render_frontend::assemble::drawsurf::dof::{DofFrame, GlowDvars};
use render_frontend::assemble::drawsurf::{ColourDrawMethod, FilmVisionView, RuntimeImageHandles};
use render_gpu::{ExtractedPostFx, ExtractedRuntimeImageHandles, PublishedRenderFrame};

use super::seal::install_empty;

/// Hands the sealed frame across, or an empty one when none was sealed.
pub fn extract_exact_colour(
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

pub fn extract_image_handles(
    mut commands: Commands,
    source: Extract<Option<Res<RuntimeImageHandles>>>,
    held: Option<ResMut<ExtractedRuntimeImageHandles>>,
) {
    let Some(source) = source.as_deref() else {
        commands.insert_resource(ExtractedRuntimeImageHandles::default());
        return;
    };
    let handles = source.clone();
    match held {
        Some(mut held) => held.handles = handles,
        None => commands.insert_resource(ExtractedRuntimeImageHandles { handles }),
    }
}

pub fn extract_postfx(
    focus: Extract<Res<DofFrame>>,
    film: Extract<Res<FilmVisionView>>,
    glow_dvars: Extract<Res<GlowDvars>>,
    draw_method: Extract<Res<ColourDrawMethod>>,
    mut out: ResMut<ExtractedPostFx>,
) {
    out.vision = film.current;
    let fullbright = !film.script_forced && matches!(**draw_method, ColourDrawMethod::Fullbright);
    let dof = &focus.dof;
    out.frame = render_gpu::DofFrame {
        dof: render_gpu::DepthOfField {
            view_model_start: dof.view_model_start,
            view_model_end: dof.view_model_end,
            near_start: dof.near_start,
            near_end: dof.near_end,
            far_start: dof.far_start,
            far_end: dof.far_end,
            near_blur: dof.near_blur,
            far_blur: dof.far_blur,
        },
        blur: film.blur,
        grading: film.grading,
        bias: focus.bias,
        scene_near: focus.scene_near,
        view_model_near: focus.view_model_near,
        glow: render_gpu::GlowFrame {
            r_glow: glow_dvars.enable || film.script_forced,
            r_fullbright: fullbright,
            ..render_gpu::GlowFrame::from_vision(out.vision)
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
