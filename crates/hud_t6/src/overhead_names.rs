//! Overhead names: the name, rank and icon over a player, faded in and out and scaled by range.
//!
//! TODO(fact): BO2's overhead name trace mask, fade times, ranges and sizes are not in the facts
//! yet (QUESTIONS.md 10-21 Clean1 hud overhead names); until answered these are MW2's.

pub const CROSSHAIR_SCAN_DISTANCE: f32 = 8192.0;

pub const OVERHEAD_TRACE_MASK: u32 = 0x0280_1001;

pub const ENEMY_NAME_FADE_MS: i32 = 250;

pub const FRIENDLY_NAME_FADE_OUT_DEFAULT_MS: i32 = 1500;

pub const OVERHEAD_HEAD_LIFT: f32 = 10.0;

pub const OVERHEAD_ORIGIN_FALLBACK_Z: f32 = 82.0;

pub const OVERHEAD_MAX_DISTANCE_DEFAULT: f32 = 10_000.0;

pub const OVERHEAD_NEAR_DISTANCE_DEFAULT: f32 = 64.0;

pub const OVERHEAD_FAR_DISTANCE_DEFAULT: f32 = 512.0;

pub const OVERHEAD_FAR_SCALE_DEFAULT: f32 = 0.7;

pub const OVERHEAD_NAME_SIZE_DEFAULT: f32 = 0.65;

pub const OVERHEAD_ICON_SIZE_DEFAULT: f32 = 1.1;

pub const OVERHEAD_RANK_SIZE_DEFAULT: f32 = 0.5;

#[inline]
pub fn overhead_fade_alpha(
    now_ms: i32,
    start_ms: i32,
    last_seen_ms: i32,
    fade_in_ms: i32,
    fade_out_ms: i32,
) -> f32 {
    let since_last = now_ms.wrapping_sub(last_seen_ms);
    if fade_out_ms < since_last {
        return 0.0;
    }
    let visible_ms = last_seen_ms.wrapping_sub(start_ms);
    if visible_ms < fade_in_ms {
        return visible_ms as f32 / fade_in_ms as f32;
    }
    if since_last < fade_out_ms {
        return (fade_out_ms - since_last) as f32 / fade_out_ms as f32;
    }
    1.0
}

#[inline]
pub fn overhead_distance_scale(
    view_origin: [f32; 3],
    anchor: [f32; 3],
    near_distance: f32,
    far_distance: f32,
    far_scale: f32,
) -> f32 {
    let delta = [
        anchor[0] - view_origin[0],
        anchor[1] - view_origin[1],
        anchor[2] - view_origin[2],
    ];
    let distance_sq = dot(delta, delta);
    if distance_sq < near_distance * near_distance {
        return 1.0;
    }
    if distance_sq <= far_distance * far_distance {
        let t = (libm::sqrtf(distance_sq) - near_distance) / (far_distance - near_distance);
        return (1.0 - t) + t * far_scale;
    }
    far_scale
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
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
