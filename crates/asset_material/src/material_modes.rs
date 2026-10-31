//! What a material's state bits say about how it is drawn.

use crate::AssetCategoryScope;

/// The layout of the first state-bits word that these readers look at.
const SRC_BLEND_MASK: u32 = 0xf;
const DST_BLEND_SHIFT: u32 = 4;
const BLEND_OP_SHIFT: u32 = 8;
const ALPHA_TEST_OFF_BIT: u32 = 1 << 11;
const ALPHA_TEST_MODE_BIT: u32 = 1 << 12;

const BLEND_ADD: u32 = 1;
const BLEND_ZERO_SRC_COLOR: (u32, u32) = (1, 3);
const BLEND_SRC_ALPHA_ONE: (u32, u32) = (2, 2);
const BLEND_ONE_MINUS_DST_ONE: (u32, u32) = (10, 2);

/// Which sort of blending a material asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderDrawStyle {
    Opaque,
    AlphaTest { at_or_over_half: bool },
    Blend,
    Multiply,
    Additive,
    Screen,
}

impl ShaderDrawStyle {
    pub fn from_state_bits(loading_flag_mask: [u32; 2]) -> Self {
        let word = loading_flag_mask[0];
        let blend_op = (word >> BLEND_OP_SHIFT) & 0b111;
        let pair = (word & SRC_BLEND_MASK, (word >> DST_BLEND_SHIFT) & 0xf);
        match (blend_op, pair) {
            (0, _) => alpha_test_wants_half(word)
                .map_or(Self::Opaque, |at_or_over_half| Self::AlphaTest { at_or_over_half }),
            (BLEND_ADD, BLEND_ZERO_SRC_COLOR) => Self::Multiply,
            (BLEND_ADD, BLEND_SRC_ALPHA_ONE) => Self::Additive,
            (BLEND_ADD, BLEND_ONE_MINUS_DST_ONE) => Self::Screen,
            _ => Self::Blend,
        }
    }
}

/// Whether the alpha test wants alpha of at least 128 (true) or any alpha above zero
/// (false); `None` when the test is switched off.
fn alpha_test_wants_half(word: u32) -> Option<bool> {
    (word & ALPHA_TEST_OFF_BIT == 0).then_some(word & ALPHA_TEST_MODE_BIT != 0)
}

/// The alpha value a pixel must reach to survive the alpha test.
pub fn cutout_level_of(loading_flag_mask: [u32; 2]) -> Option<f32> {
    alpha_test_wants_half(loading_flag_mask[0]).map(|at_or_over_half| if at_or_over_half { 0.5 } else { 0.01 })
}

/// How a material's colour map is turned before it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintMapRemap {
    Square,
    Unknown,
}

pub use crate::raster_state::{
    AlphaGate, WINDING_CULL_COUNTERCLOCKWISE, FACE_CULL_CLOCKWISE, FACE_CULL_DISABLED, STATE_FACE_CULL, CUTOFF_OFF_BIT,
    CUTOFF_HALF_OR_MORE, CUTOFF_ABOVE_ZERO, CUTOFF_FIELD_MASK, CUTOFF_FIELD_SHIFT,
    RASTER_DROP_REAR_FACES, CULL_FIELD_FRONT, CULL_FIELD_MASK, CULL_FIELD_OFF, CULL_FIELD_SHIFT,
    TransparencyCutoffMode, FaceCullingMode, CUTOFF_FOR_FIELD, CULL_MODE_FOR_FIELD,
    cutout_flag_from_raster_bits, face_culling_of, device_cull_mode_of,
};

/// A material constant's name: the bytes up to the first zero.
pub fn surface_parameter_label(name: &[u8; 12]) -> &str {
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    std::str::from_utf8(&name[..len]).unwrap_or("")
}

/// The alpha test a material draws with. Black Ops II state bits are not
/// decoded into one yet, so no material gets any.
pub fn material_alpha_test(_namespace: AssetCategoryScope, _bits: [u32; 2]) -> Option<AlphaGate> {
    None
}

/// The camera region in which a static model is never drawn.
const CAMERA_REGION_SKIPPED: u8 = 3;
/// The game-flags bit that keeps a static model out of the camera pass.
const FLAG_NOT_IN_CAMERA: u8 = 1;

/// Whether a static model with these flags is drawn in this camera region.
pub const fn t5_smodel_camera_emits(game_flags: u8, camera_zone: u8) -> bool {
    camera_zone != CAMERA_REGION_SKIPPED && game_flags & FLAG_NOT_IN_CAMERA == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(word: u32) -> ShaderDrawStyle {
        ShaderDrawStyle::from_state_bits([word, 0])
    }

    #[test]
    fn blend_pairs_pick_their_mode() {
        let blend = |op: u32, src: u32, dst: u32| (op << 8) | src | (dst << 4);
        assert_eq!(mode(blend(1, 1, 3)), ShaderDrawStyle::Multiply);
        assert_eq!(mode(blend(1, 2, 2)), ShaderDrawStyle::Additive);
        assert_eq!(mode(blend(1, 10, 2)), ShaderDrawStyle::Screen);
        assert_eq!(mode(blend(1, 5, 6)), ShaderDrawStyle::Blend);
        assert_eq!(mode(blend(2, 1, 3)), ShaderDrawStyle::Blend);
    }

    #[test]
    fn no_blend_falls_back_to_the_alpha_test() {
        assert_eq!(mode(ALPHA_TEST_OFF_BIT), ShaderDrawStyle::Opaque);
        assert_eq!(
            mode(1 << 12),
            ShaderDrawStyle::AlphaTest { at_or_over_half: true }
        );
        assert_eq!(mode(0), ShaderDrawStyle::AlphaTest { at_or_over_half: false });
        // Bit 13 is unused in Black Ops II and must not change the reading.
        assert_eq!(
            mode(1 << 13),
            ShaderDrawStyle::AlphaTest { at_or_over_half: false }
        );
    }

    #[test]
    fn the_cutoff_follows_the_alpha_test_field() {
        assert_eq!(cutout_level_of([ALPHA_TEST_OFF_BIT, 0]), None);
        // Black Ops II: one bit at 12 picks "at least 128" (set) or "above zero" (clear).
        assert_eq!(cutout_level_of([1 << 12, 0]), Some(0.5));
        assert_eq!(cutout_level_of([0, 0]), Some(0.01));
        assert_eq!(cutout_level_of([1 << 13, 0]), Some(0.01));
        assert_eq!(
            cutout_level_of([ALPHA_TEST_OFF_BIT | (1 << 12), 0]),
            None
        );
    }

    #[test]
    fn constant_names_read_up_to_the_first_zero() {
        assert_eq!(surface_parameter_label(b"colorTint\0\0\0"), "colorTint");
        assert_eq!(surface_parameter_label(b"abcdefghijkl"), "abcdefghijkl");
    }

    #[test]
    fn static_models_skip_region_three_and_flagged_ones() {
        assert!(t5_smodel_camera_emits(0, 0));
        assert!(!t5_smodel_camera_emits(1, 0));
        assert!(!t5_smodel_camera_emits(0, 3));
    }
}
