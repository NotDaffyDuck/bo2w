//! A material pass's render-state words, decoded into the blend, cull, colour
//! write and depth settings the draw needs.
//!
//! The two 32-bit words are the authored layout: in `word0` the colour blend
//! sits in bits 0-10 (source, destination, operation), the alpha blend in
//! bits 16-26, and the colour-write, line-fill flags in bits 27, 28 and 31;
//! `word1` carries the stencil and depth state.

use bevy::render::render_resource::{
    BlendComponent, BlendFactor as WgpuBlendFactor, BlendOperation, BlendState, ColorWrites,
};
use d3d9_state::BlendFactor;

/// Width of one blend description: source and destination (4 bits each) and
/// the operation (3 bits).
const BLEND_FIELD_MASK: u32 = 0x7ff;
const BLEND_OP_BITS: u32 = 0x700;
const ALPHA_BLEND_SHIFT: u32 = 16;

const COLOUR_WRITE_RGB: u32 = 0x0800_0000;
const COLOUR_WRITE_ALPHA: u32 = 0x1000_0000;
const LINE_FILL: u32 = 0x8000_0000;

const STENCIL_ENABLE_BITS: u32 = 0xc0;
const STENCIL_STATE_BITS: u32 = 0xffff_ffc0;

/// Raw blend-operation numbers the pass words use.
const OP_NONE: u8 = 0;
const OP_ADD: u8 = 1;
const OP_LAST_KNOWN: u8 = 5;

/// Raw factor numbers whose pair marks a multiply pass.
const FACTOR_ZERO_RAW: u32 = 1;
const FACTOR_SRC_COLOUR_RAW: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DrawBlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
    Unknown(u8),
}

impl DrawBlendOperation {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Add,
            2 => Self::Subtract,
            3 => Self::ReverseSubtract,
            4 => Self::Min,
            5 => Self::Max,
            other => Self::Unknown(other),
        }
    }

    fn to_wgpu(self) -> Option<BlendOperation> {
        match self {
            Self::Add => Some(BlendOperation::Add),
            Self::Subtract => Some(BlendOperation::Subtract),
            Self::ReverseSubtract => Some(BlendOperation::ReverseSubtract),
            Self::Min => Some(BlendOperation::Min),
            Self::Max => Some(BlendOperation::Max),
            Self::Unknown(_) => None,
        }
    }
}

/// One half (colour or alpha) of a blend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DrawBlendComponent {
    pub src: BlendFactor,
    pub dst: BlendFactor,
    pub operation: DrawBlendOperation,
}

impl DrawBlendComponent {
    /// Decode the low 11 bits of `bits`.
    fn from_bits(bits: u32) -> Self {
        Self {
            src: BlendFactor::from_raw(bits & 0xf),
            dst: BlendFactor::from_raw((bits >> 4) & 0xf),
            operation: DrawBlendOperation::from_raw(((bits >> 8) & 0x7) as u8),
        }
    }

    /// Source zero, destination the source colour, added: the blend a
    /// multiply pass uses for its colour.
    fn multiply() -> Self {
        Self {
            src: BlendFactor::Zero,
            dst: BlendFactor::SrcColor,
            operation: DrawBlendOperation::Add,
        }
    }

    /// The GPU form. The factors and operation must be ones the GPU has;
    /// callers reject unsupported passes before they get here.
    fn to_wgpu(self) -> BlendComponent {
        BlendComponent {
            src_factor: d3d_blend_to_wgpu(self.src)
                .expect("unsupported D3D9 source blend factor reached the GPU adapter"),
            dst_factor: d3d_blend_to_wgpu(self.dst)
                .expect("unsupported D3D9 destination blend factor reached the GPU adapter"),
            operation: self
                .operation
                .to_wgpu()
                .expect("unsupported D3D9 blend operation reached the GPU adapter"),
        }
    }
}

/// How a pass combines with what is already on screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum DrawBlend {
    /// Replace the destination.
    #[default]
    Opaque,
    /// Separate colour and alpha blends.
    Factors {
        colour: DrawBlendComponent,
        alpha: DrawBlendComponent,
    },
    /// Colour multiplies the destination; only alpha has its own blend.
    Multiply { alpha: DrawBlendComponent },
}

/// The alpha half of a blend in `word0`; a pass that leaves the alpha
/// operation unset blends alpha like colour.
fn alpha_component(word0: u32, like_colour: DrawBlendComponent) -> DrawBlendComponent {
    let bits = (word0 >> ALPHA_BLEND_SHIFT) & BLEND_FIELD_MASK;
    if bits & BLEND_OP_BITS == 0 {
        like_colour
    } else {
        DrawBlendComponent::from_bits(bits)
    }
}

impl DrawBlend {
    /// Decode the blend in `word0`. A `multiply_pass` is always a multiply, a
    /// plain pass is one when it asks for zero × source colour added.
    pub fn from_word0(word0: u32, multiply_pass: bool) -> Self {
        if multiply_pass {
            return Self::Multiply {
                alpha: alpha_component(word0, DrawBlendComponent::multiply()),
            };
        }
        let operation = ((word0 >> 8) & 0x7) as u8;
        if operation == OP_NONE {
            return Self::Opaque;
        }
        let colour = DrawBlendComponent::from_bits(word0);
        let alpha = alpha_component(word0, colour);
        let is_multiply = operation == OP_ADD
            && word0 & 0xf == FACTOR_ZERO_RAW
            && (word0 >> 4) & 0xf == FACTOR_SRC_COLOUR_RAW;
        if is_multiply {
            Self::Multiply { alpha }
        } else {
            Self::Factors { colour, alpha }
        }
    }

    /// The GPU blend state, `None` for an opaque draw.
    pub fn blend_state(self) -> Option<BlendState> {
        match self {
            Self::Opaque => None,
            Self::Factors { colour, alpha } => Some(BlendState {
                color: colour.to_wgpu(),
                alpha: alpha.to_wgpu(),
            }),
            Self::Multiply { alpha } => Some(BlendState {
                color: BlendComponent {
                    src_factor: WgpuBlendFactor::Zero,
                    dst_factor: WgpuBlendFactor::Src,
                    operation: BlendOperation::Add,
                },
                alpha: alpha.to_wgpu(),
            }),
        }
    }
}

fn d3d_blend_to_wgpu(factor: BlendFactor) -> Option<WgpuBlendFactor> {
    use BlendFactor as D3d;
    use WgpuBlendFactor as Gpu;
    match factor {
        D3d::Zero => Some(Gpu::Zero),
        D3d::One => Some(Gpu::One),
        D3d::SrcColor => Some(Gpu::Src),
        D3d::InvSrcColor => Some(Gpu::OneMinusSrc),
        D3d::SrcAlpha => Some(Gpu::SrcAlpha),
        D3d::InvSrcAlpha => Some(Gpu::OneMinusSrcAlpha),
        D3d::DestAlpha => Some(Gpu::DstAlpha),
        D3d::InvDestAlpha => Some(Gpu::OneMinusDstAlpha),
        D3d::DestColor => Some(Gpu::Dst),
        D3d::InvDestColor => Some(Gpu::OneMinusDst),
        D3d::SrcAlphaSat => Some(Gpu::SrcAlphaSaturated),
        D3d::Unknown(_) => None,
    }
}

/// Colour-pipeline state decoded from `word0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChangeState0Host {
    pub blend: DrawBlend,
    /// 0 none, 1 back, 2 front.
    pub cull: u8,
    pub srgb_write: bool,
    /// Bit 0 red/green/blue, bit 1 alpha.
    pub colour_write: u8,
    pub line_fill: bool,
    pub alpha_test: Option<d3d9_state::AlphaTest>,
}

impl ChangeState0Host {
    pub fn colour_writes(self) -> ColorWrites {
        let mut writes = ColorWrites::empty();
        if self.colour_write & 1 != 0 {
            writes |= ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE;
        }
        if self.colour_write & 2 != 0 {
            writes |= ColorWrites::ALPHA;
        }
        writes
    }
}

/// Depth and stencil state decoded from `word1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChangeState1Host {
    pub stencil: u32,
    pub depth_write: bool,
    pub depth_test_enable: bool,
    pub depth_func: u8,
    pub polyoffset_level: u8,
}

/// A pass's two state words, plus the alpha test its material authored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GfxPassState {
    pub word0: u32,
    pub word1: u32,
    alpha_test: Option<d3d9_state::AlphaTest>,
}

/// State a pass asks for that the host cannot draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnsupportedStateFields {
    pub unknown_blend_factor: bool,
    pub unknown_blend_operation: bool,
    pub stencil: bool,
}

impl UnsupportedStateFields {
    pub fn any(self) -> bool {
        self.unknown_blend_factor || self.unknown_blend_operation || self.stencil
    }
}

/// Which non-default features a pass authored, for diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthoredStateFields {
    pub non_add_blend: bool,
    pub independent_alpha_blend: bool,
    pub partial_colour_write: bool,
    pub line_fill: bool,
    pub stencil: bool,
}

impl GfxPassState {
    pub fn from_state_bits(word0: u32, word1: u32) -> Self {
        Self::for_material(asset_core::AssetNamespace::T6, [word0, word1])
    }

    pub fn for_material(namespace: asset_core::AssetNamespace, bits: [u32; 2]) -> Self {
        Self {
            word0: bits[0],
            word1: bits[1],
            alpha_test: asset_material::material_alpha_test(namespace, bits),
        }
    }

    pub fn from_bits(bits: render_material::GfxPassStateBits) -> Self {
        Self::for_material(bits.namespace, [bits.word0, bits.word1])
    }

    fn words(self) -> [u32; 2] {
        [self.word0, self.word1]
    }

    pub fn draw_mode(self) -> asset_material::MaterialDrawMode {
        asset_material::MaterialDrawMode::from_state_bits(self.words())
    }

    pub fn to_draw_blend(self, multiply_pass: bool) -> DrawBlend {
        DrawBlend::from_word0(self.word0, multiply_pass)
    }

    pub fn srgb_write_enable(self) -> bool {
        asset_material::srgb_write_enable_from_state_bits(self.words())
    }

    pub fn cull_face(self) -> asset_material::MaterialCullFace {
        asset_material::cull_face_from_state_bits(self.words())
    }

    pub fn apply_change_state_0_host(
        self,
        _alpha_mode: bevy::prelude::AlphaMode,
        multiply_pass: bool,
    ) -> ChangeState0Host {
        use asset_material::MaterialCullFace as Face;
        let cull = match self.cull_face() {
            Face::None => 0,
            Face::Back => 1,
            Face::Front => 2,
        };
        let rgb = u8::from(self.word0 & COLOUR_WRITE_RGB != 0);
        let alpha = u8::from(self.word0 & COLOUR_WRITE_ALPHA != 0);
        ChangeState0Host {
            blend: self.to_draw_blend(multiply_pass),
            cull,
            srgb_write: self.srgb_write_enable(),
            colour_write: rgb | (alpha << 1),
            line_fill: self.word0 & LINE_FILL != 0,
            alpha_test: self.authored_alpha_test(),
        }
    }

    pub fn authored_host_fields(self) -> AuthoredStateFields {
        let colour_op = ((self.word0 >> 8) & 0x7) as u8;
        let colour_blend = self.word0 & BLEND_FIELD_MASK;
        let alpha_blend = (self.word0 >> ALPHA_BLEND_SHIFT) & BLEND_FIELD_MASK;
        let alpha_op = ((alpha_blend >> 8) & 0x7) as u8;
        let both_colour_writes = COLOUR_WRITE_RGB | COLOUR_WRITE_ALPHA;
        AuthoredStateFields {
            non_add_blend: colour_op > OP_ADD || (colour_op != OP_NONE && alpha_op > OP_ADD),
            independent_alpha_blend: colour_op != OP_NONE
                && alpha_op != OP_NONE
                && alpha_blend != colour_blend,
            partial_colour_write: self.word0 & both_colour_writes != both_colour_writes,
            line_fill: self.word0 & LINE_FILL != 0,
            stencil: self.word1 & STENCIL_ENABLE_BITS != 0,
        }
    }

    pub fn authored_alpha_test(self) -> Option<d3d9_state::AlphaTest> {
        self.alpha_test
    }

    /// The blend problems the host cannot draw, if any. (Stencil is reported
    /// as supported.)
    pub fn unsupported_host_fields(self) -> Option<UnsupportedStateFields> {
        let colour_op = ((self.word0 >> 8) & 0x7) as u8;
        let colour = DrawBlendComponent::from_bits(self.word0);
        let alpha_bits = (self.word0 >> ALPHA_BLEND_SHIFT) & BLEND_FIELD_MASK;
        let alpha_op = ((alpha_bits >> 8) & 0x7) as u8;
        let alpha = DrawBlendComponent::from_bits(alpha_bits);

        let blends = colour_op != OP_NONE;
        let alpha_blends = blends && alpha_op != OP_NONE;
        let fields = UnsupportedStateFields {
            unknown_blend_factor: blends
                && (!colour.src.is_known()
                    || !colour.dst.is_known()
                    || (alpha_blends && (!alpha.src.is_known() || !alpha.dst.is_known()))),
            unknown_blend_operation: colour_op > OP_LAST_KNOWN
                || (blends && alpha_op > OP_LAST_KNOWN),
            stencil: false,
        };
        fields.any().then_some(fields)
    }

    pub fn apply_change_state_1_host(self) -> ChangeState1Host {
        ChangeState1Host {
            stencil: self.word1 & STENCIL_STATE_BITS,
            depth_write: render_material::depth_write_enable(self.word1),
            depth_test_enable: render_material::depth_test_enable(self.word1),
            depth_func: ((self.word1 >> 2) & 3) as u8,
            polyoffset_level: render_material::polygon_offset_level(self.word1) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `word0` with a colour blend (src, dst, op) and an optional alpha blend.
    fn word0(colour: (u32, u32, u32), alpha: Option<(u32, u32, u32)>) -> u32 {
        let low = colour.0 | (colour.1 << 4) | (colour.2 << 8);
        let high = alpha.map_or(0, |(s, d, o)| (s | (d << 4) | (o << 8)) << 16);
        low | high
    }

    fn state(word0: u32, word1: u32) -> GfxPassState {
        GfxPassState {
            word0,
            word1,
            alpha_test: None,
        }
    }

    #[test]
    fn operations_decode_and_unknowns_are_kept() {
        assert_eq!(DrawBlendOperation::from_raw(1), DrawBlendOperation::Add);
        assert_eq!(DrawBlendOperation::from_raw(5), DrawBlendOperation::Max);
        assert_eq!(
            DrawBlendOperation::from_raw(6),
            DrawBlendOperation::Unknown(6)
        );
        assert_eq!(
            DrawBlendOperation::from_raw(2).to_wgpu(),
            Some(BlendOperation::Subtract)
        );
        assert_eq!(DrawBlendOperation::Unknown(0).to_wgpu(), None);
    }

    #[test]
    fn no_operation_is_opaque() {
        assert_eq!(DrawBlend::from_word0(0, false), DrawBlend::Opaque);
        assert_eq!(DrawBlend::Opaque.blend_state(), None);
    }

    #[test]
    fn a_plain_blend_blends_alpha_like_colour() {
        // source alpha, inverse source alpha, add
        let blend = DrawBlend::from_word0(word0((5, 6, 1), None), false);
        let DrawBlend::Factors { colour, alpha } = blend else {
            panic!("expected factors, got {blend:?}");
        };
        assert_eq!(colour, alpha);
        assert_eq!(colour.operation, DrawBlendOperation::Add);
        let state = blend.blend_state().expect("blends");
        assert_eq!(state.color, state.alpha);
        assert_eq!(state.color.src_factor, WgpuBlendFactor::SrcAlpha);
        assert_eq!(state.color.dst_factor, WgpuBlendFactor::OneMinusSrcAlpha);
    }

    #[test]
    fn an_authored_alpha_blend_is_used_when_it_has_an_operation() {
        let blend = DrawBlend::from_word0(word0((5, 6, 1), Some((2, 2, 1))), false);
        let DrawBlend::Factors { colour, alpha } = blend else {
            panic!("expected factors, got {blend:?}");
        };
        assert_ne!(colour, alpha);
        assert_eq!(
            alpha,
            DrawBlendComponent::from_bits(2 | (2 << 4) | (1 << 8))
        );
    }

    #[test]
    fn zero_by_source_colour_is_a_multiply() {
        let blend = DrawBlend::from_word0(word0((1, 3, 1), None), false);
        assert!(matches!(blend, DrawBlend::Multiply { .. }));
        let state = blend.blend_state().expect("blends");
        assert_eq!(state.color.src_factor, WgpuBlendFactor::Zero);
        assert_eq!(state.color.dst_factor, WgpuBlendFactor::Src);
    }

    #[test]
    fn a_multiply_pass_is_a_multiply_even_without_a_blend() {
        let blend = DrawBlend::from_word0(0, true);
        assert_eq!(
            blend,
            DrawBlend::Multiply {
                alpha: DrawBlendComponent::multiply()
            }
        );
    }

    #[test]
    fn a_multiply_pass_takes_its_authored_alpha_blend() {
        let blend = DrawBlend::from_word0(word0((0, 0, 0), Some((2, 2, 1))), true);
        let DrawBlend::Multiply { alpha } = blend else {
            panic!("expected multiply, got {blend:?}");
        };
        assert_eq!(
            alpha,
            DrawBlendComponent::from_bits(2 | (2 << 4) | (1 << 8))
        );
    }

    #[test]
    fn colour_writes_follow_the_two_flags() {
        let host = |colour_write| ChangeState0Host {
            blend: DrawBlend::Opaque,
            cull: 0,
            srgb_write: false,
            colour_write,
            line_fill: false,
            alpha_test: None,
        };
        assert_eq!(host(0).colour_writes(), ColorWrites::empty());
        assert_eq!(
            host(1).colour_writes(),
            ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE
        );
        assert_eq!(host(2).colour_writes(), ColorWrites::ALPHA);
        assert_eq!(host(3).colour_writes(), ColorWrites::ALL);
    }

    #[test]
    fn partial_colour_writes_are_reported() {
        let both = COLOUR_WRITE_RGB | COLOUR_WRITE_ALPHA;
        assert!(!state(both, 0).authored_host_fields().partial_colour_write);
        assert!(
            state(COLOUR_WRITE_RGB, 0)
                .authored_host_fields()
                .partial_colour_write
        );
        assert!(state(0, 0).authored_host_fields().partial_colour_write);
    }

    #[test]
    fn line_fill_and_stencil_are_reported() {
        let fields =
            state(LINE_FILL | COLOUR_WRITE_RGB | COLOUR_WRITE_ALPHA, 0x40).authored_host_fields();
        assert!(fields.line_fill);
        assert!(fields.stencil);
        let fields = state(COLOUR_WRITE_RGB | COLOUR_WRITE_ALPHA, 0x3f).authored_host_fields();
        assert!(!fields.line_fill);
        assert!(!fields.stencil);
    }

    #[test]
    fn non_add_and_independent_alpha_blends_are_reported() {
        let both = COLOUR_WRITE_RGB | COLOUR_WRITE_ALPHA;
        let add = state(word0((5, 6, 1), None) | both, 0).authored_host_fields();
        assert!(!add.non_add_blend && !add.independent_alpha_blend);

        let subtract = state(word0((5, 6, 2), None) | both, 0).authored_host_fields();
        assert!(subtract.non_add_blend);

        let separate = state(word0((5, 6, 1), Some((2, 2, 1))) | both, 0).authored_host_fields();
        assert!(separate.independent_alpha_blend);
        assert!(!separate.non_add_blend);

        // no colour operation: neither flag, whatever the alpha half says
        let none = state(word0((0, 0, 0), Some((2, 2, 3))) | both, 0).authored_host_fields();
        assert!(!none.non_add_blend);
        assert!(!none.independent_alpha_blend);
    }

    #[test]
    fn known_blends_are_supported() {
        assert_eq!(
            state(word0((5, 6, 1), None), 0).unsupported_host_fields(),
            None
        );
        assert_eq!(state(0, 0).unsupported_host_fields(), None);
    }

    #[test]
    fn an_unknown_operation_is_unsupported() {
        let fields = state(word0((5, 6, 6), None), 0)
            .unsupported_host_fields()
            .expect("unsupported");
        assert!(fields.unknown_blend_operation);
        assert!(!fields.stencil);
        assert!(fields.any());
    }

    #[test]
    fn an_unknown_factor_is_unsupported_only_when_blending() {
        let fields = state(word0((15, 6, 1), None), 0)
            .unsupported_host_fields()
            .expect("unsupported");
        assert!(fields.unknown_blend_factor);
        // the same factor with no operation is just an opaque draw
        assert_eq!(
            state(word0((15, 6, 0), None), 0).unsupported_host_fields(),
            None
        );
    }

    #[test]
    fn depth_func_and_stencil_come_from_word1() {
        let host = state(0, 0b1100 | 0x100).apply_change_state_1_host();
        assert_eq!(host.depth_func, 3);
        assert_eq!(host.stencil, 0x100);
    }
}
