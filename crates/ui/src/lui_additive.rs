//! bo2mp scope lane: BO2's additive HUD pictures (blend ONE/ONE or
//! SRC_ALPHA/ONE: the bottom-left score panel's lines and glows, the
//! faction emblems' glow) drawn the way the game draws them: the picture's
//! colour is added onto what is under it. Bevy's UI only alpha-blends, which
//! darkens the scene under the lines instead of lighting it.

use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendState, RenderPipelineDescriptor,
};
use bevy::shader::ShaderRef;

const SHADER_SOURCE_PATH: &str = "embedded://ui/lui_additive.wgsl";

/// One additive picture and its tint (linear, alpha = fade).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct AdditiveUi {
    #[uniform(0)]
    pub tint: LinearRgba,
    #[texture(1)]
    #[sampler(2)]
    pub picture: Handle<Image>,
    /// The part of the picture the node shows: offset xy, size zw (0..1).
    #[uniform(3)]
    pub uv: Vec4,
    /// 1 when drawn into an encoded (gamma-space) main texture: the colour is
    /// added as it is (BO2's ONE/ONE); 0 for the straight-alpha overlay target.
    #[uniform(4)]
    pub encoded: f32,
    /// bo2mp vehicle screens: 1 for a picture packed as (AddMap colour,
    /// colour map alpha): BO2's `sw4_2d_color_add` pixel shader, `rgb =
    /// (ColorMap.a * colour.rgb + AddMap.rgb) * colour.a` (the VTOL Warship's
    /// reticles).
    #[uniform(5)]
    pub color_add: f32,
}

impl UiMaterial for AdditiveUi {
    fn fragment_shader() -> ShaderRef {
        SHADER_SOURCE_PATH.into()
    }

    fn specialize(descriptor: &mut RenderPipelineDescriptor, _key: UiMaterialKey<Self>) {
        // Replace's sum, keeping what is already there too: plain addition.
        let additive = BlendComponent { dst_factor: BlendFactor::One, ..BlendComponent::REPLACE };
        if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.first_mut()).and_then(|t| t.as_mut()) {
            target.blend = Some(BlendState { color: additive, alpha: additive });
        }
    }
}

pub(crate) fn attach(host: &mut App) {
    bevy::asset::embedded_asset!(host, "lui_additive.wgsl");
    host.add_plugins(UiMaterialPlugin::<AdditiveUi>::default());
}
