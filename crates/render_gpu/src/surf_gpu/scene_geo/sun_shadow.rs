//! bo2mp: Black Ops II's sun shadows: the things that move (players,
//! bodies, thrown things, script models) cast shadows from the sun onto the
//! world and onto each other. As the game: two sun partitions, the near one
//! `1024 * sm_sunSampleSizeNear` units across and the far one four times
//! that. In the sun's view the camera sits inside both where its near plane
//! leans (an eye looking along the sun's view sits at the centre; one
//! looking across it sits at the edge behind it, so the map reaches ahead),
//! and both snap to whole far texels so they do not crawl; the lit shaders
//! read them with a compare sampler (`sun_shadow_at` in the world shader).
//! The world itself keeps its baked sun shadows.

use super::*;
use bevy::render::render_resource::{BufferDescriptor, Extent3d, LoadOp, Operations, RenderPassDepthStencilAttachment};
use bevy::render::render_resource::{RenderPassDescriptor, TextureDescriptor, TextureDimension, TextureUsages};
use bevy::render::render_resource::{TextureView, TextureViewDescriptor};

pub(super) const SUN_SHADOW_SHADER: &str = "embedded://render_gpu/surf_gpu/t6_sun_shadow.wgsl";
/// Texels per partition side (`1024 << sm_sunQuality`, quality 1).
const SIZE: u32 = 2048;
/// The lit side's constants: both partitions' clip-from-world, then (on,
/// near texel, far texel, depth range).
pub(super) const PARAMS_BYTES: u64 = 144;
/// `sm_sunSampleSizeNear` where the map's own is not known (`_load.csc`'s).
const SAMPLE_SIZE_NEAR: f32 = 0.5;
/// How far casters reach toward the sun and receivers away from it.
const DEPTH_HALF: f32 = 4096.0;

#[derive(Resource)]
pub(super) struct SunShadow {
    /// Texels per side: `SIZE`, or 1 under the fast quality setting, which
    /// draws no sun shadow (the full pair is 32 MB of card memory a game).
    size: u32,
    layers: [TextureView; 2],
    pub(super) array: TextureView,
    pub(super) sampler: Sampler,
    pub(super) params: Buffer,
    casters: [Buffer; 2],
    caster_layout: BindGroupLayoutDescriptor,
    caster_groups: Option<[BindGroup; 2]>,
    shader_h: Handle<Shader>,
    pipeline: Option<CachedRenderPipelineId>,
}

pub(super) fn init(mut commands: Commands, gpu_dev: Res<RenderDevice>, asset_srv: Res<AssetServer>) {
    // The settings (Startup) are read before the renderer starts.
    let side = if asset_core::t6_fast() { 1 } else { SIZE };
    let pair_ext = Extent3d { width: side, height: side, depth_or_array_layers: 2 };
    let bound = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    let depth = gpu_dev.create_texture(&TextureDescriptor {
        label: Some("bo2mp_sun_shadow"),
        size: pair_ext,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: bound,
        view_formats: &[],
    });
    let layer = |base_array_layer| {
        depth.create_view(&TextureViewDescriptor {
            label: Some("bo2mp_sun_shadow_partition"),
            dimension: Some(TextureViewDimension::D2),
            base_array_layer,
            array_layer_count: Some(1),
            ..Default::default()
        })
    };
    let layers = [layer(0), layer(1)];
    let array = depth.create_view(&TextureViewDescriptor {
        label: Some("bo2mp_sun_shadow_partitions"),
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = gpu_dev.create_sampler(&SamplerDescriptor {
        label: Some("bo2mp_sun_shadow_compare"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        compare: Some(CompareFunction::LessEqual),
        ..Default::default()
    });
    let usage = BufferUsages::UNIFORM | BufferUsages::COPY_DST;
    let uniform = |label: &'static str, size: u64| {
        gpu_dev.create_buffer(&BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
    };
    commands.insert_resource(SunShadow {
        size: side,
        layers,
        array,
        sampler,
        params: uniform("bo2mp_sun_shadow_params", PARAMS_BYTES),
        casters: [
            uniform("bo2mp_sun_shadow_near", 64),
            uniform("bo2mp_sun_shadow_far", 64),
        ],
        caster_layout: BindGroupLayoutDescriptor::new(
            "bo2mp_sun_shadow_casters",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (uniform_buffer_sized(false, std::num::NonZeroU64::new(64)),),
            ),
        ),
        caster_groups: None,
        shader_h: asset_srv.load(SUN_SHADOW_SHADER),
        pipeline: None,
    });
}

/// The casters' pipeline: depth only from the sun, both faces (a body's
/// back faces the sun as often as its front), with a slope bias against
/// a lit side shadowing itself.
fn caster_pipeline(shadow_res: &SunShadow) -> RenderPipelineDescriptor {
    let position = VertexAttribute { format: VertexFormat::Float32x3, offset: 0, shader_location: 0 };
    let mesh_desc = VertexBufferLayout { array_stride: size_of::<PropVertex>() as u64, step_mode: VertexStepMode::Vertex, attributes: vec![position] };
    // The instance rows: 28 floats (`t6_prop_instances`), one column per attribute.
    let column = |at: u32| VertexAttribute { format: VertexFormat::Float32x4, offset: u64::from(at) * 16, shader_location: 6 + at };
    let (stride, attributes) = (size_of::<[f32; 28]>() as u64, (0..4).map(column).collect());
    let row_data = VertexBufferLayout { array_stride: stride, step_mode: VertexStepMode::Instance, attributes };
    let entry_point = Some("vertex_caster".into());
    let vertex = VertexState { shader: shadow_res.shader_h.clone(), shader_defs: Vec::new(), entry_point, buffers: vec![mesh_desc, row_data] };
    let bias = DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 };
    let (format, depth_compare) = (TextureFormat::Depth32Float, Some(CompareFunction::LessEqual));
    let depth = DepthStencilState { format, depth_write_enabled: Some(true), depth_compare, stencil: Default::default(), bias };
    RenderPipelineDescriptor {
        label: Some("bo2mp_sun_shadow_casters".into()),
        layout: vec![shadow_res.caster_layout.clone()],
        immediate_size: 0,
        vertex,
        fragment: None,
        primitive: PrimitiveState { front_face: FrontFace::Cw, cull_mode: None, ..Default::default() },
        depth_stencil: Some(depth),
        multisample: MultisampleState::default(),
        zero_initialize_workgroup_memory: false,
    }
}

/// Both partitions' clip-from-world, as Black Ops II places them: the
/// camera's texel is where it sits in the sun-view box around it and its
/// near plane's corners (the same in both partitions), and the corner of
/// each snaps to whole far texels.
fn split_cascades(sun_mat: Mat4, view_to_world: Mat4, tan: Vec2, texel_near: f32) -> [Mat4; 2] {
    let n = SIZE as f32;
    let o = sun_mat.transform_point3(view_to_world.w_axis.truncate());
    let (mut lo, mut hi) = (o.truncate(), o.truncate());
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let corner = view_to_world.transform_point3(Vec3::new(sx * tan.x, sy * tan.y, -1.0));
        let c = sun_mat.transform_point3(corner).truncate();
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let side = hi - lo;
    let k = (n - 1.0) / side.x.max(side.y).max(1e-6);
    let mid = (lo + hi) * 0.5;
    // The camera's texel from the left and from the top.
    let cx = if o.x < mid.x {
        (o.x - lo.x) * k + 1.0
    } else {
        n - 1.0 - (hi.x - o.x) * k
    };
    let cy = if o.y > mid.y {
        (hi.y - o.y) * k + 1.0
    } else {
        n - 1.0 - (o.y - lo.y) * k
    };
    let far_texel = texel_near * 4.0;
    let snap = (o.truncate() / far_texel).floor() * far_texel;
    let depth = -o.z;
    [texel_near, far_texel].map(|t| {
        let left = snap.x - ((snap.x - o.x) / t + cx).floor() * t;
        let top = snap.y + (cy - (snap.y - o.y) / t).floor() * t;
        Mat4::orthographic_rh(
            left,
            left + n * t,
            top - n * t,
            top,
            depth - DEPTH_HALF,
            depth + DEPTH_HALF,
        ) * sun_mat
    })
}

/// A draw that casts a sun shadow: an opaque, untested surface, or a
/// shadow-only one (effects and clouds never do).
fn casts(code: u32) -> bool {
    if code == T6_NONE || code & (T6_EFFECT | T6_CLOUD) != 0 {
        return false;
    }
    match code & T6_BLEND_MASK {
        0 => code & T6_ALPHA_TEST == 0,
        T6_SHADOW_ONLY => true,
        _ => false,
    }
}

/// Draws both partitions before the world pass, or switches the lit side's
/// shadow off (no sun, nothing to cast).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_sun_shadow(
    view: ViewQuery<(&ExtractedView, &OverlayCameraBinds), Without<crate::surf_gpu::portal_view::PortalCamera>>,
    shadow: Option<ResMut<SunShadow>>,
    dynamic: Res<OverlayMovingLists>,
    geometry: Res<OverlayGpuScene>,
    pipes: Res<PipelineCache>,
    gpu_dev: Res<RenderDevice>,
    gpu_queue: Res<RenderQueue>,
    mut render: RenderContext,
) {
    if !geometry_inspect_enabled() {
        return;
    }
    let Some(mut shadow) = shadow else {
        return;
    };
    let shadow = &mut *shadow;
    if shadow.pipeline.is_none() {
        shadow.pipeline = Some(pipes.queue_render_pipeline(caster_pipeline(shadow)));
    }
    if shadow.caster_groups.is_none() {
        let layout = pipes.get_bind_group_layout(&shadow.caster_layout);
        let group = |buffer: &Buffer| {
            let entries = BindGroupEntries::sequential((buffer.as_entire_binding(),));
            gpu_dev.create_bind_group("bo2mp_sun_shadow_casters", &layout, &entries)
        };
        shadow.caster_groups = Some([group(&shadow.casters[0]), group(&shadow.casters[1])]);
    }
    let lists = [dynamic.missiles.as_ref(), dynamic.script_static.as_ref()];
    // bo2mp: the fast quality setting (8 games at once) draws no sun shadow.
    let any = !asset_core::t6_fast()
        && shadow.size == SIZE
        && lists
            .iter()
            .flatten()
            .any(|(_, _, _, draws)| draws.iter().any(|d| d[1] != 0 && casts(d[3])));
    let sun = geometry
        .t6_lighting
        .map(|l| Vec3::new(l[0][0], l[0][1], l[0][2]))
        .filter(|d| d.length_squared() > 1e-6)
        .map(Vec3::normalize);
    let pipeline = shadow.pipeline.and_then(|id| pipes.get_render_pipeline(id));
    let mut params = [0.0f32; (PARAMS_BYTES / 4) as usize];
    let (Some(sun_res), Some(pipe), true) = (sun, pipeline, any) else {
        gpu_queue.write_buffer(&shadow.params, 0, bytemuck::cast_slice(&params));
        return;
    };
    let extracted = view.into_inner().0;
    let world_from_view = extracted.world_from_view.to_matrix();
    let clip_from_view = extracted.clip_from_view;
    let tan = Vec2::new(1.0 / clip_from_view.x_axis.x, 1.0 / clip_from_view.y_axis.y);
    // R_GetSunAxes: up is the world's up unless the sun is near overhead.
    let up = if sun_res.truncate().length_squared() < 0.1 {
        Vec3::X
    } else {
        Vec3::Z
    };
    let sun_view = Mat4::look_to_rh(Vec3::ZERO, -sun_res, up);
    let sample_size = match geometry.t6_sun_sample_size_near {
        size if size > 0.0 => size,
        _ => SAMPLE_SIZE_NEAR,
    };
    let near_extent = 1024.0 * sample_size;
    let extents = [near_extent, near_extent * 4.0];
    let mats = split_cascades(sun_view, world_from_view, tan, extents[0] / SIZE as f32);
    params[..16].copy_from_slice(&mats[0].to_cols_array());
    params[16..32].copy_from_slice(&mats[1].to_cols_array());
    params[32] = 1.0;
    params[33] = extents[0] / SIZE as f32;
    params[34] = extents[1] / SIZE as f32;
    params[35] = DEPTH_HALF * 2.0;
    gpu_queue.write_buffer(&shadow.params, 0, bytemuck::cast_slice(&params));
    for (buffer, mat) in shadow.casters.iter().zip(mats.iter()) {
        gpu_queue.write_buffer(buffer, 0, bytemuck::cast_slice(&mat.to_cols_array()));
    }
    let Some(groups) = shadow.caster_groups.as_ref() else {
        return;
    };
    for (layer, group) in shadow.layers.iter().zip(groups.iter()) {
        let clear_ops = Operations { load: LoadOp::Clear(1.0), store: StoreOp::Store };
        let out_tgt = RenderPassDepthStencilAttachment { view: layer, depth_ops: Some(clear_ops), stencil_ops: None };
        let mut rpass = render.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("bo2mp_sun_shadow_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(out_tgt),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        rpass.set_render_pipeline(pipe);
        rpass.set_bind_group(0, group, &[]);
        for (vb, ibuf, inst, draws) in lists.iter().flatten() {
            rpass.set_vertex_buffer(0, vb.slice(..));
            rpass.set_vertex_buffer(1, inst.slice(..));
            rpass.set_index_buffer(ibuf.slice(..), IndexFormat::Uint32);
            for &[first_ix, ix_count, _, code, instance, _, _] in draws {
                if ix_count != 0 && casts(code) {
                    rpass.draw_indexed(first_ix..first_ix.saturating_add(ix_count), 0, instance..instance + 1);
                }
            }
        }
    }
}
