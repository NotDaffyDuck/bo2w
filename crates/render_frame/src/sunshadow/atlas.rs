//! Atlas sizes. Cascades are stacked vertically in one column of the atlas.

/// How many cascades the sun renders.
pub const SUNLIGHT_CASCADE_TOTAL: u32 = 2;

/// Technique that draws casters into the shadow map.
pub const SUN_CASTER_TECH: u8 = lighting_t6::TECHNIQUE_SUN_SHADOW_CASTER;

/// Quality tiers of the shadow atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaylightShadowAtlasProfile {
    Large,
    Small,
}

/// The tier every frame uses for now.
pub const DAYLIGHT_SHADOW_FORCED_PROFILE: DaylightShadowAtlasProfile = DaylightShadowAtlasProfile::Large;

/// Where one cascade sits inside the atlas, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunlightShadowRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl DaylightShadowAtlasProfile {
    pub const fn target_surface_code(self) -> u8 {
        match self {
            Self::Large => 10,
            Self::Small => 11,
        }
    }

    /// Side of one square cascade, in texels.
    pub const fn split_size(self) -> u32 {
        match self {
            Self::Large => 0x400,
            Self::Small => 0x200,
        }
    }

    pub const fn atlas_tiles_across(self) -> u32 {
        match self {
            Self::Large => 2,
            Self::Small => 4,
        }
    }

    pub const fn atlas_px_width(self) -> u32 {
        self.split_size() * self.atlas_tiles_across()
    }

    /// The viewport of cascade `partition`, or `None` past the last cascade.
    pub const fn slice_rect(self, partition: u32) -> Option<SunlightShadowRegion> {
        if partition >= SUNLIGHT_CASCADE_TOTAL {
            return None;
        }
        let side = self.split_size();
        Some(SunlightShadowRegion { x: 0, y: partition * side, width: side, height: side })
    }
}
