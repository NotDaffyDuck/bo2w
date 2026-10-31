//! The baked lightmap images of a world, and the reasons one can be missing.

use std::fmt;

use bevy::prelude::{Image, UVec2};

/// One lightmap page: the three baked layers, then the images that stand in
/// for ambient light, directional light and the sun mask.
#[derive(Clone, Debug)]
pub struct LevelLightbake {
    pub first_slot_texture: Option<Image>,
    pub auxiliary_texture: Option<Image>,
    pub second_b_map: Option<Image>,
    pub background_light_texture: Image,
    pub heading_texture: Image,
    pub daylight_mask_image: Image,
    pub background_light_origin_label: String,
    pub daylight_mask_source_name: String,
    pub background_light_extent: UVec2,
    pub daylight_mask_size: UVec2,
}

/// Why a world has no usable lightmap. The zone walk only ever finds the
/// lightmap absent; every other way a page could fail is decided before the
/// pages reach this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LevelLightmapHole {
    Missing,
}

impl fmt::Display for LevelLightmapHole {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Missing = self;
        out.write_str("the map carries no baked light page")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_lightmap_says_so() {
        assert_eq!(LevelLightmapHole::Missing.to_string(), "the map carries no baked light page");
    }
}
