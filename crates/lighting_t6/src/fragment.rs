//! The lit-model pixel shader's arithmetic, done on the CPU for previews and
//! baked colours.

use crate::expand::{LOOKUP_SPAN, MODEL_LIGHTING_TILE_DIM};

/// The largest of the three components (not the largest magnitude).
fn max_component(v: [f32; 3]) -> f32 {
    v[0].max(v[1]).max(v[2])
}

/// A lighting sample is stored halved and square-rooted; this undoes both.
pub fn decode_model_lighting_sample(sample: [f32; 3]) -> [f32; 3] {
    let channel = |v: f32| {
        let doubled = v + v;
        doubled * doubled
    };
    [channel(sample[0]), channel(sample[1]), channel(sample[2])]
}

/// The surface colour after the vertex tint, squared back out of gamma.
pub fn lit_albedo(color_map: [f32; 3], vertex_color: [f32; 3]) -> [f32; 3] {
    let channel = |map: f32, tint: f32| {
        let tinted = map * tint;
        tinted * tinted
    };
    [
        channel(color_map[0], vertex_color[0]),
        channel(color_map[1], vertex_color[1]),
        channel(color_map[2], vertex_color[2]),
    ]
}

/// Ambient light from the sample plus the sun, scaled by how much sun the
/// sample's alpha lets through and how squarely the surface faces it.
pub fn lit_sun_lighting(
    decoded_ambient: [f32; 3],
    sample_alpha: f32,
    n_dot_l_sat: f32,
    light_diffuse_rgb: [f32; 3],
) -> [f32; 3] {
    let sun = |diffuse: f32| sample_alpha * n_dot_l_sat * diffuse;
    [
        decoded_ambient[0] + sun(light_diffuse_rgb[0]),
        decoded_ambient[1] + sun(light_diffuse_rgb[1]),
        decoded_ambient[2] + sun(light_diffuse_rgb[2]),
    ]
}

/// The final colour: the surface lit by `decoded_lighting`, plus shine.
pub fn lit_fragment_color(
    lit_albedo: [f32; 3],
    decoded_lighting: [f32; 3],
    specular_term: [f32; 3],
) -> [f32; 3] {
    [
        lit_albedo[0] * decoded_lighting[0] + specular_term[0],
        lit_albedo[1] * decoded_lighting[1] + specular_term[1],
        lit_albedo[2] * decoded_lighting[2] + specular_term[2],
    ]
}

/// Which texel of a model's 4x4x4 lighting tile a surface normal looks up
/// (nearest texel, no blending). `None` when no component of the normal is
/// positive, since there is then no direction to scale by.
pub fn model_lighting_local_tile_nearest_index(normal: [f32; 3]) -> Option<usize> {
    let largest = max_component(normal);
    if largest == 0.0 {
        return None;
    }
    let inverse = 1.0 / largest;
    let texel = |component: f32| -> usize {
        let coord = component * LOOKUP_SPAN * inverse + 0.5;
        libm::roundf(coord * MODEL_LIGHTING_TILE_DIM as f32 - 0.5).clamp(0.0, 3.0) as usize
    };
    let (x, y, z) = (texel(normal[0]), texel(normal[1]), texel(normal[2]));
    Some(x + MODEL_LIGHTING_TILE_DIM * y + MODEL_LIGHTING_TILE_DIM * MODEL_LIGHTING_TILE_DIM * z)
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
            decode_model_lighting_sample([0.5, 0.25, 0.0]),
            [1.0, 0.25, 0.0]
        ));
        assert!(near3(
            decode_model_lighting_sample([1.0, 0.1, 0.0]),
            [4.0, 0.04, 0.0]
        ));
    }

    #[test]
    fn albedo_is_tinted_then_squared() {
        assert!(near3(
            lit_albedo([0.5, 1.0, 0.0], [1.0, 0.5, 1.0]),
            [0.25, 0.25, 0.0]
        ));
    }

    #[test]
    fn an_untinted_grey_albedo_is_its_square() {
        assert!(near3(lit_albedo([0.5; 3], [1.0; 3]), [0.25; 3]));
    }

    #[test]
    fn sun_light_adds_to_the_ambient() {
        let lit = lit_sun_lighting([0.1, 0.2, 0.3], 0.5, 0.8, [1.0, 2.0, 3.0]);
        assert!(near3(lit, [0.5, 1.0, 1.5]));
    }

    #[test]
    fn a_surface_facing_away_from_the_sun_keeps_only_the_ambient() {
        let lit = lit_sun_lighting([0.1, 0.2, 0.3], 1.0, 0.0, [9.0; 3]);
        assert_eq!(lit, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn the_fragment_is_albedo_times_light_plus_shine() {
        let colour = lit_fragment_color([0.5, 0.25, 1.0], [2.0, 2.0, 0.5], [0.1, 0.0, 0.2]);
        assert!(near3(colour, [1.1, 0.5, 0.7]));
    }

    #[test]
    fn straight_up_looks_up_the_top_middle_texel() {
        // x and y land on the middle texel (2), z clamps to the last (3).
        assert_eq!(
            model_lighting_local_tile_nearest_index([0.0, 0.0, 1.0]),
            Some(2 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn a_normal_along_x_reaches_the_edge_in_x_only() {
        assert_eq!(
            model_lighting_local_tile_nearest_index([1.0, 0.0, 0.0]),
            Some(3 + 4 * 2 + 16 * 2)
        );
    }

    #[test]
    fn a_normal_equal_in_all_axes_reaches_the_far_corner() {
        assert_eq!(
            model_lighting_local_tile_nearest_index([1.0, 1.0, 1.0]),
            Some(63)
        );
    }

    #[test]
    fn a_small_lean_moves_to_a_neighbouring_texel() {
        // x coordinate 0.35 sits in texel 1.
        assert_eq!(
            model_lighting_local_tile_nearest_index([-0.1, 0.0, 1.0]),
            Some(1 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn a_big_lean_back_clamps_to_the_first_texel() {
        assert_eq!(
            model_lighting_local_tile_nearest_index([-0.5, 0.0, 1.0]),
            Some(0 + 4 * 2 + 16 * 3)
        );
    }

    #[test]
    fn with_no_positive_component_there_is_no_texel() {
        assert_eq!(model_lighting_local_tile_nearest_index([0.0; 3]), None);
        assert_eq!(
            model_lighting_local_tile_nearest_index([-1.0, 0.0, 0.0]),
            None
        );
    }
}
