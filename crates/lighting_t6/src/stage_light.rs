//! Turning scene lights into the numbers the shaders read: colour in linear
//! space, positions relative to the eye, and the spot cone.

pub const LAMP_KIND_PARALLEL: u8 = 1;
pub const CONE_LAMP: u8 = 2;
pub const GFX_LIGHT_TYPE_SPOT_SQUARE: u8 = 3;
pub const GFX_LIGHT_TYPE_SPOT_ROUND: u8 = 4;
pub const POINT_LAMP: u8 = 5;

/// The three spot shapes all behave as spots; only the cookie differs.
pub const fn is_cone_lamp(kind: u8) -> bool {
    matches!(
        kind,
        CONE_LAMP | GFX_LIGHT_TYPE_SPOT_SQUARE | GFX_LIGHT_TYPE_SPOT_ROUND
    )
}

/// Default for the colour-scale dvars.
pub const COLOR_GAIN_INITIAL: f32 = 1.0;

// Shader constant slots a light writes.
pub const ENGINE_CONST_LAMP_ORIGIN: u16 = 0;
pub const ENGINE_CONST_LAMP_DIFFUSE: u16 = 1;
pub const ENGINE_CONST_LAMP_GLOSS: u16 = 2;
pub const ENGINE_CONST_LAMP_AIM: u16 = 3;
pub const ENGINE_CONST_LAMP_CONE_TERMS: u16 = 4;
pub const ENGINE_CONST_LAMP_FALLOFF_ORIGIN: u16 = 5;

/// The sRGB curve as bit-exact single-precision numbers: below `knee` it is a
/// straight line of slope `slope` (about 1/12.92); above, the power curve
/// `((x + lift) * shrink) ^ power` with lift 0.055, shrink 1/1.055, power 2.4.
struct SrgbCurve {
    knee: f32,
    slope: f32,
    lift: f32,
    shrink: f32,
    power: f32,
}

const SRGB: SrgbCurve = SrgbCurve {
    knee: f32::from_bits(0x3d20_e411),
    slope: f32::from_bits(0x3d9e_8391),
    lift: f32::from_bits(0x3d61_47ae),
    shrink: f32::from_bits(0x3f72_a76f),
    power: f32::from_bits(0x4019_999a),
};

/// A colour channel from sRGB to linear light.
pub fn gamma_encoded_to_linear(x: f32) -> f32 {
    let c = &SRGB;
    if x <= c.knee {
        return x * c.slope;
    }
    libm::powf((x + c.lift) * c.shrink, c.power)
}

/// One light as the scene holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LampRecord {
    pub kind: u8,
    pub color: [f32; 3],
    pub direction: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub outer_cone_cosine: f32,
    pub inner_cone_cosine: f32,
    pub falloff: u8,
    /// Width of the falloff image, if the light has one.
    pub falloff_map_width: Option<u16>,
    /// Where in the falloff image this light's row starts.
    pub lightmap_first: i32,
}

impl LampRecord {
    /// A light of `light_type` at `origin` reaching `radius`, with no cone,
    /// no direction and no falloff image.
    const fn bare(kind: u8, spot: [f32; 3], reach: f32, tint: [f32; 3]) -> Self {
        Self {
            kind,
            origin: spot,
            radius: reach,
            color: tint,
            direction: [0.0; 3],
            inner_cone_cosine: 0.0,
            outer_cone_cosine: 0.0,
            falloff: 0,
            lightmap_first: 0,
            falloff_map_width: None,
        }
    }
}

/// A sun's "position" is its direction, with w = 0 to mark it as infinitely
/// far.
pub const fn sun_point_at_infinity(direction: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = direction;
    [x, y, z, 0.0]
}

/// A colour constant: each channel scaled, then taken to linear light; alpha 1.
fn linear_rgba(tint: [f32; 3], gain: f32) -> [f32; 4] {
    let mut out = [1.0; 4];
    for (slot, channel) in out.iter_mut().zip(tint) {
        *slot = gamma_encoded_to_linear(channel * gain);
    }
    out
}

/// The diffuse colour constant: linear light, alpha 1.
pub fn lamp_diffuse_rgba(rgb: [f32; 3], diffuse_gain: f32) -> [f32; 4] {
    linear_rgba(rgb, diffuse_gain)
}

/// The specular colour constant: linear light, alpha 1.
pub fn lamp_gloss_rgba(rgb: [f32; 3], shine_gain: f32) -> [f32; 4] {
    linear_rgba(rgb, shine_gain)
}

/// Why an explosion-style light was not added to the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointLampRefusal {
    NoWorld,
    RadiusZeroOrNegative,
    /// The scene already holds as many dynamic lights as it can.
    Cap,
}

/// The scene takes at most 32 dynamic lights.
pub(crate) const SCENE_DLIGHT_LIMIT: u32 = 32;

/// Whether one more omni light may join the scene, given how many are there.
/// The checks run in order: a world, then a reach, then room.
pub fn add_omni_light_to_scene_allows(
    level_present: bool,
    reach: f32,
    active_total: u32,
) -> Result<(), PointLampRefusal> {
    let checks = [
        (level_present, PointLampRefusal::NoWorld),
        (reach > 0.0, PointLampRefusal::RadiusZeroOrNegative),
        (active_total < SCENE_DLIGHT_LIMIT, PointLampRefusal::Cap),
    ];
    checks.into_iter().find(|&(passed, _)| !passed).map_or(Ok(()), |(_, why)| Err(why))
}

/// A point light with no cone and no falloff image.
pub fn pack_point_lamp(point: [f32; 3], reach: f32, tint_blue_first: [f32; 3]) -> LampRecord {
    LampRecord::bare(POINT_LAMP, point, reach, tint_blue_first)
}

/// How a spot effect's cone is shaped. The values are dvars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConeLightTuningVars {
    pub inner_reach: f32,
    pub finish_reach: f32,
    /// The inner cone's angle as a fraction of the outer one.
    pub cone_core_fraction: f32,
    pub brightness: f32,
    pub falloff: u8,
}

impl ConeLightTuningVars {
    pub const fn default_registrations() -> Self {
        let mut cone = Self { inner_reach: 0.0, finish_reach: 0.0, cone_core_fraction: 0.0, brightness: 0.0, falloff: 0 };
        (cone.inner_reach, cone.finish_reach) = (36.0, 196.0);
        (cone.cone_core_fraction, cone.brightness, cone.falloff) = (0.5, 14.0, 2);
        cone
    }
}

/// The least width a spot cone keeps, at either end of its range.
const CONE_GAP: f32 = 0.1;

/// The far radius, pulled back inside `(start, start + radius)` so the cone
/// has some width and does not run past the light's reach.
fn spot_end_radius(near_end: f32, far_end: f32, span: f32) -> f32 {
    let reach = near_end + span;
    let far_end = if far_end <= near_end { near_end + CONE_GAP } else { far_end };
    match reach <= far_end {
        true => reach - CONE_GAP,
        false => far_end,
    }
}

/// How far behind the effect the cone's apex sits, so that the cone is
/// `start` wide where the effect is.
fn spot_apex_offset(near_end: f32, far_end: f32, reach: f32) -> f32 {
    let widening = (far_end - near_end) / reach;
    near_end / widening
}

/// A spot light for an effect at `origin` shining along `forward`. The scene
/// stores the direction pointing back at the light, and the apex is moved
/// back so the cone is `inner_reach` wide at the effect.
pub fn spot_light_pack(
    spot: [f32; 3],
    ahead: [f32; 3],
    reach: f32,
    tint_blue_first: [f32; 3],
    cone: ConeLightTuningVars,
) -> LampRecord {
    let start = cone.inner_reach;
    let end = spot_end_radius(start, cone.finish_reach, reach);
    let back = spot_apex_offset(start, end, reach);
    let half_angle = libm::atanf((end - start) / reach);

    let toward_light = ahead.map(|axis| -axis);
    let apex = std::array::from_fn(|k| spot[k] + back * toward_light[k]);
    let tinted = tint_blue_first.map(|channel| channel * cone.brightness);
    LampRecord {
        direction: toward_light,
        outer_cone_cosine: libm::cosf(half_angle),
        inner_cone_cosine: libm::cosf(half_angle * cone.cone_core_fraction),
        falloff: cone.falloff,
        ..LampRecord::bare(CONE_LAMP, apex, reach + back, tinted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn spot_types_are_the_three_cone_shapes() {
        let spots = [CONE_LAMP, GFX_LIGHT_TYPE_SPOT_SQUARE, GFX_LIGHT_TYPE_SPOT_ROUND];
        assert!(spots.into_iter().all(is_cone_lamp));
        assert!([LAMP_KIND_PARALLEL, POINT_LAMP, 0].into_iter().all(|t| !is_cone_lamp(t)));
    }

    #[test]
    fn srgb_black_and_white_map_to_themselves() {
        assert_eq!(gamma_encoded_to_linear(0.0), 0.0);
        assert!(near(gamma_encoded_to_linear(1.0), 1.0));
    }

    #[test]
    fn srgb_dark_values_follow_the_straight_part() {
        assert!(near(gamma_encoded_to_linear(0.02), 0.02 / 12.92));
    }

    #[test]
    fn srgb_mid_grey_is_darker_in_linear_light() {
        assert!((gamma_encoded_to_linear(0.5) - 0.2140).abs() < 1e-3);
    }

    #[test]
    fn srgb_is_continuous_across_the_join() {
        let gap = gamma_encoded_to_linear(0.0405) - gamma_encoded_to_linear(0.0404);
        assert!(gap.abs() < 1e-4);
    }

    #[test]
    fn a_sun_position_is_its_direction_with_zero_w() {
        assert_eq!(sun_point_at_infinity([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn diffuse_and_specular_scale_before_the_curve() {
        let half = lamp_diffuse_rgba([0.5; 3], 1.0);
        assert_eq!(half, lamp_diffuse_rgba([0.25; 3], 2.0));
        assert_eq!(half[3], 1.0);
        assert_eq!(lamp_gloss_rgba([0.5; 3], 1.0), half);
    }

    #[test]
    fn the_scene_takes_thirty_two_dynamic_lights() {
        let room = |live| add_omni_light_to_scene_allows(true, 50.0, live);
        assert_eq!([room(0), room(31)], [Ok(()), Ok(())]);
        assert_eq!(room(32), Err(PointLampRefusal::Cap));
    }

    #[test]
    fn a_light_with_no_world_or_no_reach_is_refused_first() {
        let cases = [
            ((false, 0.0, 99), PointLampRefusal::NoWorld),
            ((true, 0.0, 99), PointLampRefusal::RadiusZeroOrNegative),
            ((true, -3.0, 0), PointLampRefusal::RadiusZeroOrNegative),
        ];
        for ((world, radius, live), why) in cases {
            assert_eq!(add_omni_light_to_scene_allows(world, radius, live), Err(why));
        }
    }

    #[test]
    fn an_omni_pack_has_no_cone() {
        let lamp = pack_point_lamp([1.0, 2.0, 3.0], 64.0, [0.1, 0.2, 0.3]);
        assert_eq!((lamp.kind, lamp.radius), (POINT_LAMP, 64.0));
        assert_eq!((lamp.origin, lamp.color), ([1.0, 2.0, 3.0], [0.1, 0.2, 0.3]));
        assert_eq!((lamp.direction, lamp.falloff_map_width), ([0.0; 3], None));
    }

    #[test]
    fn the_default_cone_matches_the_registered_dvars() {
        let c = ConeLightTuningVars::default_registrations();
        assert_eq!((c.inner_reach, c.finish_reach, c.cone_core_fraction), (36.0, 196.0, 0.5));
        assert_eq!((c.brightness, c.falloff), (14.0, 2));
    }

    #[test]
    fn the_far_radius_is_kept_when_it_is_in_range() {
        assert_eq!(spot_end_radius(36.0, 196.0, 300.0), 196.0);
    }

    #[test]
    fn a_far_radius_inside_the_start_is_pushed_out() {
        assert!([10.0, 36.0].into_iter().all(|end| near(spot_end_radius(36.0, end, 300.0), 36.1)));
    }

    #[test]
    fn a_far_radius_past_the_reach_is_pulled_in() {
        assert!(near(spot_end_radius(36.0, 400.0, 100.0), 135.9));
    }

    #[test]
    fn the_apex_sits_behind_the_effect() {
        // Slope (196 - 36) / 320 = 0.5, so a 36-wide start is 72 back.
        assert!(near(spot_apex_offset(36.0, 196.0, 320.0), 72.0));
    }

    #[test]
    fn a_spot_pack_points_back_and_moves_its_apex() {
        let cone = ConeLightTuningVars::default_registrations();
        let lamp = spot_light_pack([0.0, 0.0, 100.0], [0.0, 0.0, 1.0], 320.0, [1.0, 0.5, 0.0], cone);
        assert_eq!((lamp.kind, lamp.direction), (CONE_LAMP, [0.0, 0.0, -1.0]));
        assert!(near(lamp.origin[2], 28.0) && near(lamp.radius, 392.0));
        assert_eq!((lamp.color, lamp.falloff), ([14.0, 7.0, 0.0], 2));
    }

    #[test]
    fn a_spot_pack_inner_cone_is_narrower_than_the_outer() {
        let cone = ConeLightTuningVars::default_registrations();
        let light = spot_light_pack([0.0; 3], [1.0, 0.0, 0.0], 320.0, [1.0; 3], cone);
        // atan(0.5) is about 26.57 degrees.
        let outer = libm::atanf(0.5);
        assert!(near(light.outer_cone_cosine, libm::cosf(outer)));
        assert!(light.inner_cone_cosine > light.outer_cone_cosine);
        assert!(near(light.inner_cone_cosine, libm::cosf(outer * 0.5)));
    }
}
