use bevy::prelude::*;
use bevy::render::RenderApp;

use crate::SubmitReadyGpu;

/// Installs the GPU side of the renderer into the app and its render sub-app.
pub struct DrawGpuPlugin;

impl Plugin for DrawGpuPlugin {
    fn build(&self, host: &mut App) {
        crate::surf_gpu::register_facet_draw_render(host);
        host.init_resource::<SubmitReadyGpu>();
        if let Some(render_sub) = host.get_sub_app_mut(RenderApp) {
            render_sub.init_resource::<SubmitReadyGpu>();
        }
    }
}
