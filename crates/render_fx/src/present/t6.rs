//! The few effect rules that only Black Ops II data needs. They live apart from the
//! generic draw and tick code so the core never has to name the game; a later step
//! moves this file into a `_t6` crate.

use asset_core::AssetNamespace;

/// Element flag: the sprite is `size0` wide and `size1` tall instead of square.
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
pub(super) fn light_scale(scale: f32, namespace: AssetNamespace) -> f32 {
    if scale <= 0.0 && namespace == AssetNamespace::T6 {
        1.0
    } else {
        scale
    }
}
