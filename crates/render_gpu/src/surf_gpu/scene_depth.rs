//! The scene's depth+stencil target. Cameras that render into the same target at the same
//! sample count share one texture; a depth-only view of it is kept for effects that sample depth.

use std::collections::HashMap;

use bevy::camera::{Camera3d, Camera3dDepthLoadOp};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::TextureCache;
use bevy::render::view::{Msaa, ViewDepthTexture};

pub const SCENE_DEPTH_FORMAT: TextureFormat = TextureFormat::Depth24PlusStencil8;

/// A camera's depth target together with the view that reads only its depth.
#[derive(Component)]
pub struct SceneDepthTexture {
    attachment: ViewDepthTexture,
    depth_only: TextureView,
}

impl SceneDepthTexture {
    /// The depth aspect on its own, for sampling.
    pub fn view(&self) -> &TextureView {
        &self.depth_only
    }

    /// Depth and stencil both read-only, so the same pass can also sample `view()`
    /// (soft effect edges).
    pub fn read_only_attachment(&self) -> RenderPassDepthStencilAttachment<'_> {
        RenderPassDepthStencilAttachment {
            view: self.attachment.view(),
            depth_ops: None,
            stencil_ops: None,
        }
    }

    /// The writable attachment; the stencil clears exactly when the depth does.
    pub fn get_attachment(&self, store: StoreOp) -> RenderPassDepthStencilAttachment<'_> {
        let mut depth_att = self.attachment.get_attachment(store);
        let clearing = matches!(
            depth_att.depth_ops.as_ref().map(|ops| ops.load),
            Some(LoadOp::Clear(_))
        );
        depth_att.stencil_ops = Some(Operations {
            load: if clearing { LoadOp::Clear(0) } else { LoadOp::Load },
            store: StoreOp::Store,
        });
        depth_att
    }
}

fn depth_descriptor(px_w: u32, px_h: u32, sample_n: u32) -> TextureDescriptor<'static> {
    TextureDescriptor {
        label: Some("bo2_scene_depth"),
        size: Extent3d {
            width: px_w,
            height: px_h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: sample_n,
        dimension: TextureDimension::D2,
        format: SCENE_DEPTH_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    }
}

/// Gives every camera its depth target, one texture per (target, sample count) pair, and
/// forgets the depth-only views of textures no camera uses any more.
pub(crate) fn prepare_scene_depth(
    mut commands: Commands,
    gpu_dev: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    mut depth_views: Local<HashMap<TextureId, TextureView>>,
    cameras: Query<(Entity, &ExtractedCamera, &Camera3d, &Msaa)>,
) {
    let mut shared = HashMap::new();
    let mut alive = Vec::new();
    for (ent, cam, camera3d, aa_mode) in &cameras {
        let Some(size) = cam.physical_target_size else {
            continue;
        };
        let texture = shared
            .entry((cam.target.clone(), *aa_mode))
            .or_insert_with(|| {
                texture_cache.get(&gpu_dev, depth_descriptor(size.x, size.y, aa_mode.samples()))
            })
            .clone();
        let id = texture.texture.id();
        alive.push(id);
        let depth_only = depth_views
            .entry(id)
            .or_insert_with(|| {
                texture.texture.create_view(&TextureViewDescriptor {
                    aspect: TextureAspect::DepthOnly,
                    ..Default::default()
                })
            })
            .clone();
        let clear_depth = match camera3d.depth_load_op {
            Camera3dDepthLoadOp::Clear(value) => Some(value),
            Camera3dDepthLoadOp::Load => None,
        };
        commands.entity(ent).insert(SceneDepthTexture {
            depth_only,
            attachment: ViewDepthTexture::new(texture, clear_depth),
        });
    }
    depth_views.retain(|id, _| alive.contains(id));
}
