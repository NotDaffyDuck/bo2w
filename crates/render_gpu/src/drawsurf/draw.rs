use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ExactColourDrawSet;

pub(crate) fn register_drawsurf_render(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        diag::warn!(
            World,
            "drawsurf: RenderApp missing — exact Colour execution cannot be extracted"
        );
        return;
    };

    render_app
        .init_resource::<super::gpu_resources::ExtractedRuntimeImageHandles>()
        .init_resource::<super::gpu_resources::RuntimeUploadedImageRegistry>()
        .add_systems(
            Render,
            super::gpu_resources::prepare_uploaded_image_registry
                .in_set(RenderSystems::PrepareResources),
        );
    render_app.add_systems(
        Render,
        super::scene_depth::prepare_scene_depth.in_set(RenderSystems::PrepareResources),
    );
    // bo2mp: the MW2 SM3 colour pass no longer runs; the BO2 passes still read
    // the frame the extract publishes.
    render_app
        .init_resource::<super::InstalledRenderWorld>()
        .init_resource::<super::PublishedRenderFrame>()
        .init_resource::<super::ExtractedPostFx>();
    super::sun_effects::register(app);
    super::hud_quads::register(app);
    super::model_lighting_tiles::register(app);
    super::geometry_diagnostic::register(app);
    super::guest_view::register(app);
    super::portal_view::register(app);
    super::ui3d::register(app);
}
