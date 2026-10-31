//! bo2mp: depth of field on a Black Ops II map. The engine's own depth of
//! field (`postfx.rs`) needs the MW2 post-effect materials, which a BO2 map
//! does not load, so a BO2 match had none: no blur on near things while
//! aiming down the sights (real 53: the near wall and car are soft, the gun
//! and the distance sharp) and none in the death view (real 54: the body in
//! the near corner is blurred, the killer sharp). The ranges are the same
//! frame values the engine's pass reads (`BokehFrame`: the sights' autofocus,
//! the dead view's, script and dvar overrides); this pass draws them: the
//! frame at quarter size with each block's blur amount, blurred, then mixed
//! back by each pixel's distance from the scene depth.

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::RenderStartup;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer, BufferDescriptor,
    BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FilterMode,
    FragmentState, LoadOp, Operations, PipelineCache, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, StoreOp, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::shader::Shader;
use std::collections::HashMap;
use std::num::NonZeroU64;

use super::scene_depth::SceneDepthTexture;

const SHADER_SOURCE_PATH: &str = "embedded://render_gpu/surf_gpu/bo2_dof.wgsl";
const PARAMS_BYTES: u64 = 80;
const QUARTER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

#[derive(Resource)]
struct Bo2Dof {
    blur_layout: BindGroupLayoutDescriptor,
    shrink_layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    program: Handle<Shader>,
    sampler: Option<Sampler>,
    /// Two quarter-size ping-pong targets, for the frame size they were made at.
    quarter: Option<(UVec2, [(Texture, TextureView); 2])>,
    /// shrink, blur x, blur y, composite.
    params: Option<[Buffer; 4]>,
    shrink: Option<CachedRenderPipelineId>,
    blur: Option<CachedRenderPipelineId>,
    composite: HashMap<TextureFormat, CachedRenderPipelineId>,
}

fn init(mut commands: Commands, asset_srv: Res<AssetServer>) {
    let float = || texture_2d(TextureSampleType::Float { filterable: true });
    let params = || uniform_buffer_sized(false, NonZeroU64::new(PARAMS_BYTES));
    commands.insert_resource(Bo2Dof {
        blur_layout: BindGroupLayoutDescriptor::new(
            "bo2mp_dof_blur",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, float()),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, params()),
                ),
            ),
        ),
        shrink_layout: BindGroupLayoutDescriptor::new(
            "bo2mp_dof_shrink",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, float()),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, params()),
                    (4, texture_depth_2d()),
                ),
            ),
        ),
        composite_layout: BindGroupLayoutDescriptor::new(
            "bo2mp_dof_composite",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, float()),
                    (1, sampler(SamplerBindingType::Filtering)),
                    (2, params()),
                    (3, float()),
                    (4, texture_depth_2d()),
                ),
            ),
        ),
        program: asset_srv.load(SHADER_SOURCE_PATH),
        sampler: None,
        quarter: None,
        params: None,
        shrink: None,
        blur: None,
        composite: HashMap::new(),
    });
}

fn pipeline(
    dof: &Bo2Dof,
    entry: &'static str,
    bind_layout: &BindGroupLayoutDescriptor,
    format: TextureFormat,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(format!("bo2mp_dof_{entry}").into()),
        layout: vec![bind_layout.clone()],
        immediate_size: 0,
        vertex: VertexState {
            shader: dof.program.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("vertex".into()),
            buffers: Vec::new(),
        },
        fragment: Some(FragmentState {
            shader: dof.program.clone(),
            shader_defs: Vec::new(),
            entry_point: Some(entry.into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        zero_initialize_workgroup_memory: false,
    }
}

/// The blur's step in quarter texels for a frame `height` high: the engine's
/// near blur radius (`near_blur` at a 480-high screen, a quarter of it in
/// quarter texels), over the 9-tap blur's own spread.
fn blur_step(height: u32, near_blur: f32) -> f32 {
    (height as f32 * near_blur * 0.25 / 480.0 / 1.5).clamp(1.0, 4.0)
}

fn draw_bo2_dof(
    cam_view: ViewQuery<(&ViewTarget, &SceneDepthTexture, &ExtractedView), Without<crate::surf_gpu::portal_view::PortalCamera>>,
    enabled: Res<super::bo2_bloom::Bo2BloomEnabled>,
    extracted: Res<super::postfx_frame::ExtractedPostEffect>,
    dof: Option<ResMut<Bo2Dof>>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
    mut logged: Local<Option<super::dof_bloom::FocusBlur>>,
    mut context: RenderContext,
) {
    if !enabled.0 || !crate::surf_gpu::geometry_inspect_enabled() {
        return;
    }
    let focus = extracted.frame.dof_vals;
    // The engine's own chain draws nothing under a near blur of 4 either
    // (`dof_bloom`'s plan).
    let on = focus.active() && focus.near_soft >= 4.0;
    // Once per change: what the pass draws (the sights' and the dead tex_view's
    // focus come from the frame values, so this shows they reach a BO2 map).
    let dof_shown = on.then_some(focus);
    if *logged != dof_shown {
        match dof_shown {
            Some(focus) => info!(
                "bo2mp dof: near {:.0}-{:.0} far {:.0}-{:.0} gun {:.1}-{:.1} blur {:.2}/{:.2}",
                focus.near_from,
                focus.near_to,
                focus.far_from,
                focus.far_to,
                focus.hand_model_start,
                focus.hand_model_end,
                focus.near_soft,
                focus.far_soft
            ),
            None => info!("bo2mp dof: off"),
        }
        *logged = dof_shown;
    }
    if !on {
        return;
    }
    let Some(mut dof) = dof else {
        return;
    };
    let (out_tgt, depth, ext_view) = cam_view.into_inner();
    let Some(znear) = znear_from_clip_from_view(ext_view.clip_from_view)
    else {
        return;
    };
    let format = out_tgt.main_texture_format();
    if dof.shrink.is_none() {
        let p = pipeline(&dof, "shrink", &dof.shrink_layout, QUARTER_FORMAT);
        dof.shrink = Some(pipes.queue_render_pipeline(p));
        let p = pipeline(&dof, "blur", &dof.blur_layout, QUARTER_FORMAT);
        dof.blur = Some(pipes.queue_render_pipeline(p));
    }
    if !dof.composite.contains_key(&format) {
        let p = pipeline(&dof, "composite", &dof.composite_layout, format);
        let id = pipes.queue_render_pipeline(p);
        dof.composite.insert(format, id);
    }
    let (Some(shrink), Some(blur), Some(composite)) = (
        dof.shrink.and_then(|id| pipes.get_render_pipeline(id)),
        dof.blur.and_then(|id| pipes.get_render_pipeline(id)),
        dof.composite.get(&format).and_then(|&id| pipes.get_render_pipeline(id)),
    ) else {
        return;
    };
    let out_ext = out_tgt.main_texture().size();
    let full_size = UVec2::new(out_ext.width, out_ext.height);
    let qsize = (full_size / 4).max(UVec2::ONE);
    if dof.quarter.as_ref().is_none_or(|(have, _)| *have != full_size) {
        let make = |pass_name: &'static str| {
            let tex = gpu_dev.create_texture(&TextureDescriptor {
                label: Some(pass_name),
                size: Extent3d {
                    width: qsize.x,
                    height: qsize.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: QUARTER_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let tex_view = tex.create_view(&TextureViewDescriptor::default());
            (tex, tex_view)
        };
        dof.quarter = Some((full_size, [make("bo2mp_dof_a"), make("bo2mp_dof_b")]));
    }
    let sampler = dof
        .sampler
        .get_or_insert_with(|| {
            gpu_dev.create_sampler(&SamplerDescriptor {
                label: Some("bo2mp_dof_sampler"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            })
        })
        .clone();
    let params = dof
        .params
        .get_or_insert_with(|| {
            std::array::from_fn(|_| {
                gpu_dev.create_buffer(&BufferDescriptor {
                    label: Some("bo2mp_dof_params"),
                    size: PARAMS_BYTES,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
        })
        .clone();
    let band = super::z_layers::GUN_DEPTH_SHARE;
    // On a BO2 map the gun is drawn with the scene's own projection
    // (scene_geo.wgsl: tex_view.clip_from_world), only squeezed into
    // the gun's depth band, so its distance comes back with the scene near.
    // The engine's gun near (0.1) put every gun pixel 20 times too close and
    // the whole gun blurred while aiming.
    let gun_near = znear;
    // r_dof_bias: BO2's blur size is near blur x amount^(1 / bias), and the
    // far blur's share is (far / near)^bias, as the engine's own chain has.
    let bias = extracted.frame.bias;
    let (far_share, power) = if bias > 0.0 {
        ((focus.far_soft / focus.near_soft).powf(bias), 1.0 / bias)
    } else {
        (focus.far_soft / focus.near_soft, 1.0)
    };
    let write = |buffer: &Buffer, texel: Vec2, dir: Vec2| {
        let data: [f32; 20] = [
            texel.x,
            texel.y,
            dir.x,
            dir.y,
            focus.near_from,
            focus.near_to,
            focus.far_from,
            focus.far_to,
            far_share.clamp(0.0, 1.0),
            power,
            0.0,
            0.0,
            znear,
            1.0 - band,
            band,
            gun_near,
            focus.hand_model_start,
            focus.hand_model_end,
            0.0,
            0.0,
        ];
        gpu_queue.write_buffer(buffer, 0, bytemuck::cast_slice(&data));
    };
    let full_texel = Vec2::new(1.0 / full_size.x as f32, 1.0 / full_size.y as f32);
    let quarter_texel = Vec2::new(1.0 / qsize.x as f32, 1.0 / qsize.y as f32);
    let step = blur_step(full_size.y, focus.near_soft);
    write(&params[0], full_texel, Vec2::ZERO);
    write(&params[1], quarter_texel, Vec2::new(step, 0.0));
    write(&params[2], quarter_texel, Vec2::new(0.0, step));
    write(&params[3], full_texel, Vec2::ZERO);
    let Some(halves) = dof.quarter.as_ref() else {
        return;
    };
    let (_, [(_, a), (_, b)]) = halves;
    let blur_layout = pipes.get_bind_group_layout(&dof.blur_layout);
    let shrink_layout = pipes.get_bind_group_layout(&dof.shrink_layout);
    let composite_layout = pipes.get_bind_group_layout(&dof.composite_layout);
    let post_tex = out_tgt.post_process_write();
    let enc = context.command_encoder();
    let mut run = |pass_name: &'static str,
                   pipe: &bevy::render::render_resource::RenderPipeline,
                   bind_grp: &bevy::render::render_resource::BindGroup,
                   out: &TextureView| {
        let mut rp = enc.begin_render_pass(&RenderPassDescriptor {
            label: Some(pass_name),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: out,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipe);
        rp.set_bind_group(0, bind_grp, &[]);
        rp.draw(0..3, 0..1);
    };
    let shrink_bind = gpu_dev.create_bind_group(
        "bo2mp_dof_shrink",
        &shrink_layout,
        &BindGroupEntries::with_indices((
            (0, post_tex.source),
            (1, &sampler),
            (2, params[0].as_entire_binding()),
            (4, depth.view()),
        )),
    );
    let blur_bind = |source: &TextureView, buffer: &Buffer| {
        gpu_dev.create_bind_group(
            "bo2mp_dof_blur",
            &blur_layout,
            &BindGroupEntries::with_indices((
                (0, source),
                (1, &sampler),
                (2, buffer.as_entire_binding()),
            )),
        )
    };
    run("bo2mp_dof_shrink", shrink, &shrink_bind, a);
    run("bo2mp_dof_blur_h", blur, &blur_bind(a, &params[1]), b);
    run("bo2mp_dof_blur_v", blur, &blur_bind(b, &params[2]), a);
    let composite_bind = gpu_dev.create_bind_group(
        "bo2mp_dof_composite",
        &composite_layout,
        &BindGroupEntries::with_indices((
            (0, post_tex.source),
            (1, &sampler),
            (2, params[3].as_entire_binding()),
            (3, a),
            (4, depth.view()),
        )),
    );
    run("bo2mp_dof_composite", composite, &composite_bind, post_tex.destination);
}

pub(super) fn register(host: &mut App) {
    bevy::asset::embedded_asset!(host, "bo2_dof.wgsl");
    let Some(render_sub) = host.get_sub_app_mut(bevy::render::RenderApp) else {
        return;
    };
    render_sub.add_systems(RenderStartup, init).add_systems(
        Core3d,
        draw_bo2_dof
            .in_set(Core3dSystems::PostProcess)
            .before(super::bo2_bloom::draw_bo2_bloom),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blur_step_grows_with_the_screen_and_the_near_blur() {
        // 720 high, the engine's near blur 6: 2.25 quarter texels of radius.
        assert!((blur_step(720, 6.0) - 1.5).abs() < 1e-4);
        assert!(blur_step(1440, 6.0) > blur_step(720, 6.0));
        assert_eq!(blur_step(240, 1.0), 1.0);
        assert_eq!(blur_step(4320, 10.0), 4.0);
    }
}

/// The view's near plane from its reverse-Z projection (moved from the MW2 floatz pass).
fn znear_from_clip_from_view(clip_from_view: Mat4) -> Option<f32> {
    let z_near = clip_from_view.w_axis.z;
    (z_near > 0.0).then_some(z_near)
}
