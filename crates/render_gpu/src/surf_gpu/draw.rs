use bevy::prelude::*;
use bevy::render::{Render, RenderApp, RenderSystems};

use super::gpu_bufs::{
    ExtractedLiveImageHandles, LiveUploadedImageRegistry, refresh_uploaded_images,
};
use super::scene_depth::prepare_scene_depth;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PreciseColourDrawSet;

/// Sets up the render-world side of drawsurf: the resources the frame publishes into, the
/// per-frame upload systems, and every pass module's own registration.
pub(crate) fn register_facet_draw_render(host: &mut App) {
    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        diag::warn!(
            World,
            "drawsurf: no RenderApp, so nothing can be extracted for the exact colour pass"
        );
        return;
    };

    render_sub
        .init_resource::<ExtractedLiveImageHandles>()
        .init_resource::<LiveUploadedImageRegistry>()
        .init_resource::<super::InstalledRenderWorld>()
        .init_resource::<super::PublishedRenderFrame>()
        .init_resource::<super::ExtractedPostEffect>()
        .add_systems(
            Render,
            (refresh_uploaded_images, prepare_scene_depth)
                .in_set(RenderSystems::PrepareResources),
        );

    // The old colour pass is gone; the BO2 passes read what the extract publishes.
    super::sun_effects::register(host);
    super::hud_quads::register(host);
    super::model_lighting_tiles::register(host);
    super::scene_geo::register(host);
    super::guest_view::register(host);
    super::portal_view::register(host);
    super::ui3d::register(host);
}
