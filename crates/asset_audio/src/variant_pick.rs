// BO2 unknown: Black Ops II's alias variant pick (its random step and weighting) is not
// confirmed yet, so this keeps the MW2 pick exactly as it was.

pub const SND_LCG_MUL: u32 = 0x0003_43fd;

pub const SND_LCG_ADD: u32 = 0x0026_9ec3;

pub const SND_LCG_UNIT_SCALE: f32 = 32_768.0;

pub fn advance_lcg(state: &mut u32) {
    let stepped = state.wrapping_mul(SND_LCG_MUL);
    *state = stepped.wrapping_add(SND_LCG_ADD);
}

pub fn unit_random(state: &mut u32) -> f32 {
    advance_lcg(state);
    let bits = (*state >> 16) & 0x7fff;
    bits as f32 / SND_LCG_UNIT_SCALE
}

pub fn mix_interval(a: f32, b: f32, along: f32) -> f32 {
    let (lo, hi) = match a < b {
        true => (a, b),
        false => (b, a),
    };
    let frac = along.clamp(0.0, 1.0);
    lo + (hi - lo) * frac
}

pub fn weighted_pick(chances: &[f32], roll: &mut u32, skip: Option<usize>) -> usize {
    debug_assert!(!chances.is_empty());
    let last = chances.len().saturating_sub(1);
    // An avoided slot counts as weightless, unless it is the only one.
    let effective = |(slot, &weight): (usize, &f32)| -> f32 {
        if skip == Some(slot) && chances.len() > 1 { 0.0 } else { weight }
    };
    let sum: f32 = chances.iter().enumerate().map(effective).sum();
    if sum <= 1e-8 {
        let scaled = unit_random(roll) * chances.len() as f32;
        return (scaled as usize).min(last);
    }
    let mut remaining = unit_random(roll) * sum;
    chances
        .iter()
        .enumerate()
        .map(effective)
        .position(|weight| {
            remaining -= weight;
            remaining <= 0.0
        })
        .unwrap_or(last)
}

#[cfg(test)]
mod pick_tests {
    use super::*;

    fn run(weights: &[f32], avoid: Option<usize>) -> Vec<usize> {
        let mut rng = 12345u32;
        (0..12).map(|_| weighted_pick(weights, &mut rng, avoid)).collect()
    }

    #[test]
    fn avoided_slot_is_never_chosen_when_others_exist() {
        assert!(run(&[1.0, 1.0, 1.0], Some(1)).iter().all(|&s| s != 1));
    }

    #[test]
    fn single_slot_ignores_avoid() {
        assert_eq!(run(&[0.5], Some(0)), vec![0; 12]);
    }

    #[test]
    fn zero_weights_fall_back_to_uniform_in_range() {
        assert!(run(&[0.0, 0.0, 0.0, 0.0], None).iter().all(|&s| s < 4));
    }

    #[test]
    fn heavy_slot_wins_every_time() {
        assert_eq!(run(&[0.0, 5.0, 0.0], None), vec![1; 12]);
    }

    #[test]
    fn sequence_is_pinned() {
        assert_eq!(run(&[1.0, 2.0, 3.0], None), vec![1, 2, 2, 2, 1, 2, 2, 0, 2, 2, 1, 1]);
        assert_eq!(run(&[1.0, 2.0, 3.0], Some(2)), vec![0, 1, 1, 1, 0, 1, 1, 0, 1, 1, 1, 1]);
    }
}
