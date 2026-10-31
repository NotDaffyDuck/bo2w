//! The few effect rules that only Black Ops II data needs. They live apart from the
//! generic draw and tick code so the core never has to name the game; a later step
//! moves this file into a `_t6` crate.

use asset_core::AssetCategoryScope;

/// Element flag: the sprite is `start_extent` wide and `second_extent` tall instead of square.
pub(super) const NONUNIFORM_SCALE: i32 = 0x1000_0000;

/// Swap the red and blue bytes of a four-byte colour (BGRA <-> RGBA), alpha untouched.
pub(super) fn swap_red_blue([a, g, b, alpha]: [u8; 4]) -> [u8; 4] {
    [b, g, a, alpha]
}

/// The scale a light element is drawn with.
///
/// Black Ops II light elements leave their scale at zero because the colour alone carries
/// the brightness (it fades to black over the element's life), so a scale of zero or less
/// means "full strength" for them.
pub(super) fn light_scale(scale: f32, namespace: AssetCategoryScope) -> f32 {
    if scale <= 0.0 && namespace == AssetCategoryScope::T6 {
        1.0
    } else {
        scale
    }
}

/// A light element's colour as blue, green, red floats.
///
/// Black Ops II's effect light colour (`R_AddOmniLightToScene`): the element's colour from
/// sRGB to linear, times its alpha times 32, in the world's light units (its sun is about 15
/// before exposure). Other games scale the bytes by `scale`.
pub(super) fn light_color_bgr(color_rgba: [u8; 4], scale: f32, namespace: AssetCategoryScope) -> [f32; 3] {
    if namespace != AssetCategoryScope::T6 {
        return fx_t6::runtime::elem_light_color_bgr(color_rgba, scale);
    }
    let linear = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let strength = f32::from(color_rgba[3]) / 255.0 * 32.0;
    [
        linear(color_rgba[2]) * strength,
        linear(color_rgba[1]) * strength,
        linear(color_rgba[0]) * strength,
    ]
}
