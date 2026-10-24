//! Reading an element's colour, size, scale and spin from its visual-state
//! keyframes.
//!
//! The keyframes are packed back to back. Each is two 0x18-byte states, a
//! base and an amplitude, laid out as colour (4 bytes), spin per millisecond,
//! total spin, size 0, size 1, scale (a float each). The element's random
//! roll picks a point between base and amplitude; its age picks where
//! between two keyframes to read.

use crate::runtime::origin::sample_float_range;
use crate::runtime::random::{
    FX_RAND_CH_INITIAL_ROTATION, FX_RAND_CH_ROTATION_DELTA, random_table_f32,
};

/// One state: a base or an amplitude.
const STATE_LEN: usize = 0x18;

/// One keyframe: a base state then an amplitude state.
const KEYFRAME_LEN: usize = 2 * STATE_LEN;

// Where each field sits in a state.
const COLOR_AT: usize = 0x00;
const SPIN_DELTA_AT: usize = 0x04;
const SPIN_TOTAL_AT: usize = 0x08;
const SIZE0_AT: usize = 0x0c;
const SIZE1_AT: usize = 0x10;
const SCALE_AT: usize = 0x14;

/// How far through its life an element is, from 0 to 1. An element with no
/// life is always at its end.
pub fn elem_norm_time(age_msec: i32, life_msec: i32) -> f32 {
    if life_msec <= 0 {
        return 1.0;
    }
    let t = age_msec as f32 / life_msec as f32;
    if t <= 0.0 {
        0.0
    } else if t >= 1.0 {
        1.0
    } else {
        t
    }
}

/// The two keyframes around a moment, and how far between them it is.
struct Span<'a> {
    from: &'a [u8],
    to: &'a [u8],
    frac: f32,
}

impl<'a> Span<'a> {
    /// The span at `norm_time` of the way through `interval_count` intervals,
    /// or `None` when `samples` is too short to hold both keyframes' base
    /// states.
    fn at(samples: &'a [u8], interval_count: u8, norm_time: f32) -> Option<Self> {
        let point = f32::from(interval_count) * norm_time;
        let index = libm::floorf(point).max(0.0) as usize;
        let frac = point - index as f32;

        let from_at = index.checked_mul(KEYFRAME_LEN)?;
        let to_at = from_at.checked_add(KEYFRAME_LEN)?;
        if samples.len() < to_at.checked_add(STATE_LEN)? {
            return None;
        }
        let to_end = to_at.saturating_add(KEYFRAME_LEN).min(samples.len());
        Some(Self {
            from: &samples[from_at..to_at],
            to: &samples[to_at..to_end],
            frac,
        })
    }
}

fn read_f32(bytes: &[u8], at: usize) -> Option<f32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(f32::from_le_bytes(raw))
}

/// A float field of a keyframe's base state.
fn base_f32(keyframe: &[u8], field: usize) -> Option<f32> {
    read_f32(keyframe, field)
}

/// A float field of a keyframe's amplitude state; a keyframe cut short has
/// none.
fn amp_f32(keyframe: &[u8], field: usize) -> f32 {
    read_f32(keyframe, STATE_LEN + field).unwrap_or(0.0)
}

/// How far an element has turned from nothing, summed over the span so far.
fn spin_since_start(span: &Span<'_>, seed: u32, life_msec: f32) -> f32 {
    let roll = random_table_f32(seed, FX_RAND_CH_ROTATION_DELTA);
    let lerp = span.frac;
    let half_sq = lerp * lerp * 0.5;

    let delta_from = base_f32(span.from, SPIN_DELTA_AT).unwrap_or(0.0);
    let delta_from_amp = amp_f32(span.from, SPIN_DELTA_AT);
    let total_from = base_f32(span.from, SPIN_TOTAL_AT).unwrap_or(0.0);
    let total_from_amp = amp_f32(span.from, SPIN_TOTAL_AT);
    let delta_to = base_f32(span.to, SPIN_DELTA_AT).unwrap_or(0.0);
    let delta_to_amp = amp_f32(span.to, SPIN_DELTA_AT);

    let toward_next = (delta_to + delta_to_amp * roll) * half_sq;
    let banked = total_from_amp * roll + total_from;
    let carried = (lerp - half_sq) * (delta_from + roll * delta_from_amp);
    (toward_next + banked + carried) * life_msec
}

/// An element's rotation at `norm_time`: its starting angle plus the spin
/// accumulated so far. `None` if `samples` is too short.
pub fn evaluate_rotation_total(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    seed: u32,
    initial_rotation: [f32; 2],
    life_msec: f32,
) -> Option<f32> {
    let span = Span::at(samples, interval_count, norm_time)?;
    let initial = sample_float_range(
        initial_rotation[0],
        initial_rotation[1],
        random_table_f32(seed, FX_RAND_CH_INITIAL_ROTATION),
    );
    Some(initial + spin_since_start(&span, seed, life_msec))
}

/// One float field blended across the span.
fn blend_f32(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    roll: f32,
    field: usize,
) -> Option<f32> {
    let span = Span::at(samples, interval_count, norm_time)?;
    let from = base_f32(span.from, field)? + amp_f32(span.from, field) * roll;
    let to = base_f32(span.to, field)? + amp_f32(span.to, field) * roll;
    Some(from * (1.0 - span.frac) + to * span.frac)
}

/// An element's first size at `norm_time`, for a random roll of `rand01`.
pub fn evaluate_size0(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    rand01: f32,
) -> Option<f32> {
    blend_f32(samples, interval_count, norm_time, rand01, SIZE0_AT)
}

/// An element's second size at `norm_time`.
pub fn evaluate_size1(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    rand01: f32,
) -> Option<f32> {
    blend_f32(samples, interval_count, norm_time, rand01, SIZE1_AT)
}

/// An element's scale at `norm_time`.
pub fn evaluate_scale(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    rand01: f32,
) -> Option<f32> {
    blend_f32(samples, interval_count, norm_time, rand01, SCALE_AT)
}

/// An element's colour at `norm_time` as blue, green, red, alpha bytes.
pub fn evaluate_color_bgra(
    samples: &[u8],
    interval_count: u8,
    norm_time: f32,
    rand01: f32,
) -> Option<[u8; 4]> {
    let span = Span::at(samples, interval_count, norm_time)?;
    let byte = |keyframe: &[u8], channel: usize, amp: bool| {
        let at = COLOR_AT + channel + if amp { STATE_LEN } else { 0 };
        f32::from(keyframe.get(at).copied().unwrap_or(0))
    };
    let mut out = [0u8; 4];
    for (channel, slot) in out.iter_mut().enumerate() {
        // The first keyframe's base colour is always there.
        span.from.get(COLOR_AT + channel)?;
        let from = byte(span.from, channel, false) * (1.0 - rand01)
            + byte(span.from, channel, true) * rand01;
        let to =
            byte(span.to, channel, false) * (1.0 - rand01) + byte(span.to, channel, true) * rand01;
        let blended = from * (1.0 - span.frac) + to * span.frac;
        *slot = libm::roundf(blended).clamp(0.0, 255.0) as u8;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One state with the given colour and floats.
    fn state(color: [u8; 4], floats: [f32; 5]) -> Vec<u8> {
        let mut bytes = color.to_vec();
        for f in floats {
            bytes.extend(f.to_le_bytes());
        }
        assert_eq!(bytes.len(), STATE_LEN);
        bytes
    }

    /// A keyframe whose base has `size0` and whose amplitude has `amp0`.
    fn keyframe(color: [u8; 4], size0: f32, amp0: f32) -> Vec<u8> {
        let mut bytes = state(color, [0.0, 0.0, size0, 0.0, 0.0]);
        bytes.extend(state([0; 4], [0.0, 0.0, amp0, 0.0, 0.0]));
        bytes
    }

    /// A keyframe at each end of one interval, then a repeat of the last one:
    /// reading the very end of the life still needs a keyframe past it.
    fn keyframes() -> Vec<u8> {
        let mut samples = keyframe([0, 0, 0, 0], 2.0, 4.0);
        let last = keyframe([200, 100, 50, 255], 6.0, 0.0);
        samples.extend(&last);
        samples.extend(&last);
        samples
    }

    #[test]
    fn normal_time_is_the_share_of_life_used_and_stays_inside_zero_to_one() {
        assert_eq!(elem_norm_time(250, 1000), 0.25);
        assert_eq!(elem_norm_time(-50, 1000), 0.0);
        assert_eq!(elem_norm_time(2000, 1000), 1.0);
        assert_eq!(elem_norm_time(10, 0), 1.0);
        assert_eq!(elem_norm_time(10, -5), 1.0);
    }

    #[test]
    fn size_blends_between_the_keyframes() {
        let samples = keyframes();
        assert_eq!(evaluate_size0(&samples, 1, 0.0, 0.0), Some(2.0));
        assert_eq!(evaluate_size0(&samples, 1, 0.5, 0.0), Some(4.0));
        assert_eq!(evaluate_size0(&samples, 1, 1.0, 0.0), Some(6.0));
    }

    #[test]
    fn the_roll_moves_size_toward_the_amplitude() {
        let samples = keyframes();
        assert_eq!(evaluate_size0(&samples, 1, 0.0, 0.5), Some(4.0));
        assert_eq!(evaluate_size0(&samples, 1, 0.0, 1.0), Some(6.0));
    }

    #[test]
    fn short_samples_give_nothing() {
        let samples = keyframes();
        assert_eq!(
            evaluate_size0(&samples[..STATE_LEN * 3 - 1], 1, 0.5, 0.0),
            None
        );
        assert_eq!(evaluate_size0(&[], 1, 0.5, 0.0), None);
        assert_eq!(evaluate_color_bgra(&[], 1, 0.5, 0.0), None);
        assert_eq!(evaluate_rotation_total(&[], 1, 0.5, 0, [0.0; 2], 1.0), None);
    }

    #[test]
    fn a_missing_amplitude_in_the_last_keyframe_counts_as_zero() {
        let mut samples = keyframe([0; 4], 2.0, 4.0);
        samples.extend(keyframe([0; 4], 6.0, 10.0));
        // Keep the second keyframe's base state but drop its amplitude.
        let cut = &samples[..KEYFRAME_LEN + STATE_LEN];
        assert_eq!(evaluate_size0(cut, 1, 0.5, 1.0), Some(6.0));
    }

    #[test]
    fn colour_blends_by_channel_and_rounds() {
        let samples = keyframes();
        assert_eq!(
            evaluate_color_bgra(&samples, 1, 0.0, 0.0),
            Some([0, 0, 0, 0])
        );
        assert_eq!(
            evaluate_color_bgra(&samples, 1, 1.0, 0.0),
            Some([200, 100, 50, 255])
        );
        assert_eq!(
            evaluate_color_bgra(&samples, 1, 0.5, 0.0),
            Some([100, 50, 25, 128])
        );
    }

    #[test]
    fn colour_never_leaves_the_byte_range() {
        let mut samples = keyframe([250, 0, 0, 0], 0.0, 0.0);
        samples.extend(keyframe([250, 0, 0, 0], 0.0, 0.0));
        // Put a large amplitude on the first channel of the first keyframe.
        samples[STATE_LEN] = 255;
        let got = evaluate_color_bgra(&samples, 1, 0.0, 1.0).unwrap();
        assert_eq!(got[0], 255);
    }

    #[test]
    fn rotation_starts_at_the_initial_angle_when_nothing_spins() {
        let samples = keyframes();
        let got = evaluate_rotation_total(&samples, 1, 0.5, 3, [10.0, 0.0], 1000.0);
        assert_eq!(got, Some(10.0));
    }

    #[test]
    fn spin_adds_up_with_the_element_s_life() {
        // Spin of 0.002 per millisecond at the start of the first keyframe,
        // the same at the second, over a 1000 ms life, read at the midpoint.
        let mut first = state([0; 4], [0.002, 0.0, 0.0, 0.0, 0.0]);
        first.extend(state([0; 4], [0.0; 5]));
        let mut second = state([0; 4], [0.002, 0.0, 0.0, 0.0, 0.0]);
        second.extend(state([0; 4], [0.0; 5]));
        first.extend(second);
        let got = evaluate_rotation_total(&first, 1, 0.5, 0, [0.0, 0.0], 1000.0).unwrap();
        // lerp 0.5: 0.002*0.125 + 0.002*(0.5-0.125) = 0.001, times 1000.
        assert!((got - 1.0).abs() < 1e-5, "{got}");
    }
}
