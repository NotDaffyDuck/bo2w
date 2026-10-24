//! The baked lightmap images of a world, and the reasons one can be missing.

use std::fmt;

use bevy::prelude::{Image, UVec2};

/// One lightmap page: the three baked layers, then the images that stand in
/// for ambient light, directional light and the sun mask.
#[derive(Clone, Debug)]
pub struct WorldLightmap {
    pub primary_image: Option<Image>,
    pub secondary_image: Option<Image>,
    pub secondary_b_image: Option<Image>,
    pub ambient_image: Image,
    pub directional_image: Image,
    pub sun_mask_image: Image,
    pub ambient_source_name: String,
    pub sun_mask_source_name: String,
    pub ambient_size: UVec2,
    pub sun_mask_size: UVec2,
}

/// Why a world has no usable lightmap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldLightmapGap {
    Missing,
    MultiplePages(usize),
    MissingPrimary,
    MissingSecondary,
    MissingSecondaryB,
    UnsupportedPrimaryFormat(u32),
    UnsupportedSecondaryFormat(u32),
    UnsupportedSecondaryBFormat(u32),
    InvalidDimensions { width: u16, height: u16, depth: u16 },
    MismatchedAspectRatio { primary: UVec2, secondary: UVec2 },
    Truncated { needed: usize, have: usize },
}

impl fmt::Display for WorldLightmapGap {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        use WorldLightmapGap::*;
        match self {
            Missing => out.write_str("world has no baked lightmap"),
            MultiplePages(count) => write!(
                out,
                "world has {count} lightmaps; none decoded (page walk failed)"
            ),
            MissingPrimary => out.write_str("lightmap primary image was not retained"),
            MissingSecondary => out.write_str("lightmap secondary image was not retained"),
            MissingSecondaryB => out.write_str("lightmap secondaryB image was not retained"),
            UnsupportedPrimaryFormat(format) => {
                write!(out, "lightmap primary D3D format {format} is unsupported")
            }
            UnsupportedSecondaryFormat(format) => {
                write!(out, "lightmap secondary D3D format {format} is unsupported")
            }
            UnsupportedSecondaryBFormat(format) => {
                write!(out, "lightmap secondaryB D3D format {format} is unsupported")
            }
            InvalidDimensions {
                width,
                height,
                depth,
            } => write!(out, "invalid lightmap dimensions {width}x{height}x{depth}"),
            MismatchedAspectRatio { primary, secondary } => write!(
                out,
                "lightmap primary {}x{} and secondary page {}x{} have different aspect ratios",
                primary.x, primary.y, secondary.x, secondary.y
            ),
            Truncated { needed, have } => write!(
                out,
                "lightmap payload is truncated: need {needed}, have {have}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_say_what_is_wrong() {
        assert_eq!(
            WorldLightmapGap::Missing.to_string(),
            "world has no baked lightmap"
        );
        let size = WorldLightmapGap::InvalidDimensions {
            width: 4,
            height: 8,
            depth: 0,
        };
        assert_eq!(size.to_string(), "invalid lightmap dimensions 4x8x0");
        let cut = WorldLightmapGap::Truncated { needed: 9, have: 3 };
        assert!(cut.to_string().ends_with("need 9, have 3"));
    }
}
