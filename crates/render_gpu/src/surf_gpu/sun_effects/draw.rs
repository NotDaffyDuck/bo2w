use super::SunGpu;
use super::pipelines::Stage;
use super::uniforms::SunUniforms;
use crate::surf_gpu::PublishedRenderFrame;
use crate::surf_gpu::gpu_bufs::LiveUploadedImageRegistry;
use crate::surf_gpu::portal_view::PortalCamera;
use crate::surf_gpu::scene_depth::SceneDepthTexture;
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
    pipelines: &'a PipelineCache,
    render_device: &'a RenderDevice,
    depth: &'a TextureView,
}

impl Binder<'_> {
    fn band(&self, history: &TextureView, sprite: &TextureView, scene: &TextureView) -> BindGroup {
        let Self { gpu, pipelines, render_device, depth } = *self;
        // Binding order: uniforms, scene depth, history, sprite, sampler, scene colour.
        let entries = BindGroupEntries::sequential((gpu.uniforms.as_entire_buffer_binding(), depth, history, sprite, &gpu.filter_samp, scene));
        let layout = pipelines.get_bind_group_layout(&gpu.layout);
        render_device.create_bind_group(Some("bo2_sun_effects"), &layout, &entries)
    }
}

fn uploaded_view(uploaded: &LiveUploadedImageRegistry, image: Option<u32>) -> Option<&TextureView> {
    uploaded.material_view(image?).map(|uploaded| &uploaded.tex_view)
}

fn cleared(cam_view: &TextureView) -> RenderPassColorAttachment<'_> {
    let ops = Operations { store: StoreOp::Store, load: LoadOp::Clear(LinearRgba::BLACK.into()) };
    RenderPassColorAttachment { view: cam_view, ops, resolve_target: None, depth_slice: None }
}

/// One instance of `vertices` vertices into one colour target, with no depth attachment.
fn run_pass(commands: &mut CommandEncoder, pass_name: &'static str, into: RenderPassColorAttachment, pipeline: &RenderPipeline, band: &BindGroup, vtx_list: u32) {
    let color_attachments = [Some(into)];
    let descriptor = RenderPassDescriptor { label: Some(pass_name), color_attachments: &color_attachments, ..Default::default() };
    let mut rp = commands.begin_render_pass(&descriptor);
    rp.set_pipeline(pipeline);
    rp.set_bind_group(0, &**band, &[]);
    rp.draw(0..vtx_list, 0..1);
}

pub(super) fn draw_sun(
    cam_view: ViewQuery<(&ViewTarget, &ExtractedView, &SceneDepthTexture), Without<PortalCamera>>,
    frame: Res<PublishedRenderFrame>,
    gpu: Option<ResMut<SunGpu>>,
    pipelines: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    uploaded: Res<LiveUploadedImageRegistry>,
    mut render_context: RenderContext,
) {
    let Some(mut gpu) = gpu else {
        return;
    };
    let (Some(def), Some(sun)) = (frame.world().sun_effects, frame.sun_effects) else {
        return;
    };
    let ids = gpu.pipelines;
    let pipeline = |stage: Stage| ids[stage.bay()].and_then(|id| pipelines.get_render_pipeline(id));
    let Some(history_pipe) = pipeline(Stage::History) else {
        return;
    };
    let (out_tgt, extracted, depth) = cam_view.into_inner();
    if extracted.viewport.z == 0 || extracted.viewport.w == 0 {
        return;
    }

    let written = gpu.write;
    let previous = 1 - written;
    let uniforms = SunUniforms::from_frame(&sun);
    render_queue.write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&uniforms));
    let binder = Binder { gpu: &gpu, pipelines: &pipelines, render_device: &render_device, depth: depth.view() };
    let white = &gpu.white.view;
    let commands = render_context.command_encoder();

    // Update the visibility history first; every later pass reads what this one wrote.
    let bind_grp = binder.band(&gpu.history[previous].view, white, white);
    run_pass(commands, "bo2_sun_history", cleared(&gpu.history[written].view), history_pipe, &bind_grp, 3);

    let current = &gpu.history[written].view;
    let in_front = sun.view_dot > 0.0 && !sun.behind_camera;
    let colour_ok = out_tgt.main_texture_format() == TextureFormat::Rgba8Unorm;
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
            let bind_grp = binder.band(current, sprite, white);
            run_pass(commands, label, out_tgt.get_unsampled_color_attachment(), pipe, &bind_grp, 6);
        }

        let grading = def.blind_max_darken > 0.0 || def.glare_max_lighten > 0.0;
        if grading && let Some(pipe) = pipeline(Stage::Grade) {
            // Read the scene as it stands and write the graded copy back as the new main texture.
            let swap = out_tgt.post_process_write();
            let bind_grp = binder.band(current, white, swap.source);
            run_pass(commands, "bo2_sun_blind", cleared(swap.destination), pipe, &bind_grp, 3);
        }
    }
    gpu.write = previous;
}
