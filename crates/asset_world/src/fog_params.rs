//! The fog a map's createart file sets up.

use std::f32::consts::LN_2;

/// Distance fog, which thickens exponentially past `start_dist`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExpHaze {
    /// Nothing nearer than this is fogged.
    pub near_edge: f32,
    /// The distance past the start at which the fog is half as thick as it
    /// will get.
    pub half_density_at: f32,
    pub tint_rgb: [f32; 3],
    /// The thickest the fog gets, from 0 (clear) to 1 (solid).
    pub opacity_cap: f32,
    /// Seconds a change to this fog takes to fade in.
    pub fade_duration: f32,
    /// A second fog, tinted toward the sun's side of the sky.
    pub sun: Option<SunlitMist>,
    /// Fog that thins with height.
    pub volumetric: Option<VolumetricMist>,
}

/// Height fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumetricMist {
    /// Nothing reads a map's height fog yet, so only where it starts is kept.
    pub floor_elevation: f32,
}

/// The sun-side tint of a fog: looking toward the sun blends this colour in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunlitMist {
    pub tint_rgb: [f32; 3],
    pub toward_sun: [f32; 3],
    /// Angle from the sun, in degrees, where the tint starts to fade out.
    pub inner_cone_deg: f32,
    /// Angle from the sun, in degrees, where the tint is gone.
    pub outer_cone_deg: f32,
    pub scale: f32,
}

impl ExpHaze {
    /// The exponent's rate: `ln 2 / halfway_dist`, or 0 for a fog with no
    /// positive halfway distance.
    pub fn thickness_at(&self) -> f32 {
        if self.half_density_at > 0.0 {
            LN_2 / self.half_density_at
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(halfway_dist: f32) -> ExpHaze {
        ExpHaze {
            near_edge: 0.0,
            half_density_at: halfway_dist,
            tint_rgb: [0.5; 3],
            opacity_cap: 1.0,
            fade_duration: 0.0,
            sun: None,
            volumetric: None,
        }
    }

    #[test]
    fn density_halves_the_light_every_halfway_distance() {
        let rate = fog(200.0).thickness_at();
        assert!(((-rate * 200.0).exp() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_fog_with_no_halfway_distance_has_no_density() {
        assert_eq!(fog(0.0).thickness_at(), 0.0);
        assert_eq!(fog(-5.0).thickness_at(), 0.0);
    }
}
