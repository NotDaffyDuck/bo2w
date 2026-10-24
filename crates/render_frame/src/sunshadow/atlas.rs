//! Atlas sizes. Cascades are stacked vertically in one column of the atlas.

/// How many cascades the sun renders.
pub const SUN_SHADOW_PARTITION_COUNT: u32 = 2;

/// Technique that draws casters into the shadow map.
pub const SUN_SHADOW_CASTER_TECH: u8 = lighting_t6::TECHNIQUE_SUN_SHADOW_CASTER;

/// Quality tiers of the shadow atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SunShadowAtlasProfile {
    Large,
    Small,
}

/// The tier every frame uses for now.
pub const SUN_SHADOW_FORCED_PROFILE: SunShadowAtlasProfile = SunShadowAtlasProfile::Large;

/// Where one cascade sits inside the atlas, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SunShadowViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl SunShadowAtlasProfile {
    pub const fn render_target_id(self) -> u8 {
        match self {
            Self::Large => 10,
            Self::Small => 11,
        }
    }

    /// Side of one square cascade, in texels.
    pub const fn partition_size(self) -> u32 {
        match self {
            Self::Large => 0x400,
            Self::Small => 0x200,
        }
    }

    pub const fn atlas_columns(self) -> u32 {
        match self {
            Self::Large => 2,
            Self::Small => 4,
        }
    }

    pub const fn atlas_width(self) -> u32 {
        self.partition_size() * self.atlas_columns()
    }

    /// The viewport of cascade `partition`, or `None` past the last cascade.
    pub const fn partition_viewport(self, partition: u32) -> Option<SunShadowViewport> {
        if partition >= SUN_SHADOW_PARTITION_COUNT {
            return None;
        }
        let side = self.partition_size();
        Some(SunShadowViewport { x: 0, y: partition * side, width: side, height: side })
    }
}
