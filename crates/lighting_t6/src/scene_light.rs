//! Turning scene lights into the numbers the shaders read: colour in linear
//! space, positions relative to the eye, and the spot cone.

pub const GFX_LIGHT_TYPE_DIR: u8 = 1;
pub const GFX_LIGHT_TYPE_SPOT: u8 = 2;
pub const GFX_LIGHT_TYPE_SPOT_SQUARE: u8 = 3;
pub const GFX_LIGHT_TYPE_SPOT_ROUND: u8 = 4;
pub const GFX_LIGHT_TYPE_OMNI: u8 = 5;

/// The three spot shapes all behave as spots; only the cookie differs.
pub const fn gfx_light_is_spot(light_type: u8) -> bool {
    matches!(
        light_type,
        GFX_LIGHT_TYPE_SPOT | GFX_LIGHT_TYPE_SPOT_SQUARE | GFX_LIGHT_TYPE_SPOT_ROUND
    )
}

/// Default for the colour-scale dvars.
pub const R_COLOR_SCALE_DEFAULT: f32 = 1.0;

// Shader constant slots a light writes.
pub const CONST_SRC_CODE_LIGHT_POSITION: u16 = 0;
pub const CONST_SRC_CODE_LIGHT_DIFFUSE: u16 = 1;
pub const CONST_SRC_CODE_LIGHT_SPECULAR: u16 = 2;
pub const CONST_SRC_CODE_LIGHT_SPOTDIR: u16 = 3;
pub const CONST_SRC_CODE_LIGHT_SPOTFACTORS: u16 = 4;
pub const CONST_SRC_CODE_LIGHT_FALLOFF_PLACEMENT: u16 = 5;

/// Where the straight-line part of the sRGB curve ends (about 0.0404).
const SRGB_LINEAR_LIMIT: f32 = f32::from_bits(0x3d20_e411);
/// Slope of that straight-line part (about 1/12.92).
const SRGB_LINEAR_SLOPE: f32 = f32::from_bits(0x3d9e_8391);
/// The curved part is `((x + 0.055) / 1.055) ^ 2.4`.
const SRGB_CURVE_OFFSET: f32 = f32::from_bits(0x3d61_47ae);
const SRGB_CURVE_INV_SCALE: f32 = f32::from_bits(0x3f72_a76f);
const SRGB_CURVE_EXPONENT: f32 = f32::from_bits(0x4019_999a);

/// A colour channel from sRGB to linear light.
pub fn color_srgb_to_linear(x: f32) -> f32 {
    if x > SRGB_LINEAR_LIMIT {
        libm::powf(
            (x + SRGB_CURVE_OFFSET) * SRGB_CURVE_INV_SCALE,
            SRGB_CURVE_EXPONENT,
        )
    } else {
        x * SRGB_LINEAR_SLOPE
    }
}

/// Falloff-image coordinates are in units of 1/512.
const FALLOFF_PLACEMENT_UNIT: f32 = 1.0 / 512.0;

/// One light as the scene holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GfxLightPack {
    pub light_type: u8,
    pub color: [f32; 3],
    pub direction: [f32; 3],
    pub origin: [f32; 3],
    pub radius: f32,
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub exponent: u8,
    /// Width of the falloff image, if the light has one.
    pub falloff_image_width: Option<u16>,
    /// Where in the falloff image this light's row starts.
    pub lmap_lookup_start: i32,
}

/// What to write into the shader constants for a light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadowableLightPack {
    /// Slot 0, or no such light: leave the constants as they are.
    Unchanged,
    Dir {
        position: [f32; 4],
        diffuse: [f32; 4],
        specular: [f32; 4],
    },
    Omni {
        position: [f32; 4],
        diffuse: [f32; 4],
        specular: [f32; 4],
        spot_dir: [f32; 4],
        falloff_placement: Option<[f32; 4]>,
    },
    Spot {
        position: [f32; 4],
        diffuse: [f32; 4],
        specular: [f32; 4],
        spot_dir: [f32; 4],
        spot_factors: [f32; 4],
        falloff_placement: Option<[f32; 4]>,
    },
}

/// A sun's "position" is its direction, with w = 0 to mark it as infinitely
/// far.
pub const fn dir_light_position(direction: [f32; 3]) -> [f32; 4] {
    [direction[0], direction[1], direction[2], 0.0]
}

/// Offset from the eye to the light, with the reciprocal radius in w.
fn eye_relative_position(origin: [f32; 3], eye: [f32; 3], radius: f32) -> [f32; 4] {
    [
        origin[0] - eye[0],
        origin[1] - eye[1],
        origin[2] - eye[2],
        1.0 / radius,
    ]
}

fn linear_color(color: [f32; 3], scale: f32) -> [f32; 4] {
    [
        color_srgb_to_linear(color[0] * scale),
        color_srgb_to_linear(color[1] * scale),
        color_srgb_to_linear(color[2] * scale),
        1.0,
    ]
}

/// The diffuse colour constant: linear light, alpha 1.
pub fn light_diffuse(color: [f32; 3], diffuse_color_scale: f32) -> [f32; 4] {
    linear_color(color, diffuse_color_scale)
}

/// The specular colour constant: linear light, alpha 1.
pub fn light_specular(color: [f32; 3], specular_color_scale: f32) -> [f32; 4] {
    linear_color(color, specular_color_scale)
}

/// The cone constant: `scale` and `bias` turn the cosine of the angle to the
/// axis into a 0..1 blend between the inner and outer edge; the exponent
/// shapes it.
fn spot_factors(cos_outer: f32, cos_inner: f32, exponent: u8) -> [f32; 4] {
    let scale = 1.0 / (cos_inner - cos_outer);
    [scale, -scale * cos_outer, f32::from(exponent), 0.0]
}

fn falloff_placement(image_width: u16, lmap_lookup_start: i32) -> [f32; 4] {
    [
        f32::from(image_width) * FALLOFF_PLACEMENT_UNIT,
        0.0,
        lmap_lookup_start as f32 * FALLOFF_PLACEMENT_UNIT,
        0.0,
    ]
}

/// Works out the constants for the light in slot `scene_light_index`. Slot 0
/// is the "no light" slot and, like a missing light, changes nothing.
pub fn pack_shadowable_light(
    scene_light_index: u8,
    light: Option<&GfxLightPack>,
    eye: [f32; 3],
    diffuse_color_scale: f32,
    specular_color_scale: f32,
) -> ShadowableLightPack {
    let Some(light) = light.filter(|_| scene_light_index != 0) else {
        return ShadowableLightPack::Unchanged;
    };
    let diffuse = light_diffuse(light.color, diffuse_color_scale);
    let specular = light_specular(light.color, specular_color_scale);
    if light.light_type == GFX_LIGHT_TYPE_DIR {
        return ShadowableLightPack::Dir {
            position: dir_light_position(light.direction),
            diffuse,
            specular,
        };
    }
    let position = eye_relative_position(light.origin, eye, light.radius);
    let spot_dir = [
        light.direction[0],
        light.direction[1],
        light.direction[2],
        0.0,
    ];
    let falloff = light
        .falloff_image_width
        .map(|width| falloff_placement(width, light.lmap_lookup_start));
    if gfx_light_is_spot(light.light_type) {
        ShadowableLightPack::Spot {
            position,
            diffuse,
            specular,
            spot_dir,
            spot_factors: spot_factors(light.cos_outer, light.cos_inner, light.exponent),
            falloff_placement: falloff,
        }
    } else {
        ShadowableLightPack::Omni {
            position,
            diffuse,
            specular,
            spot_dir,
            falloff_placement: falloff,
        }
    }
}

/// Why an explosion-style light was not added to the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddOmniLightRefuse {
    NoWorld,
    NonPositiveRadius,
    /// The scene already holds as many dynamic lights as it can.
    Cap,
}

/// The scene takes at most 32 dynamic lights.
pub(crate) const SCENE_DLIGHT_LIMIT: u32 = 32;

/// Whether one more omni light may join the scene, given how many are there.
pub fn add_omni_light_to_scene_allows(
    world_present: bool,
    radius: f32,
    live_count: u32,
) -> Result<(), AddOmniLightRefuse> {
    if !world_present {
        Err(AddOmniLightRefuse::NoWorld)
    } else if radius <= 0.0 {
        Err(AddOmniLightRefuse::NonPositiveRadius)
    } else if live_count >= SCENE_DLIGHT_LIMIT {
        Err(AddOmniLightRefuse::Cap)
    } else {
        Ok(())
    }
}

/// A point light with no cone and no falloff image.
pub fn omni_light_pack(origin: [f32; 3], radius: f32, color_bgr: [f32; 3]) -> GfxLightPack {
    GfxLightPack {
        light_type: GFX_LIGHT_TYPE_OMNI,
        color: color_bgr,
        direction: [0.0; 3],
        origin,
        radius,
        cos_outer: 0.0,
        cos_inner: 0.0,
        exponent: 0,
        falloff_image_width: None,
        lmap_lookup_start: 0,
    }
}

/// The cone is never allowed to collapse to a line.
const SPOT_CONE_MARGIN: f32 = 0.1;
/// The scene stores a spot's direction pointing back at the light.
const SPOT_DIR_SIGN: f32 = -1.0;

/// How a spot effect's cone is shaped. The values are dvars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLightConeDvars {
    pub start_radius: f32,
    pub end_radius: f32,
    /// The inner cone's angle as a fraction of the outer one.
    pub fov_inner_fraction: f32,
    pub brightness: f32,
    pub exponent: u8,
}

impl SpotLightConeDvars {
    pub const fn register_defaults() -> Self {
        Self {
            start_radius: 36.0,
            end_radius: 196.0,
            fov_inner_fraction: 0.5,
            brightness: 14.0,
            exponent: 2,
        }
    }
}

/// The far radius, pulled back inside `(start, start + radius)` so the cone
/// has some width and does not run past the light's reach.
fn spot_end_radius(start: f32, end: f32, radius: f32) -> f32 {
    let mut end = end;
    if end <= start {
        end = start + SPOT_CONE_MARGIN;
    }
    if start + radius <= end {
        end = start + radius - SPOT_CONE_MARGIN;
    }
    end
}

/// How far behind the effect the cone's apex sits, so that the cone is
/// `start` wide where the effect is.
fn spot_apex_offset(start: f32, end: f32, radius: f32) -> f32 {
    start / ((end - start) / radius)
}

/// A spot light for an effect at `origin` shining along `forward`.
pub fn spot_light_pack(
    origin: [f32; 3],
    forward: [f32; 3],
    radius: f32,
    color_bgr: [f32; 3],
    cone: SpotLightConeDvars,
) -> GfxLightPack {
    let end = spot_end_radius(cone.start_radius, cone.end_radius, radius);
    let offset = spot_apex_offset(cone.start_radius, end, radius);
    let direction = [
        forward[0] * SPOT_DIR_SIGN,
        forward[1] * SPOT_DIR_SIGN,
        forward[2] * SPOT_DIR_SIGN,
    ];
    let apex = [
        origin[0] + offset * direction[0],
        origin[1] + offset * direction[1],
        origin[2] + offset * direction[2],
    ];
    let outer = libm::atanf((end - cone.start_radius) / radius);
    let inner = outer * cone.fov_inner_fraction;
    GfxLightPack {
        light_type: GFX_LIGHT_TYPE_SPOT,
        color: [
            color_bgr[0] * cone.brightness,
            color_bgr[1] * cone.brightness,
            color_bgr[2] * cone.brightness,
        ],
        direction,
        origin: apex,
        radius: radius + offset,
        cos_outer: libm::cosf(outer),
        cos_inner: libm::cosf(inner),
        exponent: cone.exponent,
        falloff_image_width: None,
        lmap_lookup_start: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn sample_light(light_type: u8) -> GfxLightPack {
        GfxLightPack {
            light_type,
            color: [0.5, 0.25, 0.0],
            direction: [0.0, 0.0, 1.0],
            origin: [10.0, 20.0, 30.0],
            radius: 100.0,
            cos_outer: 0.5,
            cos_inner: 0.9,
            exponent: 3,
            falloff_image_width: None,
            lmap_lookup_start: 0,
        }
    }

    #[test]
    fn spot_types_are_the_three_cone_shapes() {
        assert!(gfx_light_is_spot(GFX_LIGHT_TYPE_SPOT));
        assert!(gfx_light_is_spot(GFX_LIGHT_TYPE_SPOT_SQUARE));
        assert!(gfx_light_is_spot(GFX_LIGHT_TYPE_SPOT_ROUND));
        assert!(!gfx_light_is_spot(GFX_LIGHT_TYPE_DIR));
        assert!(!gfx_light_is_spot(GFX_LIGHT_TYPE_OMNI));
        assert!(!gfx_light_is_spot(0));
    }

    #[test]
    fn srgb_black_and_white_map_to_themselves() {
        assert_eq!(color_srgb_to_linear(0.0), 0.0);
        assert!(near(color_srgb_to_linear(1.0), 1.0));
    }

    #[test]
    fn srgb_dark_values_follow_the_straight_part() {
        assert!(near(color_srgb_to_linear(0.02), 0.02 / 12.92));
    }

    #[test]
    fn srgb_mid_grey_is_darker_in_linear_light() {
        assert!((color_srgb_to_linear(0.5) - 0.2140).abs() < 1e-3);
    }

    #[test]
    fn srgb_is_continuous_across_the_join() {
        let below = color_srgb_to_linear(0.0404);
        let above = color_srgb_to_linear(0.0405);
        assert!((above - below).abs() < 1e-4);
    }

    #[test]
    fn a_sun_position_is_its_direction_with_zero_w() {
        assert_eq!(dir_light_position([1.0, 2.0, 3.0]), [1.0, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn diffuse_and_specular_scale_before_the_curve() {
        let half = light_diffuse([0.5, 0.5, 0.5], 1.0);
        let scaled = light_diffuse([0.25, 0.25, 0.25], 2.0);
        assert_eq!(half, scaled);
        assert_eq!(half[3], 1.0);
        assert_eq!(light_specular([0.5, 0.5, 0.5], 1.0), half);
    }

    #[test]
    fn slot_zero_changes_nothing() {
        let light = sample_light(GFX_LIGHT_TYPE_OMNI);
        assert_eq!(
            pack_shadowable_light(0, Some(&light), [0.0; 3], 1.0, 1.0),
            ShadowableLightPack::Unchanged
        );
    }

    #[test]
    fn a_missing_light_changes_nothing() {
        assert_eq!(
            pack_shadowable_light(3, None, [0.0; 3], 1.0, 1.0),
            ShadowableLightPack::Unchanged
        );
    }

    #[test]
    fn a_sun_packs_its_direction_and_colours() {
        let light = sample_light(GFX_LIGHT_TYPE_DIR);
        let packed = pack_shadowable_light(1, Some(&light), [5.0; 3], 1.0, 1.0);
        let ShadowableLightPack::Dir {
            position,
            diffuse,
            specular,
        } = packed
        else {
            panic!("expected a sun, got {packed:?}");
        };
        assert_eq!(position, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(diffuse, light_diffuse(light.color, 1.0));
        assert_eq!(specular, light_specular(light.color, 1.0));
    }

    #[test]
    fn an_omni_is_placed_relative_to_the_eye() {
        let light = sample_light(GFX_LIGHT_TYPE_OMNI);
        let packed = pack_shadowable_light(2, Some(&light), [4.0, 5.0, 6.0], 1.0, 1.0);
        let ShadowableLightPack::Omni {
            position,
            falloff_placement,
            ..
        } = packed
        else {
            panic!("expected an omni, got {packed:?}");
        };
        assert_eq!(position, [6.0, 15.0, 24.0, 0.01]);
        assert_eq!(falloff_placement, None);
    }

    #[test]
    fn a_falloff_image_gives_a_placement() {
        let mut light = sample_light(GFX_LIGHT_TYPE_OMNI);
        light.falloff_image_width = Some(256);
        light.lmap_lookup_start = 128;
        let packed = pack_shadowable_light(2, Some(&light), [0.0; 3], 1.0, 1.0);
        let ShadowableLightPack::Omni {
            falloff_placement, ..
        } = packed
        else {
            panic!("expected an omni, got {packed:?}");
        };
        assert_eq!(falloff_placement, Some([0.5, 0.0, 0.25, 0.0]));
    }

    #[test]
    fn a_spot_adds_cone_factors() {
        let light = sample_light(GFX_LIGHT_TYPE_SPOT_ROUND);
        let packed = pack_shadowable_light(2, Some(&light), [0.0; 3], 1.0, 1.0);
        let ShadowableLightPack::Spot {
            spot_dir,
            spot_factors,
            ..
        } = packed
        else {
            panic!("expected a spot, got {packed:?}");
        };
        assert_eq!(spot_dir, [0.0, 0.0, 1.0, 0.0]);
        assert!(near(spot_factors[0], 2.5));
        assert!(near(spot_factors[1], -1.25));
        assert_eq!(spot_factors[2], 3.0);
        assert_eq!(spot_factors[3], 0.0);
    }

    #[test]
    fn the_scene_takes_thirty_two_dynamic_lights() {
        assert_eq!(add_omni_light_to_scene_allows(true, 50.0, 0), Ok(()));
        assert_eq!(add_omni_light_to_scene_allows(true, 50.0, 31), Ok(()));
        assert_eq!(
            add_omni_light_to_scene_allows(true, 50.0, 32),
            Err(AddOmniLightRefuse::Cap)
        );
    }

    #[test]
    fn a_light_with_no_world_or_no_reach_is_refused_first() {
        assert_eq!(
            add_omni_light_to_scene_allows(false, 0.0, 99),
            Err(AddOmniLightRefuse::NoWorld)
        );
        assert_eq!(
            add_omni_light_to_scene_allows(true, 0.0, 99),
            Err(AddOmniLightRefuse::NonPositiveRadius)
        );
        assert_eq!(
            add_omni_light_to_scene_allows(true, -3.0, 0),
            Err(AddOmniLightRefuse::NonPositiveRadius)
        );
    }

    #[test]
    fn an_omni_pack_has_no_cone() {
        let light = omni_light_pack([1.0, 2.0, 3.0], 64.0, [0.1, 0.2, 0.3]);
        assert_eq!(light.light_type, GFX_LIGHT_TYPE_OMNI);
        assert_eq!(light.origin, [1.0, 2.0, 3.0]);
        assert_eq!(light.radius, 64.0);
        assert_eq!(light.color, [0.1, 0.2, 0.3]);
        assert_eq!(light.direction, [0.0; 3]);
        assert_eq!(light.falloff_image_width, None);
    }

    #[test]
    fn the_default_cone_matches_the_registered_dvars() {
        let cone = SpotLightConeDvars::register_defaults();
        assert_eq!(cone.start_radius, 36.0);
        assert_eq!(cone.end_radius, 196.0);
        assert_eq!(cone.fov_inner_fraction, 0.5);
        assert_eq!(cone.brightness, 14.0);
        assert_eq!(cone.exponent, 2);
    }

    #[test]
    fn the_far_radius_is_kept_when_it_is_in_range() {
        assert_eq!(spot_end_radius(36.0, 196.0, 300.0), 196.0);
    }

    #[test]
    fn a_far_radius_inside_the_start_is_pushed_out() {
        assert!(near(spot_end_radius(36.0, 10.0, 300.0), 36.1));
        assert!(near(spot_end_radius(36.0, 36.0, 300.0), 36.1));
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
        let cone = SpotLightConeDvars::register_defaults();
        let light = spot_light_pack(
            [0.0, 0.0, 100.0],
            [0.0, 0.0, 1.0],
            320.0,
            [1.0, 0.5, 0.0],
            cone,
        );
        assert_eq!(light.light_type, GFX_LIGHT_TYPE_SPOT);
        assert_eq!(light.direction, [0.0, 0.0, -1.0]);
        assert!(near(light.origin[2], 28.0));
        assert!(near(light.radius, 392.0));
        assert_eq!(light.color, [14.0, 7.0, 0.0]);
        assert_eq!(light.exponent, 2);
    }

    #[test]
    fn a_spot_pack_inner_cone_is_narrower_than_the_outer() {
        let cone = SpotLightConeDvars::register_defaults();
        let light = spot_light_pack([0.0; 3], [1.0, 0.0, 0.0], 320.0, [1.0; 3], cone);
        // atan(0.5) is about 26.57 degrees.
        assert!(near(light.cos_outer, libm::cosf(libm::atanf(0.5))));
        assert!(light.cos_inner > light.cos_outer);
        assert!(near(light.cos_inner, libm::cosf(libm::atanf(0.5) * 0.5)));
    }
}
