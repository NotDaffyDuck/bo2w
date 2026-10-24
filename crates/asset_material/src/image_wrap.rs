//! Putting decoded texels into a bevy `Image`: which texture format reads
//! them, and which sampler goes beside them.


use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::image_texels::{Layout, Texels};

/// What an image is wrapped in, apart from its texels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Wrap {
    pub sampler_state: u8,
    pub is_normal: bool,
    /// A colour map of an alpha-tested material: sampled without blending.
    pub sharp: bool,
    /// Read without the sRGB curve whatever the image says.
    pub force_linear: bool,
    pub srgb_reads: bool,
}

impl Wrap {
    fn linear(self) -> bool {
        self.is_normal || self.force_linear || !self.srgb_reads
    }
}

fn texture_format(layout: Layout, linear: bool) -> TextureFormat {
    match (layout, linear) {
        (Layout::Rgba8, true) => TextureFormat::Rgba8Unorm,
        (Layout::Rgba8, false) => TextureFormat::Rgba8UnormSrgb,
        (Layout::Bc1, true) => TextureFormat::Bc1RgbaUnorm,
        (Layout::Bc1, false) => TextureFormat::Bc1RgbaUnormSrgb,
        (Layout::Bc2, true) => TextureFormat::Bc2RgbaUnorm,
        (Layout::Bc2, false) => TextureFormat::Bc2RgbaUnormSrgb,
        (Layout::Bc3, true) => TextureFormat::Bc3RgbaUnorm,
        (Layout::Bc3, false) => TextureFormat::Bc3RgbaUnormSrgb,
    }
}

/// The image over `texels`, which it takes ownership of.
pub(crate) fn build_image(texels: Texels, wrap: Wrap) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width: texels.width,
            height: texels.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture_format(texels.layout, wrap.linear()),
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(texels.bytes);
    image.sampler = ImageSampler::Descriptor(sampler_from_state(wrap.sampler_state, 1, wrap.sharp));
    image
}

/// The sampler a material's sampler-state byte asks for: bits 5 to 7 clamp
/// the u, v and w axes, bits 0 to 2 pick the filter and bits 3 and 4 the mip
/// filter. `sharp` samples with nearest filtering throughout.
pub fn sampler_from_state(sampler_state: u8, levels: u32, sharp: bool) -> ImageSamplerDescriptor {
    let address = |bit: u8| {
        if (sampler_state >> bit) & 1 != 0 {
            ImageAddressMode::ClampToEdge
        } else {
            ImageAddressMode::Repeat
        }
    };
    let anisotropy = match sampler_state & 0b111 {
        _ if levels <= 1 || sharp => 1,
        3 => 2,
        4 => 4,
        _ => 1,
    };
    let mip_linear = !sharp && (((sampler_state >> 3) & 0b11) == 2 || anisotropy > 1);
    let filter = if sharp {
        ImageFilterMode::Nearest
    } else {
        ImageFilterMode::Linear
    };
    ImageSamplerDescriptor {
        address_mode_u: address(5),
        address_mode_v: address(6),
        address_mode_w: address(7),
        mag_filter: filter,
        min_filter: filter,
        mipmap_filter: if mip_linear {
            ImageFilterMode::Linear
        } else {
            ImageFilterMode::Nearest
        },
        anisotropy_clamp: anisotropy,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_bits_pick_the_axes() {
        let sampler = sampler_from_state(0b0110_0000, 1, false);
        assert_eq!(sampler.address_mode_u, ImageAddressMode::ClampToEdge);
        assert_eq!(sampler.address_mode_v, ImageAddressMode::ClampToEdge);
        assert_eq!(sampler.address_mode_w, ImageAddressMode::Repeat);
    }

    #[test]
    fn sharp_samples_nearest_without_anisotropy() {
        let sampler = sampler_from_state(0b0001_0100, 9, true);
        assert_eq!(sampler.mag_filter, ImageFilterMode::Nearest);
        assert_eq!(sampler.mipmap_filter, ImageFilterMode::Nearest);
        assert_eq!(sampler.anisotropy_clamp, 1);
    }

    #[test]
    fn filter_codes_scale_anisotropy_on_mipped_images() {
        assert_eq!(sampler_from_state(3, 5, false).anisotropy_clamp, 2);
        assert_eq!(sampler_from_state(4, 5, false).anisotropy_clamp, 4);
        assert_eq!(sampler_from_state(4, 1, false).anisotropy_clamp, 1);
        assert_eq!(sampler_from_state(4, 5, false).mipmap_filter, ImageFilterMode::Linear);
    }

    #[test]
    fn colour_space_follows_the_wrap() {
        let wrap = |is_normal, force_linear, srgb_reads| Wrap {
            sampler_state: 0,
            is_normal,
            sharp: false,
            force_linear,
            srgb_reads,
        };
        assert!(!wrap(false, false, true).linear());
        assert!(wrap(true, false, true).linear());
        assert!(wrap(false, true, true).linear());
        assert!(wrap(false, false, false).linear());
        assert_eq!(texture_format(Layout::Bc3, false), TextureFormat::Bc3RgbaUnormSrgb);
    }
}
