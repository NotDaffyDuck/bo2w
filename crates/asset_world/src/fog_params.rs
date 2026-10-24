//! The fog a map's createart file sets up.

use std::f32::consts::LN_2;

/// Distance fog, which thickens exponentially past `start_dist`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExpFog {
    /// Nothing nearer than this is fogged.
    pub start_dist: f32,
    /// The distance past the start at which the fog is half as thick as it
    /// will get.
    pub halfway_dist: f32,
    pub color_rgb: [f32; 3],
    /// The thickest the fog gets, from 0 (clear) to 1 (solid).
    pub max_opacity: f32,
    /// Seconds a change to this fog takes to fade in.
    pub transition_time: f32,
    /// A second fog, tinted toward the sun's side of the sky.
    pub sun: Option<SunFog>,
    /// Fog that thins with height.
    pub volumetric: Option<VolFog>,
}

/// Height fog.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolFog {
    /// Height above the base at which the fog has thinned to half.
    pub halfway_height: f32,
    pub base_height: f32,
    pub color_scale: f32,
}

/// The sun-side tint of a fog: looking toward the sun blends this colour in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunFog {
    pub color_rgb: [f32; 3],
    pub sun_dir: [f32; 3],
    /// Angle from the sun, in degrees, where the tint starts to fade out.
    pub begin_angle_deg: f32,
    /// Angle from the sun, in degrees, where the tint is gone.
    pub end_angle_deg: f32,
    pub scale: f32,
}

impl ExpFog {
    /// The exponent's rate: `ln 2 / halfway_dist`, or 0 for a fog with no
    /// positive halfway distance.
    pub fn density(&self) -> f32 {
        if self.halfway_dist > 0.0 {
            LN_2 / self.halfway_dist
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(halfway_dist: f32) -> ExpFog {
        ExpFog {
            start_dist: 0.0,
            halfway_dist,
            color_rgb: [0.5; 3],
            max_opacity: 1.0,
            transition_time: 0.0,
            sun: None,
            volumetric: None,
        }
    }

    #[test]
    fn density_halves_the_light_every_halfway_distance() {
        let rate = fog(200.0).density();
        assert!(((-rate * 200.0).exp() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_fog_with_no_halfway_distance_has_no_density() {
        assert_eq!(fog(0.0).density(), 0.0);
        assert_eq!(fog(-5.0).density(), 0.0);
    }
}
