//! What a material's state bits say about how it is drawn.

use crate::AssetNamespace;

/// The layout of the first state-bits word that these readers look at.
const SRC_BLEND_MASK: u32 = 0xf;
const DST_BLEND_SHIFT: u32 = 4;
const BLEND_OP_SHIFT: u32 = 8;
const ALPHA_TEST_OFF_BIT: u32 = 1 << 11;
const ALPHA_TEST_SHIFT: u32 = 12;
const ALPHA_TEST_GE_HALF: u32 = 3;
const SRGB_WRITE_BIT: u32 = 0x4000_0000;

const BLEND_ADD: u32 = 1;
const BLEND_ZERO_SRC_COLOR: (u32, u32) = (1, 3);
const BLEND_SRC_ALPHA_ONE: (u32, u32) = (2, 2);
const BLEND_ONE_MINUS_DST_ONE: (u32, u32) = (10, 2);

/// Which sort of blending a material asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialDrawMode {
    Opaque,
    AlphaTest { ge_half: bool },
    Blend,
    Multiply,
    Additive,
    Screen,
}

impl MaterialDrawMode {
    pub fn from_state_bits(load_bits: [u32; 2]) -> Self {
        let word = load_bits[0];
        let blend_op = (word >> BLEND_OP_SHIFT) & 0b111;
        if blend_op != 0 {
            let pair = (word & SRC_BLEND_MASK, (word >> DST_BLEND_SHIFT) & 0xf);
            return match (blend_op, pair) {
                (BLEND_ADD, BLEND_ZERO_SRC_COLOR) => Self::Multiply,
                (BLEND_ADD, BLEND_SRC_ALPHA_ONE) => Self::Additive,
                (BLEND_ADD, BLEND_ONE_MINUS_DST_ONE) => Self::Screen,
                _ => Self::Blend,
            };
        }
        match alpha_test_mode(word) {
            Some(mode) => Self::AlphaTest {
                ge_half: mode == ALPHA_TEST_GE_HALF,
            },
            None => Self::Opaque,
        }
    }
}

/// The alpha-test setting field, or `None` when the test is switched off.
fn alpha_test_mode(word: u32) -> Option<u32> {
    (word & ALPHA_TEST_OFF_BIT == 0).then_some((word >> ALPHA_TEST_SHIFT) & 0b11)
}

/// The alpha value a pixel must reach to survive the alpha test.
pub fn alpha_test_cutoff_from_state_bits(load_bits: [u32; 2]) -> Option<f32> {
    alpha_test_mode(load_bits[0]).map(|mode| {
        if mode == ALPHA_TEST_GE_HALF {
            0.5
        } else {
            0.01
        }
    })
}

pub fn srgb_write_enable_from_state_bits(load_bits: [u32; 2]) -> bool {
    load_bits[0] & SRGB_WRITE_BIT != 0
}

/// How a material's colour map is turned before it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMapTransform {
    Square,
    Unknown,
}

pub use crate::state_bits::{
    AlphaTest, D3DCULL_CCW, D3DCULL_CW, D3DCULL_NONE, D3DRS_CULLMODE, GFXS0_ATEST_DISABLE,
    GFXS0_ATEST_GE_128, GFXS0_ATEST_GT_0, GFXS0_ATEST_LT_128, GFXS0_ATEST_MASK, GFXS0_ATEST_SHIFT,
    GFXS0_CULL_BACK, GFXS0_CULL_FRONT, GFXS0_CULL_MASK, GFXS0_CULL_NONE, GFXS0_CULL_SHIFT,
    Gfxs0AlphaTest, Gfxs0CullFace as MaterialCullFace, S_ALPHA_TEST_TABLE, S_CULL_TABLE,
    alpha_test_from_state_bits, cull_face_from_state_bits, d3d_cull_mode_from_state_bits,
};

/// A material constant's name: the bytes up to the first zero.
pub fn material_constant_name(name: &[u8; 12]) -> &str {
    let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    std::str::from_utf8(&name[..len]).unwrap_or("")
}

/// The alpha test a material draws with. Black Ops II state bits are not
/// decoded into one yet, so no material gets any.
pub fn material_alpha_test(_namespace: AssetNamespace, _bits: [u32; 2]) -> Option<AlphaTest> {
    None
}

/// The camera region in which a static model is never drawn.
const CAMERA_REGION_SKIPPED: u8 = 3;
/// The game-flags bit that keeps a static model out of the camera pass.
const FLAG_NOT_IN_CAMERA: u8 = 1;

/// Whether a static model with these flags is drawn in this camera region.
pub const fn t5_smodel_camera_emits(game_flags: u8, camera_region: u8) -> bool {
    camera_region != CAMERA_REGION_SKIPPED && game_flags & FLAG_NOT_IN_CAMERA == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(word: u32) -> MaterialDrawMode {
        MaterialDrawMode::from_state_bits([word, 0])
    }

    #[test]
    fn blend_pairs_pick_their_mode() {
        let blend = |op: u32, src: u32, dst: u32| (op << 8) | src | (dst << 4);
        assert_eq!(mode(blend(1, 1, 3)), MaterialDrawMode::Multiply);
        assert_eq!(mode(blend(1, 2, 2)), MaterialDrawMode::Additive);
        assert_eq!(mode(blend(1, 10, 2)), MaterialDrawMode::Screen);
        assert_eq!(mode(blend(1, 5, 6)), MaterialDrawMode::Blend);
        assert_eq!(mode(blend(2, 1, 3)), MaterialDrawMode::Blend);
    }

    #[test]
    fn no_blend_falls_back_to_the_alpha_test() {
        assert_eq!(mode(ALPHA_TEST_OFF_BIT), MaterialDrawMode::Opaque);
        assert_eq!(
            mode(3 << 12),
            MaterialDrawMode::AlphaTest { ge_half: true }
        );
        assert_eq!(
            mode(1 << 12),
            MaterialDrawMode::AlphaTest { ge_half: false }
        );
    }

    #[test]
    fn the_cutoff_follows_the_alpha_test_field() {
        assert_eq!(alpha_test_cutoff_from_state_bits([ALPHA_TEST_OFF_BIT, 0]), None);
        assert_eq!(alpha_test_cutoff_from_state_bits([3 << 12, 0]), Some(0.5));
        assert_eq!(alpha_test_cutoff_from_state_bits([2 << 12, 0]), Some(0.01));
    }

    #[test]
    fn the_srgb_bit_and_constant_names_read_through() {
        assert!(srgb_write_enable_from_state_bits([SRGB_WRITE_BIT, 0]));
        assert!(!srgb_write_enable_from_state_bits([0, SRGB_WRITE_BIT]));
        assert_eq!(material_constant_name(b"colorTint\0\0\0"), "colorTint");
        assert_eq!(material_constant_name(b"abcdefghijkl"), "abcdefghijkl");
    }

    #[test]
    fn static_models_skip_region_three_and_flagged_ones() {
        assert!(t5_smodel_camera_emits(0, 0));
        assert!(!t5_smodel_camera_emits(1, 0));
        assert!(!t5_smodel_camera_emits(0, 3));
    }
}
