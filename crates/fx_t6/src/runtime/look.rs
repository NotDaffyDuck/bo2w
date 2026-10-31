//! Reading an element's colour, size, scale and spin from its visual-state
//! keyframes.
//!
//! The keyframes are packed back to back. Each is two 0x18-byte states, a
//! base and an amplitude, laid out as colour (4 bytes), spin per millisecond,
//! total spin, size 0, size 1, scale (a float each). The element's random
//! roll picks a point between base and amplitude; its age picks where
//! between two keyframes to read.

use crate::runtime::birth_spot::sample_float_range;
use crate::runtime::dice::DICE_LANE_FIRST_ROLL;
use crate::runtime::dice::random_table_f32;
use super::dice::DICE_LANE_ROLL_STEP;

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
pub fn elem_norm_time(lived_ms: i32, particle_lifespan_ms: i32) -> f32 {
    match particle_lifespan_ms {
        ..=0 => 1.0,
        life => {
            let through = lived_ms as f32 / life as f32;
            // Before birth reads as 0 (never -0), past death as 1.
            if through > 0.0 { through.min(1.0) } else { 0.0 }
        }
    }
}

/// The two keyframes around a moment, and how far between them it is.
struct Span<'a> {
    from: &'a [u8],
    to: &'a [u8],
    frac: f32,
}

impl<'a> Span<'a> {
    /// The span at `norm_time` of the way through `steps` intervals,
    /// or `None` when `samples` is too short to hold both keyframes' base
    /// states.
    fn at(samples: &'a [u8], steps: u8, norm_time: f32) -> Option<Self> {
        let point = f32::from(steps) * norm_time;
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

fn take_real(buf: &[u8], at: usize) -> Option<f32> {
    let raw: [u8; 4] = buf.get(at..4 + at)?.try_into().ok()?;
    Some(f32::from_le_bytes(raw))
}

/// A float field of a keyframe's amplitude state; a keyframe cut short has
/// none.
fn amp_f32(keyframe: &[u8], field: usize) -> f32 {
    take_real(keyframe, STATE_LEN + field).unwrap_or(0.0)
}

/// How far an element has turned from nothing, summed over the span so far.
fn spin_since_start(span: &Span<'_>, seed: u32, particle_lifespan_ms: f32) -> f32 {
    let roll = random_table_f32(seed, DICE_LANE_ROLL_STEP);
    let lerp = span.frac;
    let half_sq = lerp * lerp * 0.5;

    let delta_from = take_real(span.from, SPIN_DELTA_AT).unwrap_or(0.0);
    let delta_from_amp = amp_f32(span.from, SPIN_DELTA_AT);
    let total_from = take_real(span.from, SPIN_TOTAL_AT).unwrap_or(0.0);
    let total_from_amp = amp_f32(span.from, SPIN_TOTAL_AT);
    let delta_to = take_real(span.to, SPIN_DELTA_AT).unwrap_or(0.0);
    let delta_to_amp = amp_f32(span.to, SPIN_DELTA_AT);

    let toward_next = (delta_to + delta_to_amp * roll) * half_sq;
    let banked = total_from_amp * roll + total_from;
    let carried = (lerp - half_sq) * (delta_from + roll * delta_from_amp);
    (toward_next + banked + carried) * particle_lifespan_ms
}

/// An element's rotation at `life_frac`: its starting angle plus the spin
/// accumulated so far. `None` if `keys` is too short.
pub fn evaluate_rotation_total(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    seed: u32,
    birth_spin_angle: [f32; 2],
    particle_lifespan_ms: f32,
) -> Option<f32> {
    let span = Span::at(keys, spans, life_frac)?;
    let [start_base, start_amp] = birth_spin_angle;
    let initial = sample_float_range(start_base, start_amp, random_table_f32(seed, DICE_LANE_FIRST_ROLL));
    Some(initial + spin_since_start(&span, seed, particle_lifespan_ms))
}

/// One float field blended across the span.
fn blend_f32(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    roll: f32,
    lane: usize,
) -> Option<f32> {
    let span = Span::at(keys, spans, life_frac)?;
    let from = take_real(span.from, lane)? + amp_f32(span.from, lane) * roll;
    let to = take_real(span.to, lane)? + amp_f32(span.to, lane) * roll;
    Some(from * (1.0 - span.frac) + to * span.frac)
}

/// An element's first size at `life_frac`, for a random roll of `roll`.
pub fn evaluate_size0(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    roll: f32,
) -> Option<f32> {
    blend_f32(keys, spans, life_frac, roll, SIZE0_AT)
}

/// An element's second size at `life_frac`.
pub fn evaluate_size1(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    roll: f32,
) -> Option<f32> {
    blend_f32(keys, spans, life_frac, roll, SIZE1_AT)
}

/// An element's scale at `life_frac`.
pub fn evaluate_scale(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    roll: f32,
) -> Option<f32> {
    blend_f32(keys, spans, life_frac, roll, SCALE_AT)
}

/// An element's colour at `life_frac` as blue, green, red, alpha bytes.
pub fn evaluate_color_bgra(
    keys: &[u8],
    spans: u8,
    life_frac: f32,
    roll: f32,
) -> Option<[u8; 4]> {
    let span = Span::at(keys, spans, life_frac)?;
    let byte = |keyframe: &[u8], channel: usize, amp: bool| {
        let at = COLOR_AT + channel + if amp { STATE_LEN } else { 0 };
        f32::from(keyframe.get(at).copied().unwrap_or(0))
    };
    // The first keyframe's base colour must be all there.
    span.from.get(COLOR_AT + 3)?;
    let rolled = |keyframe: &[u8], channel: usize| {
        byte(keyframe, channel, false) * (1.0 - roll) + byte(keyframe, channel, true) * roll
    };
    Some(core::array::from_fn(|channel| {
        let blended = rolled(span.from, channel) * (1.0 - span.frac) + rolled(span.to, channel) * span.frac;
        libm::roundf(blended).max(0.0).min(255.0) as u8
    }))
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

    /// A keyframe whose base has `start_extent` and whose amplitude has `amp0`.
    fn keyframe(color: [u8; 4], start_extent: f32, amp0: f32) -> Vec<u8> {
        let mut bytes = state(color, [0.0, 0.0, start_extent, 0.0, 0.0]);
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
