//! Pass blend and colour-write decode from the material pass state bits.
//!
//! The raw numbers are the D3D9 blend enum values the authored state words use;
//! this module has no dependency on the d3d9_* crates.

use bevy::render::render_resource::{
    BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites,
};

// D3DBLEND_* values (4 bits each in the state word).
const BLEND_ZERO: u32 = 1;
const BLEND_ONE: u32 = 2;
const BLEND_SRC_COLOR: u32 = 3;
const BLEND_INV_SRC_COLOR: u32 = 4;
const BLEND_SRC_ALPHA: u32 = 5;
const BLEND_INV_SRC_ALPHA: u32 = 6;
const BLEND_DEST_ALPHA: u32 = 7;
const BLEND_INV_DEST_ALPHA: u32 = 8;
const BLEND_DEST_COLOR: u32 = 9;
const BLEND_INV_DEST_COLOR: u32 = 10;
const BLEND_SRC_ALPHA_SAT: u32 = 11;

// D3DBLENDOP_* values (3 bits in the state word); 0 means blending is off.
const BLEND_OP_NONE: u32 = 0;
const BLEND_OP_ADD: u32 = 1;
const BLEND_OP_SUBTRACT: u32 = 2;
const BLEND_OP_REV_SUBTRACT: u32 = 3;
const BLEND_OP_MIN: u32 = 4;
const BLEND_OP_MAX: u32 = 5;

// State word 0 colour-write enable bits.
const COLOUR_WRITE_RGB_BIT: u32 = 0x0800_0000;
const COLOUR_WRITE_ALPHA_BIT: u32 = 0x1000_0000;

fn factor_to_wgpu(raw: u32) -> Option<BlendFactor> {
    Some(match raw {
        BLEND_ZERO => BlendFactor::Zero,
        BLEND_ONE => BlendFactor::One,
        BLEND_SRC_COLOR => BlendFactor::Src,
        BLEND_INV_SRC_COLOR => BlendFactor::OneMinusSrc,
        BLEND_SRC_ALPHA => BlendFactor::SrcAlpha,
        BLEND_INV_SRC_ALPHA => BlendFactor::OneMinusSrcAlpha,
        BLEND_DEST_ALPHA => BlendFactor::DstAlpha,
        BLEND_INV_DEST_ALPHA => BlendFactor::OneMinusDstAlpha,
        BLEND_DEST_COLOR => BlendFactor::Dst,
        BLEND_INV_DEST_COLOR => BlendFactor::OneMinusDst,
        BLEND_SRC_ALPHA_SAT => BlendFactor::SrcAlphaSaturated,
        _ => return None,
    })
}

fn op_to_wgpu(raw: u32) -> Option<BlendOperation> {
    Some(match raw {
        BLEND_OP_ADD => BlendOperation::Add,
        BLEND_OP_SUBTRACT => BlendOperation::Subtract,
        BLEND_OP_REV_SUBTRACT => BlendOperation::ReverseSubtract,
        BLEND_OP_MIN => BlendOperation::Min,
        BLEND_OP_MAX => BlendOperation::Max,
        _ => return None,
    })
}

/// Decodes one blend component from the low 11 bits of `bits`
/// (src factor, dst factor, operation).
fn component_from_bits(bits: u32) -> BlendComponent {
    BlendComponent {
        src_factor: factor_to_wgpu(bits & 0xf)
            .expect("source blend bits have no wgpu factor"),
        dst_factor: factor_to_wgpu((bits >> 4) & 0xf)
            .expect("destination blend bits have no wgpu factor"),
        operation: op_to_wgpu((bits >> 8) & 0x7)
            .expect("blend op bits have no wgpu operation"),
    }
}

/// Blend state for a pass whose state word 0 is `dword0`; `None` when the pass is opaque.
/// `modulate_pass` forces the multiply (Zero, SrcColor) colour blend.
pub fn blend_state_from_word0(dword0: u32, modulate_pass: bool) -> Option<BlendState> {
    let alpha_of = |colour: BlendComponent| {
        let alpha_bits = (dword0 >> 16) & 0x7ff;
        if alpha_bits & 0x700 == 0 {
            colour
        } else {
            component_from_bits(alpha_bits)
        }
    };
    let multiply_colour = BlendComponent {
        src_factor: BlendFactor::Zero,
        dst_factor: BlendFactor::Src,
        operation: BlendOperation::Add,
    };
    if modulate_pass {
        // Alpha falls back to the multiply colour component when no alpha op is authored.
        return Some(BlendState {
            color: multiply_colour,
            alpha: alpha_of(multiply_colour),
        });
    }
    let blend_op = (dword0 >> 8) & 0b111;
    if blend_op == BLEND_OP_NONE {
        return None;
    }
    let src = dword0 & 0xf;
    let dst = (dword0 >> 4) & 0xf;
    let colour = component_from_bits(dword0);
    let alpha = alpha_of(colour);
    if blend_op == BLEND_OP_ADD && src == BLEND_ZERO && dst == BLEND_SRC_COLOR {
        Some(BlendState {
            color: multiply_colour,
            alpha,
        })
    } else {
        Some(BlendState {
            color: colour,
            alpha,
        })
    }
}

/// Colour write mask from state word 0.
pub fn colour_writes_from_word0(dword0: u32) -> ColorWrites {
    let mut mask_bits = ColorWrites::empty();
    if dword0 & COLOUR_WRITE_RGB_BIT != 0 {
        mask_bits |= ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE;
    }
    if dword0 & COLOUR_WRITE_ALPHA_BIT != 0 {
        mask_bits |= ColorWrites::ALPHA;
    }
    mask_bits
}
