//! The Special Weapons tab's guest weapons on a plain Black Ops II map
//! (Half-Life 2's Gravity Gun, Minecraft's bow and stick): the one in his
//! hands, drawn in the view model's depth band after Black Ops II's own
//! models, and the arrows in the map. `render_anim::guest_weapons` builds
//! the meshes; this is Minecraft Zombies' guest and hand drawing
//! (`minecraft_world`) on its own.
use std::sync::Arc;

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_2d_array, uniform_buffer_sized};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindingResource, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferBinding, BufferDescriptor, BufferInitDescriptor,
    BufferUsages, ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d, FilterMode,
    IndexFormat, MipmapFilterMode, MultisampleState, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PrimitiveState, RawFragmentState, RawRenderPipelineDescriptor,
    RawVertexBufferLayout, RawVertexState, RenderPipeline, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TexelCopyBufferLayout, TexelCopyTextureInfo,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension, VertexAttribute, VertexFormat, VertexStepMode,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewTarget};
use bevy::render::{Render, RenderApp, RenderSystems};

use super::depth_range::DepthLayer;
use super::portal_view::{PortalCamera, PortalLooksNow, PortalTargets};
use super::scene_depth::{SCENE_DEPTH_FORMAT, SceneDepthTexture};

/// Bytes of one vertex: position, uv, colour (alpha the texture layer),
/// light, pad (enchanted shine; 255 = Minecraft's item pictures, 0 = Half-
/// Life 2's sheets).
const VERTEX_BYTES: u64 = 28;

/// A guest weapon's textures: square RGBA8 layers of one size, each with
/// its levels largest first.
pub struct GuestViewTexture {
    pub size: u32,
    pub layers: Vec<Vec<Vec<u8>>>,
}

/// This frame's guest weapon, from the main world.
#[derive(Resource, Default)]
pub struct GuestViewFrame {
    /// In his hands: view-space vertices under `clip`, the first `solid`
    /// indices solid, the rest glow.
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub solid: u32,
    pub clip: [f32; 16],
    pub hl2: Option<Arc<GuestViewTexture>>,
    pub items: Option<Arc<GuestViewTexture>>,
    /// The arrows: map-unit vertices.
    pub world_vertices: Vec<u8>,
    pub world_indices: Vec<u32>,
    pub clock: f32,
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct GuestUniform {
    clip: [f32; 16],
    clip_from_world: [f32; 16],
    /// x: the shine's clock; yzw: the eye, which `clip_from_world` is about.
    misc: [f32; 4],
    /// bo2mp: x the view (0 his, 1 + slot through a portal); y the pair
    /// seen through; zw whether slot 0 / 1 was drawn.
    look: [f32; 4],
}

const UNIFORM_SIZE: u64 = std::mem::size_of::<GuestUniform>() as u64;

type Mesh = Option<(Buffer, Buffer, u32, u32)>;

#[derive(Resource, Default)]
struct GuestGpu {
    /// One per view (his, then each portal slot's): a frame's writes all
    /// land before its passes run.
    uniforms: [Option<Buffer>; 3],
    /// Stands in for a portal picture not drawn.
    blank: Option<TextureView>,
    held: Mesh,
    world: Mesh,
    hl2: Option<(Option<Arc<GuestViewTexture>>, TextureView)>,
    items: Option<(Option<Arc<GuestViewTexture>>, TextureView)>,
    linear: Option<Sampler>,
    nearest: Option<Sampler>,
    clip: [f32; 16],
    clock: f32,
    pipelines: bevy::platform::collections::HashMap<(TextureFormat, u32), [RenderPipeline; 3]>,
}

pub(super) fn register(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<GuestViewFrame>()
        .init_resource::<GuestGpu>()
        .init_resource::<GuestLayoutCache>()
        .add_systems(Render, prepare_guest.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            draw_guest
                .in_set(Core3dSystems::MainPass)
                .after(super::draw::ExactColourDrawSet)
                .after(super::geometry_diagnostic::GeometryDiagnosticDrawSet),
        );
}

/// The guest view's bind-group layout, built once on first use.
#[derive(Resource, Default)]
struct GuestLayoutCache {
    layout: std::sync::Mutex<Option<BindGroupLayout>>,
}

impl GuestLayoutCache {
    fn get(&self, device: &RenderDevice) -> BindGroupLayout {
        self.layout
            .lock()
            .expect("guest view layout cache is never poisoned")
            .get_or_insert_with(|| {
                let descriptor = layout();
                device.create_bind_group_layout(descriptor.label.as_ref(), &descriptor.entries)
            })
            .clone()
    }
}

fn layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "bo2mp_guest_view",
        &[
            uniform_buffer_sized(false, std::num::NonZeroU64::new(UNIFORM_SIZE))
                .visibility(ShaderStages::VERTEX_FRAGMENT)
                .build(0, ShaderStages::VERTEX_FRAGMENT),
            texture_2d_array(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(1, ShaderStages::FRAGMENT),
            texture_2d_array(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(2, ShaderStages::FRAGMENT),
            sampler(SamplerBindingType::Filtering)
                .visibility(ShaderStages::FRAGMENT)
                .build(3, ShaderStages::FRAGMENT),
            sampler(SamplerBindingType::Filtering)
                .visibility(ShaderStages::FRAGMENT)
                .build(4, ShaderStages::FRAGMENT),
            texture_2d(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(5, ShaderStages::FRAGMENT),
            texture_2d(TextureSampleType::Float { filterable: true })
                .visibility(ShaderStages::FRAGMENT)
                .build(6, ShaderStages::FRAGMENT),
        ],
    )
}

fn mesh(device: &RenderDevice, vertices: &[u8], indices: &[u32], solid: u32) -> Mesh {
    (!indices.is_empty() && !vertices.is_empty()).then(|| {
        (
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("bo2mp_guest_view_vertices"),
                contents: vertices,
                usage: BufferUsages::VERTEX,
            }),
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("bo2mp_guest_view_indices"),
                contents: bytemuck::cast_slice(indices),
                usage: BufferUsages::INDEX,
            }),
            solid.min(indices.len() as u32),
            indices.len() as u32,
        )
    })
}

/// Uploads a texture when it changed; true when it did.
fn keep_texture(
    device: &RenderDevice,
    queue: &RenderQueue,
    held: &mut Option<(Option<Arc<GuestViewTexture>>, TextureView)>,
    wanted: Option<Arc<GuestViewTexture>>,
) -> bool {
    let stale = match (&*held, &wanted) {
        (None, _) => true,
        (Some((Some(had), _)), Some(new)) => !Arc::ptr_eq(had, new),
        (Some((had, _)), new) => had.is_some() != new.is_some(),
    };
    if stale {
        let white = GuestViewTexture { size: 1, layers: vec![vec![vec![255; 4]]] };
        let view = upload(device, queue, wanted.as_deref().unwrap_or(&white));
        *held = Some((wanted, view));
    }
    stale
}

fn prepare_guest(
    mut frame: ResMut<GuestViewFrame>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<GuestGpu>,
) {
    let held_vertices = std::mem::take(&mut frame.vertices);
    let held_indices = std::mem::take(&mut frame.indices);
    gpu.held = mesh(&device, &held_vertices, &held_indices, frame.solid);
    let world_vertices = std::mem::take(&mut frame.world_vertices);
    let world_indices = std::mem::take(&mut frame.world_indices);
    let count = world_indices.len() as u32;
    gpu.world = mesh(&device, &world_vertices, &world_indices, count);
    gpu.clip = frame.clip;
    gpu.clock = frame.clock;
    if gpu.held.is_none() && gpu.world.is_none() {
        return;
    }
    let gpu = &mut *gpu;
    keep_texture(&device, &queue, &mut gpu.hl2, frame.hl2.clone());
    keep_texture(&device, &queue, &mut gpu.items, frame.items.clone());
    if gpu.uniforms[0].is_none() {
        for uniform in &mut gpu.uniforms {
            *uniform = Some(device.create_buffer(&BufferDescriptor {
                label: Some("bo2mp_guest_view"),
                size: UNIFORM_SIZE,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let blank = device.create_texture(&TextureDescriptor {
            label: Some("bo2mp_portal_view_blank"),
            size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        gpu.blank = Some(blank.create_view(&TextureViewDescriptor::default()));
        // Source's textures wrap and filter; Minecraft's stay sharp.
        gpu.linear = Some(device.create_sampler(&SamplerDescriptor {
            label: Some("bo2mp_guest_view_linear"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..default()
        }));
        gpu.nearest = Some(device.create_sampler(&SamplerDescriptor {
            label: Some("bo2mp_guest_view_nearest"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..default()
        }));
    }
}

/// A texture's layers as one array texture, every level.
fn upload(device: &RenderDevice, queue: &RenderQueue, guest: &GuestViewTexture) -> TextureView {
    let levels = guest.layers.iter().map(Vec::len).min().unwrap_or(1).max(1) as u32;
    let count = guest.layers.len().max(1) as u32;
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("bo2mp_guest_view"),
        size: Extent3d { width: guest.size, height: guest.size, depth_or_array_layers: count },
        mip_level_count: levels,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, mips) in guest.layers.iter().enumerate() {
        for (level, texels) in mips.iter().take(levels as usize).enumerate() {
            let side = (guest.size >> level).max(1);
            if texels.len() < (side * side * 4) as usize {
                break;
            }
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: Origin3d { x: 0, y: 0, z: layer as u32 },
                    aspect: TextureAspect::All,
                },
                texels,
                TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 4), rows_per_image: Some(side) },
                Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            );
        }
    }
    texture.create_view(&TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() })
}

fn draw_guest(
    view: ViewQuery<(&ViewTarget, &SceneDepthTexture, &ExtractedView, Option<&Msaa>)>,
    layouts: Res<GuestLayoutCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<GuestGpu>,
    published: Option<Res<super::PublishedRenderFrame>>,
    (portal_views, looks_now, targets): (Query<&PortalCamera>, Res<PortalLooksNow>, Res<PortalTargets>),
    mut context: RenderContext,
) {
    if gpu.held.is_none() && gpu.world.is_none() {
        return;
    }
    // bo2mp: a view through a portal (`portal_view`) draws the map's
    // guests from its own eye, and nothing in his hands.
    let through = portal_views.get(view.entity()).ok().map(|p| p.slot);
    let index = through.map_or(0, |slot| 1 + slot.min(1));
    let (Some(uniform), Some(blank), Some((_, hl2)), Some((_, items)), Some(linear), Some(nearest)) = (
        gpu.uniforms[index].clone(),
        gpu.blank.clone(),
        gpu.hl2.as_ref(),
        gpu.items.as_ref(),
        gpu.linear.as_ref(),
        gpu.nearest.as_ref(),
    ) else {
        return;
    };
    let pictures = [0, 1].map(|slot| {
        targets.sampled(slot).filter(|_| through.is_none() && looks_now.active[slot]).unwrap_or(&blank).clone()
    });
    let layout = layouts.get(&device);
    let bind = device.create_bind_group(
        "bo2mp_guest_view",
        &layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding { buffer: &uniform, offset: 0, size: None }),
            },
            BindGroupEntry { binding: 1, resource: BindingResource::TextureView(hl2) },
            BindGroupEntry { binding: 2, resource: BindingResource::TextureView(items) },
            BindGroupEntry { binding: 3, resource: BindingResource::Sampler(linear) },
            BindGroupEntry { binding: 4, resource: BindingResource::Sampler(nearest) },
            BindGroupEntry { binding: 5, resource: BindingResource::TextureView(&pictures[0]) },
            BindGroupEntry { binding: 6, resource: BindingResource::TextureView(&pictures[1]) },
        ],
    );
    let (target, depth, extracted_view, msaa) = view.into_inner();
    // The map's own camera (as `minecraft_world`), about the eye so the
    // map's large coordinates keep their precision; none, no arrows.
    let (origin, clip_from_world) = if through.is_some() {
        let eye = extracted_view.world_from_view.to_matrix();
        let turn = Mat4::from_mat3(bevy::math::Mat3::from_mat4(eye));
        (eye.w_axis.truncate(), Some(extracted_view.clip_from_view * turn.inverse()))
    } else {
        let exec = published.as_ref().map(|p| &p.exec_frame);
        (exec.map_or(Vec3::ZERO, |e| e.view_origin), exec.and_then(|e| e.clip_from_world))
    };
    let looked = through.is_none() && (looks_now.active[0] || looks_now.active[1]);
    let values = GuestUniform {
        clip: gpu.clip,
        clip_from_world: clip_from_world.map_or(Mat4::IDENTITY, |m| m * Mat4::from_translation(origin)).to_cols_array(),
        misc: [gpu.clock, origin.x, origin.y, origin.z],
        look: [
            through.map_or(0.0, |slot| 1.0 + slot as f32),
            if looked { (looks_now.pair & 127) as f32 } else { -1.0 },
            if looked && looks_now.active[0] { 1.0 } else { 0.0 },
            if looked && looks_now.active[1] { 1.0 } else { 0.0 },
        ],
    };
    queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&values));
    let format = target.main_texture_format();
    let samples = msaa.map_or(1, Msaa::samples);
    let [held, glow, world] = gpu
        .pipelines
        .entry((format, samples))
        .or_insert_with(|| pipelines(&device, &layouts, format, samples))
        .clone();
    let attachments = [Some(target.get_color_attachment())];
    let mut pass = context.begin_tracked_render_pass(bevy::render::render_resource::RenderPassDescriptor {
        label: Some("bo2mp_guest_view"),
        color_attachments: &attachments,
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let vp = extracted_view.viewport;
    pass.set_bind_group(0, &bind, &[]);
    if let Some((vertices, indices, _, count)) = gpu.world.as_ref().filter(|_| clip_from_world.is_some()) {
        let (depth_min, depth_max) = DepthLayer::World.viewport_depth();
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, depth_min, depth_max);
        pass.set_render_pipeline(&world);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    }
    if let Some((vertices, indices, solid, count)) = gpu.held.as_ref().filter(|_| through.is_none()) {
        let (band_min, band_max) = DepthLayer::Gun.viewport_depth();
        pass.set_viewport(vp.x as f32, vp.y as f32, vp.z as f32, vp.w as f32, band_min, band_max);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        if *solid > 0 {
            pass.set_render_pipeline(&held);
            pass.draw_indexed(0..*solid, 0, 0..1);
        }
        if count > solid {
            pass.set_render_pipeline(&glow);
            pass.draw_indexed(*solid..*count, 0, 0..1);
        }
    }
}

const GUEST_WGSL: &str = r#"
struct GuestUniform {
    clip: mat4x4<f32>,
    clip_from_world: mat4x4<f32>,
    misc: vec4<f32>,
    // x: 0 his view, 1 + slot a view through a portal; y: the pair whose
    // views were drawn (-1 none); z, w: the blue / orange end's view drawn.
    look: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: GuestUniform;
@group(0) @binding(1) var hl2_textures: texture_2d_array<f32>;
@group(0) @binding(2) var item_textures: texture_2d_array<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var nearest_sampler: sampler;
@group(0) @binding(5) var portal_view0: texture_2d<f32>;
@group(0) @binding(6) var portal_view1: texture_2d<f32>;

// The light on the weapon: a little under full, as a lit room's.
const LIGHT: f32 = 0.85;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) @interpolate(flat) layer: u32,
    @location(3) @interpolate(flat) pad: vec2<f32>,
    @location(4) @interpolate(flat) light: vec2<f32>,
}

fn out_of(clip: vec4<f32>, uv: vec2<f32>, colour: vec4<f32>, light: vec2<f32>, pad: vec2<f32>) -> Out {
    var out: Out;
    out.clip = clip;
    out.uv = uv;
    out.colour = colour.rgb;
    out.layer = u32(round(colour.a * 255.0));
    out.pad = pad;
    out.light = light;
    return out;
}

@vertex
fn held_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) light: vec2<f32>,
    @location(4) pad: vec2<f32>,
) -> Out {
    return out_of(u.clip * vec4<f32>(position, 1.0), uv, colour, light, pad);
}

@vertex
fn world_vertex(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) light: vec2<f32>,
    @location(4) pad: vec2<f32>,
) -> Out {
    return out_of(u.clip_from_world * vec4<f32>(position - u.misc.yzw, 1.0), uv, colour, light, pad);
}

// Minecraft's enchantment shine: purple bands sliding across the screen.
fn glint(frag: vec2<f32>) -> vec3<f32> {
    let t = u.misc.x;
    let a = fract((frag.x + frag.y * 0.6) * 0.0035 - t * 0.32);
    let b = fract((frag.x * 0.4 - frag.y) * 0.0028 + t * 0.21);
    let band = smoothstep(0.0, 0.12, a) * (1.0 - smoothstep(0.12, 0.3, a))
        + 0.7 * smoothstep(0.0, 0.1, b) * (1.0 - smoothstep(0.1, 0.25, b));
    return vec3<f32>(0.5, 0.25, 0.8) * (0.28 + 0.9 * band);
}

// MC Zombies' portal (`portalgun_play::portal_mesh`), as Minecraft draws a
// nether portal: square pixels, 16 to a block of its 0.45 by 0.95 block
// oval (uv -1..1 is the oval). A bright rim flickering; inside, the colour
// swirling while it is alone, a dark shimmer once linked (`pad.x`). It
// grows from a dot (`light.x`). The Super one's (`light.y`) rim pulses,
// and a halo out to 1.45 times sends Pack-a-Punch purple rings out and
// sparkles white.
const PORTAL_OVAL: vec2<f32> = vec2<f32>(0.45, 0.95);
const PORTAL_HALO: f32 = 1.45;
const PAP: vec3<f32> = vec3<f32>(190.0, 70.0, 255.0) / 255.0;

fn portal(in: Out) -> vec4<f32> {
    let t = u.misc.x;
    let grow = max(in.light.x, 0.05);
    let upgraded = in.light.y > 0.5;
    let cell = floor(in.uv * PORTAL_OVAL * 16.0);
    let at = (cell + 0.5) / 16.0;
    let q = at / (PORTAL_OVAL * grow);
    let e = dot(q, q);
    let reach = select(1.0, PORTAL_HALO, upgraded);
    if e > reach * reach {
        discard;
    }
    let ij = vec2<i32>(cell) + vec2<i32>(8, 16);
    // Linked, in his view, and its pair's views drawn: inside the rim is
    // the map as seen out of the other end (`portal_view`).
    let linked = in.pad.x > 0.5;
    let orange = in.pad.x > 0.7;
    let pair = u32(round(in.light.y * 255.0)) & 127u;
    if e <= 0.7 && linked && u.look.x < 0.5 && u.look.y >= 0.0 && pair == u32(u.look.y) {
        let at_screen = vec2<i32>(in.clip.xy);
        if orange && u.look.w > 0.5 {
            return vec4<f32>(textureLoad(portal_view1, at_screen, 0).rgb, 1.0);
        }
        if !orange && u.look.z > 0.5 {
            return vec4<f32>(textureLoad(portal_view0, at_screen, 0).rgb, 1.0);
        }
    }
    if e > 1.0 {
        // The halo: 0 at the rim to 1 at its edge.
        let h = (sqrt(e) - 1.0) / (PORTAL_HALO - 1.0);
        if abs(h - fract(t * 1.2)) < 0.12 {
            let k = 1.3 * (1.0 - h * 0.5);
            return vec4<f32>(min((in.colour + PAP) * 0.5 * k, vec3<f32>(1.0)), 1.0);
        }
        let beat = u32(t * 10.0);
        let hash = (u32(ij.x) * 73856093u) ^ (u32(ij.y) * 19349663u) ^ (beat * 83492791u);
        if (hash >> 7u) % 100u < u32(28.0 * (1.0 - h)) {
            let k = 1.0 - h * 0.4;
            return vec4<f32>(k, k * 0.9, 1.0, 1.0);
        }
        discard;
    }
    let swirl = sin(atan2(at.y, at.x) * 3.0 + sqrt(e) * 9.0 - t * 4.0) * 0.5 + 0.5;
    let twinkle = sin(t * 9.0 + f32(ij.x * 7 + ij.y * 3));
    var k = select(0.45 + 0.35 * swirl, 0.12 + 0.12 * swirl, in.pad.x > 0.5);
    if e > 0.7 && upgraded {
        k = 1.35 + 0.35 * sin(t * 6.0) + 0.15 * twinkle;
    } else if e > 0.7 {
        k = 1.1 + 0.15 * twinkle;
    }
    return vec4<f32>(min(in.colour * k, vec3<f32>(1.0)), 1.0);
}

@fragment
fn held_fragment(in: Out) -> @location(0) vec4<f32> {
    if in.pad.y > 0.75 {
        // A Minecraft item: cut out, shaded by its face, the shine on top.
        let texel = textureSampleLevel(item_textures, nearest_sampler, in.uv, in.layer, 0.0);
        if texel.a < 0.1 {
            discard;
        }
        let shine = select(vec3<f32>(0.0), glint(in.clip.xy), in.pad.x > 0.5);
        return vec4<f32>(texel.rgb * in.colour * LIGHT + shine, 1.0);
    }
    if in.pad.y > 0.25 {
        return portal(in);
    }
    // Half-Life 2's: solid; the alpha is Source's `$selfillum` mask, lit
    // where it is 0, its own colour where it is 1.
    let texel = textureSample(hl2_textures, linear_sampler, in.uv, in.layer);
    return vec4<f32>(texel.rgb * mix(in.colour * LIGHT, vec3<f32>(1.0), texel.a), 1.0);
}

// The Gravity Gun's sprites, added over the model.
@fragment
fn glow_fragment(in: Out) -> @location(0) vec4<f32> {
    let texel = textureSample(hl2_textures, linear_sampler, in.uv, in.layer);
    return vec4<f32>(texel.rgb * texel.a * in.colour, 0.0);
}
"#;

fn pipelines(
    device: &RenderDevice,
    layouts: &GuestLayoutCache,
    format: TextureFormat,
    samples: u32,
) -> [RenderPipeline; 3] {
    let shader = unsafe {
        device.create_shader_module(ShaderModuleDescriptor {
            label: Some("bo2mp_guest_view"),
            source: ShaderSource::Wgsl(GUEST_WGSL.into()),
        })
    };
    let layout = layouts.get(device);
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("bo2mp_guest_view"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let attributes = [
        VertexAttribute { format: VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        VertexAttribute { format: VertexFormat::Float32x2, offset: 12, shader_location: 1 },
        VertexAttribute { format: VertexFormat::Unorm8x4, offset: 20, shader_location: 2 },
        VertexAttribute { format: VertexFormat::Unorm8x2, offset: 24, shader_location: 3 },
        VertexAttribute { format: VertexFormat::Unorm8x2, offset: 26, shader_location: 4 },
    ];
    let buffers = [RawVertexBufferLayout {
        array_stride: VERTEX_BYTES,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    }];
    let options = PipelineCompilationOptions { constants: &[], zero_initialize_workgroup_memory: false };
    let make = |vertex: &str, fragment: &str, depth_write: bool, blend: Option<BlendState>| {
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("bo2mp_guest_view"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some(vertex),
                buffers: &buffers,
                compilation_options: options.clone(),
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some(fragment),
                targets: &[Some(ColorTargetState { format, blend, write_mask: ColorWrites::ALL })],
                compilation_options: options.clone(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: SCENE_DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState { count: samples, ..default() },
            multiview_mask: None,
            cache: None,
        })
    };
    let add = BlendState {
        color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
        alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
    };
    [
        make("held_vertex", "held_fragment", true, None),
        make("held_vertex", "glow_fragment", false, Some(add)),
        make("world_vertex", "held_fragment", true, None),
    ]
}
