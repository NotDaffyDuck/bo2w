use super::SunGpu;
use super::pipelines::Stage;
use super::uniforms::SunUniforms;
use crate::drawsurf::PublishedRenderFrame;
use crate::drawsurf::gpu_resources::RuntimeUploadedImageRegistry;
use crate::drawsurf::portal_view::PortalCamera;
use crate::drawsurf::scene_depth::SceneDepthTexture;
use bevy::prelude::*;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, CommandEncoder, LoadOp, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, StoreOp, TextureFormat,
    TextureView,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};

/// Builds the one bind group layout every pass uses; only the three sampled images change.
struct Binder<'a> {
    gpu: &'a SunGpu,
    cache: &'a PipelineCache,
    device: &'a RenderDevice,
    depth: &'a TextureView,
}

impl Binder<'_> {
    fn group(&self, history: &TextureView, sprite: &TextureView, scene: &TextureView) -> BindGroup {
        self.device.create_bind_group(
            Some("bo2_sun_effects"),
            &self.cache.get_bind_group_layout(&self.gpu.layout),
            &BindGroupEntries::sequential((
                self.gpu.uniforms.as_entire_buffer_binding(),
                self.depth,
                history,
                sprite,
                &self.gpu.sampler,
                scene,
            )),
        )
    }
}

fn uploaded_view(uploaded: &RuntimeUploadedImageRegistry, image: Option<u32>) -> Option<&TextureView> {
    let entry = uploaded.material_images.get(image? as usize)?.as_ref()?;
    entry.as_ref().ok().map(|img| &img.view)
}

fn cleared(view: &TextureView) -> RenderPassColorAttachment<'_> {
    RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: Operations {
            load: LoadOp::Clear(LinearRgba::BLACK.into()),
            store: StoreOp::Store,
        },
        depth_slice: None,
    }
}

fn run_pass(
    encoder: &mut CommandEncoder,
    label: &'static str,
    target: RenderPassColorAttachment,
    pipeline: &RenderPipeline,
    bind: &BindGroup,
    vertices: u32,
) {
    let attachments = [Some(target)];
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..vertices, 0..1);
}

pub(super) fn draw_sun(
    view: ViewQuery<(&ViewTarget, &ExtractedView, &SceneDepthTexture), Without<PortalCamera>>,
    frame: Res<PublishedRenderFrame>,
    gpu: Option<ResMut<SunGpu>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    uploaded: Res<RuntimeUploadedImageRegistry>,
    mut context: RenderContext,
) {
    let Some(mut gpu) = gpu else {
        return;
    };
    let (Some(def), Some(sun)) = (frame.world().sun_effects, frame.sun_effects) else {
        return;
    };
    let ids = gpu.pipelines;
    let pipeline = |stage: Stage| ids[stage.slot()].and_then(|id| cache.get_render_pipeline(id));
    let Some(history_pipe) = pipeline(Stage::History) else {
        return;
    };
    let (target, extracted, depth) = view.into_inner();
    if extracted.viewport.z == 0 || extracted.viewport.w == 0 {
        return;
    }

    let written = gpu.write;
    let previous = 1 - written;
    queue.write_buffer(
        &gpu.uniforms,
        0,
        bytemuck::bytes_of(&SunUniforms::from_frame(&sun)),
    );
    let binder = Binder {
        gpu: &gpu,
        cache: &cache,
        device: &device,
        depth: depth.view(),
    };
    let white = &gpu.white.view;
    let encoder = context.command_encoder();

    // Update the visibility history first; every later pass reads what this one wrote.
    let bind = binder.group(&gpu.history[previous].view, white, white);
    run_pass(
        encoder,
        "bo2_sun_history",
        cleared(&gpu.history[written].view),
        history_pipe,
        &bind,
        3,
    );

    let current = &gpu.history[written].view;
    let in_front = sun.view_dot > 0.0 && !sun.behind_camera;
    let colour_ok = target.main_texture_format() == TextureFormat::Rgba8Unorm;
    if colour_ok {
        let overlays = [
            (
                Stage::Sprite,
                "bo2_sun_sprite",
                def.sprite_image,
                sun.sprite_size > 0.0,
            ),
            (
                Stage::Flare,
                "bo2_sun_flare",
                def.flare_image,
                sun.flare_alpha > 1e-4 && sun.flare_size > 0.0,
            ),
        ];
        for (stage, label, image, sized) in overlays {
            let (Some(pipe), Some(sprite)) = (pipeline(stage), uploaded_view(&uploaded, image))
            else {
                continue;
            };
            if !(in_front && sized) {
                continue;
            }
            let bind = binder.group(current, sprite, white);
            run_pass(
                encoder,
                label,
                target.get_unsampled_color_attachment(),
                pipe,
                &bind,
                6,
            );
        }

        let grading = def.blind_max_darken > 0.0 || def.glare_max_lighten > 0.0;
        if grading && let Some(pipe) = pipeline(Stage::Grade) {
            let post = target.post_process_write();
            let bind = binder.group(current, white, post.source);
            run_pass(
                encoder,
                "bo2_sun_blind",
                cleared(post.destination),
                pipe,
                &bind,
                3,
            );
        }
    }
    gpu.write = previous;
}
