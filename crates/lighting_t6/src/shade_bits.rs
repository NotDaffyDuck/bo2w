//! The lit-model pixel shader's arithmetic, done on the CPU for previews and
//! baked colours.

use crate::tile_unpack::{LOOKUP_SPAN, ILLUM_TILE_SIDE};

/// The largest of the three components (not the largest magnitude).
fn max_component(v: [f32; 3]) -> f32 {
    v[0].max(v[1]).max(v[2])
}

/// A lighting sample is stored halved and square-rooted; this undoes both.
pub fn unpack_model_light_reading(sample: [f32; 3]) -> [f32; 3] {
    let channel = |v: f32| {
        let doubled = v + v;
        doubled * doubled
    };
    [channel(sample[0]), channel(sample[1]), channel(sample[2])]
}

/// The surface colour after the vertex tint, squared back out of gamma.
pub fn shaded_base_color(base_map: [f32; 3], tint_rgb: [f32; 3]) -> [f32; 3] {
    let channel = |map: f32, tint: f32| {
        let tinted = map * tint;
        tinted * tinted
    };
    [
        channel(base_map[0], tint_rgb[0]),
        channel(base_map[1], tint_rgb[1]),
        channel(base_map[2], tint_rgb[2]),
    ]
}

/// Ambient light from the sample plus the sun, scaled by how much sun the
/// sample's alpha lets through and how squarely the surface faces it.
pub fn sunlit_shading(
    ambient: [f32; 3],
    sun_share: f32,
    facing: f32,
    sun_rgb: [f32; 3],
) -> [f32; 3] {
    let sun = |diffuse: f32| sun_share * facing * diffuse;
    [
        ambient[0] + sun(sun_rgb[0]),
        ambient[1] + sun(sun_rgb[1]),
        ambient[2] + sun(sun_rgb[2]),
    ]
}

/// The final colour: the surface lit by `lit`, plus shine.
pub fn shaded_pixel_colour(
    shaded_base_color: [f32; 3],
    lit: [f32; 3],
    shine: [f32; 3],
) -> [f32; 3] {
    [
        shaded_base_color[0] * lit[0] + shine[0],
        shaded_base_color[1] * lit[1] + shine[1],
        shaded_base_color[2] * lit[2] + shine[2],
    ]
}

/// Which texel of a model's 4x4x4 lighting tile a surface normal looks up
/// (nearest texel, no blending). `None` when no component of the normal is
/// positive, since there is then no direction to scale by.
pub fn light_tile_closest_direction(normal: [f32; 3]) -> Option<usize> {
    let peak = max_component(normal);
    if peak == 0.0 {
        return None;
    }
    let inverse = 1.0 / peak;
    let texel = |component: f32| -> usize {
        let coord = component * (LOOKUP_SPAN / ILLUM_TILE_SIDE as f32) * inverse + 0.5;
        let last = (ILLUM_TILE_SIDE - 1) as f32;
        libm::roundf(coord * ILLUM_TILE_SIDE as f32 - 0.5).clamp(0.0, last) as usize
    };
    let (x, y, z) = (texel(normal[0]), texel(normal[1]), texel(normal[2]));
    Some(x + ILLUM_TILE_SIDE * y + ILLUM_TILE_SIDE * ILLUM_TILE_SIDE * z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near3(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn decoding_doubles_then_squares() {
        assert!(near3(
            unpack_model_light_reading([0.5, 0.25, 0.0]),
            [1.0, 0.25, 0.0]
        ));
        assert!(near3(
            unpack_model_light_reading([1.0, 0.1, 0.0]),
            [4.0, 0.04, 0.0]
        ));
    }

    #[test]
    fn albedo_is_tinted_then_squared() {
        assert!(near3(
            shaded_base_color([0.5, 1.0, 0.0], [1.0, 0.5, 1.0]),
            [0.25, 0.25, 0.0]
        ));
    }

    #[test]
    fn an_untinted_grey_albedo_is_its_square() {
        assert!(near3(shaded_base_color([0.5; 3], [1.0; 3]), [0.25; 3]));
    }

    #[test]
    fn sun_light_adds_to_the_ambient() {
        let lit = sunlit_shading([0.1, 0.2, 0.3], 0.5, 0.8, [1.0, 2.0, 3.0]);
        assert!(near3(lit, [0.5, 1.0, 1.5]));
    }

    #[test]
    fn a_surface_facing_away_from_the_sun_keeps_only_the_ambient() {
        let lit = sunlit_shading([0.1, 0.2, 0.3], 1.0, 0.0, [9.0; 3]);
        assert_eq!(lit, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn the_fragment_is_albedo_times_light_plus_shine() {
        let colour = shaded_pixel_colour([0.5, 0.25, 1.0], [2.0, 2.0, 0.5], [0.1, 0.0, 0.2]);
        assert!(near3(colour, [1.1, 0.5, 0.7]));
    }

    #[test]
    fn straight_up_looks_up_the_top_middle_texel() {
        // x and y land on the middle texel (2), z clamps to the last (3).
        assert_eq!(
            light_tile_closest_direction([0.0, 0.0, 1.0]),
            Some(2 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn a_normal_along_x_reaches_the_edge_in_x_only() {
        assert_eq!(
            light_tile_closest_direction([1.0, 0.0, 0.0]),
            Some(3 + 4 * 2 + 16 * 2)
        );
    }

    #[test]
    fn a_normal_equal_in_all_axes_reaches_the_far_corner() {
        assert_eq!(
            light_tile_closest_direction([1.0, 1.0, 1.0]),
            Some(63)
        );
    }

    #[test]
    fn a_small_lean_moves_to_a_neighbouring_texel() {
        // x coordinate 0.35 sits in texel 1.
        assert_eq!(
            light_tile_closest_direction([-0.1, 0.0, 1.0]),
            Some(1 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn a_big_lean_back_clamps_to_the_first_texel() {
        assert_eq!(
            light_tile_closest_direction([-1.0, 0.0, 1.0]),
            Some(0 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn a_half_normal_reaches_the_middle_of_the_tile_like_the_old_scale() {
        assert_eq!(light_tile_closest_direction([0.5, 0.5, 1.0]), Some(2 + 4 * 2 + 16 * 3));
    }

    #[test]
    fn with_no_positive_component_there_is_no_texel() {
        assert_eq!(light_tile_closest_direction([0.0; 3]), None);
        assert_eq!(
            light_tile_closest_direction([-1.0, 0.0, 0.0]),
            None
        );
    }
}
