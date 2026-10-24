//! The traces a knife swing sends out: one straight ahead, or a spread of nine over the
//! swing's width and height.

/// The melee button bit.
pub const BUTTON_MELEE: u32 = crate::ps_flags::buttons::MELEE_CHARGE;

/// The swing's default width and height (units either side).
pub const PLAYER_MELEE_WIDTH_DEFAULT: f32 = 10.0;
pub const PLAYER_MELEE_HEIGHT_DEFAULT: f32 = 10.0;

/// Where each trace ends, as (across, up) shares of the width and height: the middle,
/// four half-way out on the axes, then the four corners.
pub const MELEE_TRACE_OFFSETS: [[f32; 2]; 9] = [
    [0.0, 0.0],
    [0.5, 0.0],
    [-0.5, 0.0],
    [0.0, 0.5],
    [0.0, -0.5],
    [1.0, 1.0],
    [-1.0, 1.0],
    [1.0, -1.0],
    [-1.0, -1.0],
];

/// How many traces a swing sends: the whole spread when it has any width or height.
pub fn melee_trace_count(width: f32, height: f32) -> usize {
    let spread = width > 0.0 || height > 0.0;
    if spread { MELEE_TRACE_OFFSETS.len() } else { 1 }
}

/// Where one trace ends: `range` along `forward` from the muzzle, moved across by
/// `width * offset[0]` and up by `height * offset[1]`.
#[allow(clippy::too_many_arguments)]
pub fn melee_trace_end(
    muzzle: [f32; 3],
    forward: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    range: f32,
    width: f32,
    height: f32,
    offset: [f32; 2],
) -> [f32; 3] {
    let across = width * offset[0];
    let lift = height * offset[1];
    std::array::from_fn(|i| muzzle[i] + forward[i] * range + right[i] * across + up[i] * lift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swing_with_no_size_sends_one_trace() {
        assert_eq!(melee_trace_count(0.0, 0.0), 1);
        assert_eq!(melee_trace_count(10.0, 0.0), 9);
        assert_eq!(melee_trace_count(0.0, 10.0), 9);
    }

    #[test]
    fn a_corner_trace_ends_out_and_up() {
        let end = melee_trace_end(
            [0.0, 0.0, 60.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            64.0,
            10.0,
            8.0,
            MELEE_TRACE_OFFSETS[5],
        );
        assert_eq!(end, [64.0, -10.0, 68.0]);
    }
}
