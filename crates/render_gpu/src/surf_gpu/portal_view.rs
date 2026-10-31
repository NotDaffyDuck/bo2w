//! Seeing through a linked portal (Portal's look): for each end he is in
//! front of (`render_frame::portal_eyes`), a second view of the map from
//! the eye behind the other end, drawn into its own picture before his
//! view; the portal's oval (`guest_view`) shows that picture. The views
//! live only in the render world, so nothing that counts his camera sees
//! them.
use bevy::camera::{Camera3d, CameraMainTextureUsages, ManualTextureViewHandle, NormalizedRenderTarget};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::camera::{ExtractedCamera, sort_cameras};
use bevy::render::render_resource::{
    Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::TemporaryRenderEntity;
use bevy::render::texture::{ManualTextureView, ManualTextureViews};
use bevy::render::view::{ExtractedView, Msaa, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderSystems};

use render_frame::portal_eyes::{self, oblique};

/// A view through a portal: `slot` 0 looks into the blue end, 1 the orange.
#[derive(Component, Clone, Copy)]
pub struct PortalCamera {
    pub bay: usize,
}

/// The pictures the views draw into, his view's size. The texture is
/// sRGB, so the copy out of the view stores what his view would; read
/// back as plain bytes it is what his view's shaders write.
#[derive(Resource, Default)]
pub struct PortalTargets {
    size: UVec2,
    slots: Vec<(Texture, TextureView, TextureView)>,
}

impl PortalTargets {
    /// Slot `slot`'s picture, to read.
    pub fn sampled(&self, bay: usize) -> Option<&TextureView> {
        self.slots.get(bay).map(|(_, _, sampled)| sampled)
    }
}

/// This frame's looks, for the portal's oval: which slots were drawn and
/// the pair (shooter) they belong to.
#[derive(Resource, Default, Clone, Copy)]
pub struct PortalLooksNow {
    pub active: [bool; 2],
    pub pair: u32,
}

const HANDLE_BASE: u32 = 0xB020;

pub(super) fn register(host: &mut App) {
    let Some(render_sub) = host.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_sub
        .init_resource::<PortalTargets>()
        .init_resource::<PortalLooksNow>()
        .add_systems(
            Render,
            spawn_portal_cameras.in_set(RenderSystems::CreateViews).before(sort_cameras),
        );
}

fn make_targets(gpu_dev: &RenderDevice, size: UVec2) -> Vec<(Texture, TextureView, TextureView)> {
    (0..2)
        .map(|_| {
            let texture = gpu_dev.create_texture(&TextureDescriptor {
                label: Some("bo2mp_portal_view"),
                size: Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8UnormSrgb,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[TextureFormat::Rgba8Unorm],
            });
            let drawn = texture.create_view(&TextureViewDescriptor::default());
            let sampled = texture.create_view(&TextureViewDescriptor {
                format: Some(TextureFormat::Rgba8Unorm),
                ..default()
            });
            (texture, drawn, sampled)
        })
        .collect()
}

fn spawn_portal_cameras(world: &mut World) {
    let looks = portal_eyes::looks();
    let mut now = PortalLooksNow::default();
    let main = {
        let mut cameras = world.query_filtered::<
            (Entity, &ExtractedCamera, &ExtractedView, &Camera3d, &Msaa, &CameraMainTextureUsages),
            Without<PortalCamera>,
        >();
        cameras
            .iter(world)
            .filter(|(_, cam, ..)| cam.physical_target_size.is_some())
            .max_by_key(|(_, camera, ..)| camera.order)
            .map(|(ent, camera, cam_view, settings, aa_mode, usages)| {
                (
                    ent,
                    clone_camera(camera),
                    (
                        cam_view.retained_view_entity,
                        cam_view.clip_from_view,
                        cam_view.world_from_view,
                        cam_view.target_format,
                        cam_view.viewport,
                        cam_view.color_grading.clone(),
                    ),
                    settings.clone(),
                    *aa_mode,
                    usages.clone(),
                )
            })
    };
    let Some(cam_parts) = main.filter(|_| !looks.is_empty()) else {
        world.insert_resource(now);
        return;
    };
    let (_, camera, view, settings, msaa, usages) = cam_parts;
    let size = camera.physical_target_size.unwrap_or(UVec2::ONE).max(UVec2::ONE);
    {
        let gpu_dev = world.resource::<RenderDevice>().clone();
        let mut targets = world.resource_mut::<PortalTargets>();
        if targets.size != size || targets.slots.len() < 2 {
            targets.slots = make_targets(&gpu_dev, size);
            targets.size = size;
        }
    }
    let drawn: Vec<TextureView> = world.resource::<PortalTargets>().slots.iter().map(|s| s.1.clone()).collect();
    let (kept, view_to_clip, world_from_view, target_format, viewport, grading) = view;
    let eye = world_from_view.to_matrix();
    for look in &looks {
        let handle = ManualTextureViewHandle(HANDLE_BASE + look.bay as u32);
        world.resource_mut::<ManualTextureViews>().insert(
            handle,
            ManualTextureView { texture_view: drawn[look.bay].clone(), size, view_format: TextureFormat::Rgba8UnormSrgb },
        );
        let virtual_eye = look.through * eye;
        // The exit's wall in the virtual eye's view space.
        let plane = virtual_eye.transpose() * look.exit_plane;
        let mut portal_camera = clone_camera(&camera);
        portal_camera.target = Some(NormalizedRenderTarget::TextureView(handle));
        portal_camera.order = camera.order - 2 + look.bay as isize;
        world.spawn((
            TemporaryRenderEntity,
            PortalCamera { bay: look.bay },
            portal_camera,
            ExtractedView {
                retained_view_entity: RetainedViewEntity::new(
                    kept.main_entity,
                    None,
                    100 + look.bay as u32,
                ),
                clip_from_view: oblique(view_to_clip, plane),
                world_from_view: GlobalTransform::from(Affine3A::from_mat4(virtual_eye)),
                clip_from_world: None,
                target_format,
                viewport,
                color_grading: grading.clone(),
                invert_culling: false,
            },
            settings.clone(),
            msaa,
            usages.clone(),
        ));
        now.active[look.bay] = true;
        now.pair = look.pair;
    }
    world.insert_resource(now);
}

fn clone_camera(camera: &ExtractedCamera) -> ExtractedCamera {
    ExtractedCamera {
        target: camera.target.clone(),
        physical_viewport_size: camera.physical_viewport_size,
        physical_target_size: camera.physical_target_size,
        viewport: camera.viewport.clone(),
        schedule: camera.schedule,
        order: camera.order,
        output_mode: camera.output_mode,
        msaa_writeback: camera.msaa_writeback,
        clear_color: camera.clear_color,
        sorted_camera_index_for_target: 0,
        exposure: camera.exposure,
        hdr: camera.hdr,
        compositing_space: camera.compositing_space,
    }
}
