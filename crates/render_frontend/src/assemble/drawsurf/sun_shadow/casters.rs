//! What casts into the sun's shadow map, and how a receiver samples it.

/// The technique a caster is drawn with in the sun cascades.
pub const SUN_SHADOW_CASTER_TECH: u8 = lighting_t6::TECHNIQUE_SUN_SHADOW_CASTER;

/// The static-model flag that keeps a model out of the sun's shadow map.
pub const STATIC_MODEL_FLAG_NO_CAST_SHADOW: u8 = lighting_t6::STATIC_MODEL_FLAG_NO_CAST_SHADOW;

/// Whether a static model with these flags is drawn into the sun cascades.
pub const fn smodel_casts_sun_shadow(flags: u8) -> bool {
    flags & STATIC_MODEL_FLAG_NO_CAST_SHADOW == 0
}

/// Names of materials that are mostly holes, so they cast through a cutout
/// rather than as solid.
const CUTOUT_NAME_PARTS: [&[u8]; 7] = [
    b"chainlink",
    b"chain_link",
    b"chain-link",
    b"fence",
    b"grate",
    b"grating",
    b"razorwire",
];

/// Whether a material name says it is a fence or grate: any of the parts
/// above, in any case.
pub fn sun_shadow_cutout_name(name: &str) -> bool {
    let name = name.as_bytes();
    CUTOUT_NAME_PARTS.iter().any(|part| {
        name.windows(part.len())
            .any(|window| window.eq_ignore_ascii_case(part))
    })
}

/// How a receiver reads the shadow map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SunShadowSamplingPath {
    /// Straight from the depth texture, compared in hardware.
    DepthTexture,

    /// A colour copy of the depth, compared in the shader.
    Fallback,
}

impl SunShadowSamplingPath {
    /// The code image sampler the receiver binds the map with.
    pub const fn code_image_sampler(self) -> u8 {
        match self {
            Self::DepthTexture => 0x62,
            Self::Fallback => 0x61,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_0_is_no_cast() {
        assert!(!smodel_casts_sun_shadow(0x1));
        assert!(!smodel_casts_sun_shadow(0x3));
        assert!(smodel_casts_sun_shadow(0x0));
        // Bit 1 is lightmap-lit, and 0x10 was another game's no-cast: both
        // still cast.
        assert!(smodel_casts_sun_shadow(0x2));
        assert!(smodel_casts_sun_shadow(0x10));
    }

    #[test]
    fn every_other_flag_bit_still_casts() {
        for bit in 1..8 {
            assert!(smodel_casts_sun_shadow(1 << bit), "bit {bit}");
        }
        assert!(smodel_casts_sun_shadow(0xFE));
        assert!(!smodel_casts_sun_shadow(0xFF));
    }

    #[test]
    fn fences_and_grates_are_cutouts_in_any_case() {
        for name in [
            "mtl_chainlink_01",
            "MTL_CHAIN_LINK",
            "wall-Chain-Link",
            "a_Fence_b",
            "floor_grate",
            "metal_GRATING_wet",
            "RazorWire",
        ] {
            assert!(sun_shadow_cutout_name(name), "{name}");
        }
    }

    #[test]
    fn other_names_are_solid() {
        for name in [
            "",
            "concrete_wall",
            "chain",
            "link",
            "wire",
            "grat",
            "mtl_brick",
        ] {
            assert!(!sun_shadow_cutout_name(name), "{name}");
        }
    }

    #[test]
    fn the_sampling_paths_bind_different_samplers() {
        assert_eq!(
            SunShadowSamplingPath::DepthTexture.code_image_sampler(),
            0x62
        );
        assert_eq!(SunShadowSamplingPath::Fallback.code_image_sampler(), 0x61);
    }

    #[test]
    fn the_caster_technique_is_the_lighting_one() {
        assert_eq!(
            SUN_SHADOW_CASTER_TECH,
            lighting_t6::TECHNIQUE_SUN_SHADOW_CASTER
        );
        assert_eq!(
            STATIC_MODEL_FLAG_NO_CAST_SHADOW,
            lighting_t6::STATIC_MODEL_FLAG_NO_CAST_SHADOW
        );
    }
}
