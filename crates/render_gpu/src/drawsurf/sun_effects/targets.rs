use bevy::render::render_resource::{
    Extent3d, Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
    TextureView, TextureViewDescriptor, TexelCopyBufferLayout,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};

/// A one-texel texture and the view the sun passes bind it through.
pub(super) struct Texel {
    _texture: Texture,
    pub(super) view: TextureView,
}

const ONE: Extent3d = Extent3d {
    width: 1,
    height: 1,
    depth_or_array_layers: 1,
};

fn make(
    device: &RenderDevice,
    label: &'static str,
    format: TextureFormat,
    usage: TextureUsages,
) -> Texel {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: ONE,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    Texel {
        _texture: texture,
        view,
    }
}

/// Render target that holds the per-frame visibility state; two of them ping-pong.
pub(super) fn history_texel(device: &RenderDevice, label: &'static str) -> Texel {
    make(
        device,
        label,
        TextureFormat::Rgba16Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    )
}

/// Opaque white, bound wherever a pass has no real image to sample.
pub(super) fn white_texel(device: &RenderDevice, queue: &RenderQueue) -> Texel {
    let texel = make(
        device,
        "bo2_sun_white",
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    );
    queue.write_texture(
        texel._texture.as_image_copy(),
        &[255; 4],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(1),
        },
        ONE,
    );
    texel
}
