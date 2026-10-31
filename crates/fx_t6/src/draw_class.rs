//! The gates an elem passes before it is drawn, and the draw constants.

use super::flags::{
    FX_ELEM_DISABLE_ALPHA_CULLING, FX_ELEM_DRAW_PAST_FOG, FX_ELEM_DRAW_WITH_VIEWMODEL,
    FX_ELEM_MATURE_CONTENT, FX_ELEM_USE_THERMAL,
};

/// Both sizes must be above 4/255.
pub const FX_DRAW_MIN_SIZE: f32 = 0.015_686_275;

/// The alpha byte must be above this (above 0 with flag 0x40000).
pub const FX_DRAW_MIN_ALPHA: u8 = 6;

pub const FX_FRUSTUM_PLANES: usize = 6;

/// Draw-past-fog elems skip the far plane.
pub const FX_FRUSTUM_PLANES_PAST_FOG: usize = 5;

/// Model elems are drawn through the scene model filter with this distance cap.
pub const FX_MODEL_DRAW_DIST: f32 = 4096.0;

/// After the at-rest time a model's time eases for 300 ms, then clamps at
/// the at-rest time plus 150 ms.
pub const FX_MODEL_AT_REST_EASE_MS: i32 = 300;
pub const FX_MODEL_AT_REST_CLAMP_MS: i32 = 150;

pub const FX_DEG_TO_RAD: f32 = 0.017_453_292;

/// Where an elem stopped in the draw gates, in BO2's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxDrawGate {
    NoVisStates,
    NotStarted,
    MatureContent,
    TooSmall,
    Transparent,
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxDrawGateInput {
    pub elem_flags: u32,
    pub look_keyframe_total: u8,
    pub msec_draw: i32,
    pub elem_msec_begin: i32,
    pub mature_content_allowed: bool,
    pub size: [f32; 2],
    pub fade_byte: u8,
    /// fx_alphacull_enable
    pub alpha_cull: bool,
}

/// The shared gates of FX_ShouldDrawElement before the per-type cull. The
/// rotation-axis decode (flag 0x800000) sits between the mature-content and
/// size gates and does not stop the draw.
pub fn draw_gate(i: &FxDrawGateInput) -> FxDrawGate {
    if i.look_keyframe_total == 0 {
        return FxDrawGate::NoVisStates;
    }
    if i.msec_draw < i.elem_msec_begin {
        return FxDrawGate::NotStarted;
    }
    if i.elem_flags & FX_ELEM_MATURE_CONTENT != 0 && !i.mature_content_allowed {
        return FxDrawGate::MatureContent;
    }
    if !size_passes(i.size) {
        return FxDrawGate::TooSmall;
    }
    if i.alpha_cull && !alpha_passes(i.fade_byte, i.elem_flags) {
        return FxDrawGate::Transparent;
    }
    FxDrawGate::Draw
}

#[inline]
pub fn size_passes(size: [f32; 2]) -> bool {
    size[0] > FX_DRAW_MIN_SIZE && size[1] > FX_DRAW_MIN_SIZE
}

#[inline]
pub fn alpha_passes(alpha: u8, elem_flags: u32) -> bool {
    let min = if elem_flags & FX_ELEM_DISABLE_ALPHA_CULLING != 0 {
        0
    } else {
        FX_DRAW_MIN_ALPHA
    };
    alpha > min
}

#[inline]
pub fn view_wall_total(elem_flags: u32) -> usize {
    if elem_flags & FX_ELEM_DRAW_PAST_FOG != 0 {
        FX_FRUSTUM_PLANES_PAST_FOG
    } else {
        FX_FRUSTUM_PLANES
    }
}

/// A sprite's cull sphere radius: the larger of its two sizes.
// BO2 unknown: with flag 0x200000 BO2 enlarges this by the billboard-pivot
// extents, but how is not in the facts (see facts/QUESTIONS.md).
#[inline]
pub fn sprite_cull_radius(size: [f32; 2]) -> f32 {
    size[0].max(size[1])
}

#[inline]
pub fn draws_with_viewmodel(elem_flags: u32) -> bool {
    elem_flags & FX_ELEM_DRAW_WITH_VIEWMODEL != 0
}

/// Infrared on a model elem drawn in the thermal view with flag 0x20000.
#[inline]
pub fn draws_infrared(elem_flags: u32, thermal_view: bool) -> bool {
    thermal_view && elem_flags & FX_ELEM_USE_THERMAL != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> FxDrawGateInput {
        FxDrawGateInput {
            elem_flags: 0,
            look_keyframe_total: 1,
            msec_draw: 100,
            elem_msec_begin: 50,
            mature_content_allowed: true,
            size: [1.0, 1.0],
            fade_byte: 255,
            alpha_cull: true,
        }
    }

    #[test]
    fn gates_in_order() {
        assert_eq!(draw_gate(&input()), FxDrawGate::Draw);
        let mut i = input();
        i.look_keyframe_total = 0;
        i.size = [0.0, 0.0];
        assert_eq!(draw_gate(&i), FxDrawGate::NoVisStates);
        let mut i = input();
        i.msec_draw = 49;
        assert_eq!(draw_gate(&i), FxDrawGate::NotStarted);
        let mut i = input();
        i.elem_flags = FX_ELEM_MATURE_CONTENT;
        i.mature_content_allowed = false;
        assert_eq!(draw_gate(&i), FxDrawGate::MatureContent);
        let mut i = input();
        i.size = [1.0, 4.0 / 255.0];
        assert_eq!(draw_gate(&i), FxDrawGate::TooSmall);
        let mut i = input();
        i.fade_byte = 6;
        assert_eq!(draw_gate(&i), FxDrawGate::Transparent);
        i.alpha_cull = false;
        assert_eq!(draw_gate(&i), FxDrawGate::Draw);
    }

    #[test]
    fn alpha_and_planes() {
        assert!(alpha_passes(7, 0));
        assert!(!alpha_passes(6, 0));
        assert!(alpha_passes(1, FX_ELEM_DISABLE_ALPHA_CULLING));
        assert!(!alpha_passes(0, FX_ELEM_DISABLE_ALPHA_CULLING));
        assert_eq!(view_wall_total(FX_ELEM_DRAW_PAST_FOG), 5);
        assert_eq!(view_wall_total(0), 6);
        assert!(draws_with_viewmodel(0x1000));
        assert!(draws_infrared(0x2_0000, true));
        assert!(!draws_infrared(0x2_0000, false));
        assert_eq!(sprite_cull_radius([2.0, 3.0]), 3.0);
    }
}
