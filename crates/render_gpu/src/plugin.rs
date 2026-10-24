use bevy::prelude::*;

pub struct RenderGpuPlugin;

impl Plugin for RenderGpuPlugin {
    fn build(&self, app: &mut App) {
        crate::drawsurf::register_drawsurf_render(app);
        app.init_resource::<crate::GpuSubmitReady>();
        let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
            return;
        };
        render_app.init_resource::<crate::GpuSubmitReady>();
    }
}
