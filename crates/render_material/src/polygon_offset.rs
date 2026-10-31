//! Polygon offset: the level a draw state carries in bits 4-5 of its second
//! state word, turned into a depth bias. Levels 1 and 2 scale the world pair,
//! level 3 selects the shadow-map pair; the default values are a stand-in
//! until BO2's own are known.

use std::ops::RangeInclusive;

const LEVEL_MASK: u32 = 0x30;
const LEVEL_SHIFT: u32 = 4;
const SHADOW_MAP_LEVEL: u32 = 3;

/// Depth-bias units of the host depth format per unit of the bias value.
const BIAS_UNIT: f64 = 1.0 / 65_536.0;

/// One slope-scale / bias pair with the range its tuning values may take.
#[derive(Clone, Debug, PartialEq)]
pub struct OffsetPair {
    pub slope: f32,
    pub bias: f32,
    pub slope_range: RangeInclusive<f32>,
    pub bias_range: RangeInclusive<f32>,
}

impl OffsetPair {
    const WORLD: Self = Self {
        slope: -1.0,
        bias: -1.0,
        slope_range: -4.0..=0.0,
        bias_range: -16.0..=0.0,
    };

    const SHADOW_MAP: Self = Self {
        slope: 2.0,
        bias: 0.125,
        slope_range: 0.0..=8.0,
        bias_range: 0.0..=32.0,
    };
}

/// Both pairs a level can draw from.
#[derive(Clone, Debug, PartialEq)]
pub struct PolygonOffsetTuning {
    pub world: OffsetPair,
    pub shadow_map: OffsetPair,
}

impl Default for PolygonOffsetTuning {
    fn default() -> Self {
        Self {
            world: OffsetPair::WORLD,
            shadow_map: OffsetPair::SHADOW_MAP,
        }
    }
}

impl PolygonOffsetTuning {
    /// The (slope scale, bias) a draw of `level` uses, in D3D units.
    pub fn d3d(&self, level: u32) -> (f32, f32) {
        match level {
            0 => (0.0, 0.0),
            SHADOW_MAP_LEVEL => {
                let pair = &self.shadow_map;
                (pair.slope, scale_bias(1.0, pair.bias))
            }
            _ => {
                let steps = level as f32;
                (steps * self.world.slope, scale_bias(steps, self.world.bias))
            }
        }
    }
}

fn scale_bias(steps: f32, bias: f32) -> f32 {
    (f64::from(steps) * f64::from(bias) * BIAS_UNIT) as f32
}

/// The shadow-map defaults, for callers that build their own projection.
pub const SM_POLYGON_OFFSET_SCALE_DEFAULT: f32 = OffsetPair::SHADOW_MAP.slope;
pub const SM_POLYGON_OFFSET_BIAS_DEFAULT: f32 = OffsetPair::SHADOW_MAP.bias;

/// The offset level of a draw state's second word.
#[inline]
pub const fn polygon_offset_level(dword1: u32) -> u32 {
    (dword1 & LEVEL_MASK) >> LEVEL_SHIFT
}

/// A D3D depth bias as the host's integer constant bias: the 24-bit depth
/// step is the unit, and halves round away from zero.
pub fn legacy_depth_bias_to_gpu_constant(bias: f32) -> i32 {
    const STEPS_PER_UNIT: f32 = 8_388_608.0;
    let steps = bias * STEPS_PER_UNIT;
    (steps + 0.5_f32.copysign(steps)) as i32
}

/// (slope scale, constant bias) for `level` with the default tuning.
pub fn gpu_default_depth_slope_offsets(level: u32) -> (f32, i32) {
    let (slope, bias) = PolygonOffsetTuning::default().d3d(level);
    (slope, legacy_depth_bias_to_gpu_constant(bias))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_bits_four_and_five() {
        assert_eq!(polygon_offset_level(0x00), 0);
        assert_eq!(polygon_offset_level(0x10), 1);
        assert_eq!(polygon_offset_level(0x2f), 2);
        assert_eq!(polygon_offset_level(0xffff_fff3), 3);
    }

    #[test]
    fn level_zero_draws_flat() {
        assert_eq!(PolygonOffsetTuning::default().d3d(0), (0.0, 0.0));
        assert_eq!(gpu_default_depth_slope_offsets(0), (0.0, 0));
    }

    #[test]
    fn world_levels_grow_with_the_level() {
        let tuning = PolygonOffsetTuning::default();
        let (slope1, bias1) = tuning.d3d(1);
        let (slope2, bias2) = tuning.d3d(2);
        assert_eq!(slope1, -1.0);
        assert_eq!(slope2, -2.0);
        assert!((bias2 - 2.0 * bias1).abs() < f32::EPSILON);
    }

    #[test]
    fn level_three_uses_the_shadow_map_pair() {
        let (slope, bias) = PolygonOffsetTuning::default().d3d(3);
        assert_eq!(slope, SM_POLYGON_OFFSET_SCALE_DEFAULT);
        assert_eq!(bias, (0.125_f64 / 65_536.0) as f32);
    }

    #[test]
    fn constant_bias_rounds_half_away_from_zero() {
        let half_step = 0.5 / 8_388_608.0;
        assert_eq!(legacy_depth_bias_to_gpu_constant(half_step), 1);
        assert_eq!(legacy_depth_bias_to_gpu_constant(-half_step), -1);
        assert_eq!(legacy_depth_bias_to_gpu_constant(0.0), 0);
    }
}
