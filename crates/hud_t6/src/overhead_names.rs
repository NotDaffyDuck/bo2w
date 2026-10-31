//! Overhead names: the name, rank and icon over a player, faded in and out and scaled by range.
//!
//! TODO(fact): BO2's overhead name trace mask, fade times, ranges and sizes are not in the facts
//! yet (QUESTIONS.md 10-21 Clean1 hud overhead names); until answered these are MW2's.

pub const NAME_SCAN_RANGE: f32 = 8192.0;

pub const NAMEPLATE_SIGHT_FILTER: u32 = 0x0280_1001;

pub const ENEMY_FADE_IN_MS: i32 = 250;

pub const FRIENDLY_FADE_OUT_MS: i32 = 1500;

pub const NAME_LIFT_ABOVE_HEAD: f32 = 10.0;

pub const NAME_FALLBACK_HEIGHT: f32 = 82.0;

pub const NAME_RANGE_MAX: f32 = 10_000.0;

pub const NAME_RANGE_NEAR: f32 = 64.0;

pub const NAME_RANGE_FAR: f32 = 512.0;

pub const NAME_FAR_SHRINK: f32 = 0.7;

pub const NAME_TEXT_SIZE: f32 = 0.65;

pub const NAME_ICON_SIZE: f32 = 1.1;

pub const NAME_RANK_SIZE: f32 = 0.5;

#[inline]
pub fn overhead_fade_alpha(
    clock_millis: i32,
    shown_at: i32,
    seen_at: i32,
    rise_ms: i32,
    sink_ms: i32,
) -> f32 {
    let unseen_for = clock_millis.wrapping_sub(seen_at);
    let seen_for = seen_at.wrapping_sub(shown_at);
    if unseen_for > sink_ms {
        0.0
    } else if seen_for < rise_ms {
        seen_for as f32 / rise_ms as f32
    } else if unseen_for < sink_ms {
        (sink_ms - unseen_for) as f32 / sink_ms as f32
    } else {
        1.0
    }
}

#[inline]
pub fn overhead_distance_scale(
    eye_origin: [f32; 3],
    head: [f32; 3],
    near: f32,
    far: f32,
    far_size: f32,
) -> f32 {
    let offset = core::array::from_fn::<f32, 3, _>(|axis| head[axis] - eye_origin[axis]);
    let squared = dot(offset, offset);
    if squared < near * near {
        1.0
    } else if squared > far * far {
        far_size
    } else {
        let blend = (libm::sqrtf(squared) - near) / (far - near);
        (1.0 - blend) + blend * far_size
    }
}

#[inline]
fn dot(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_fades_in_then_out() {
        assert_eq!(overhead_fade_alpha(1000, 1000, 1000, 250, 1500), 0.0);
        assert_eq!(overhead_fade_alpha(1125, 1000, 1125, 250, 1500), 0.5);
        assert_eq!(overhead_fade_alpha(3000, 1000, 2250, 250, 1500), 0.5);
        assert_eq!(overhead_fade_alpha(4000, 1000, 2250, 250, 1500), 0.0);
    }

    #[test]
    fn far_names_shrink_to_the_far_scale() {
        let eye = [0.0; 3];
        assert_eq!(overhead_distance_scale(eye, [32.0, 0.0, 0.0], 64.0, 512.0, 0.7), 1.0);
        let mid = overhead_distance_scale(eye, [288.0, 0.0, 0.0], 64.0, 512.0, 0.7);
        assert!((mid - 0.85).abs() < 1e-6, "{mid}");
        assert_eq!(overhead_distance_scale(eye, [600.0, 0.0, 0.0], 64.0, 512.0, 0.7), 0.7);
    }
}
