//! The traces a knife swing sends out: one straight ahead, or a spread of nine over the
//! swing's width and height.

/// The melee button bit.
pub const BUTTON_MELEE: u32 = crate::ps_flags::buttons::BASH_WINDUP;

/// The swing's default width and height (units either side).
pub const LUNGE_BOX_WIDE_STOCK: f32 = 10.0;
pub const LUNGE_BOX_TALL_STOCK: f32 = 10.0;

/// Where each trace ends, as (across, up) shares of the width and height: the middle,
/// four half-way out on the axes, then the four corners.
pub const LUNGE_PROBE_SPREAD: [[f32; 2]; 9] = [
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
pub fn lunge_probe_total(width: f32, tall: f32) -> usize {
    let spread = width > 0.0 || tall > 0.0;
    if spread { LUNGE_PROBE_SPREAD.len() } else { 1 }
}

/// Where one trace ends: `range` along `forward` from the muzzle, moved across by
/// `width * offset[0]` and up by `height * offset[1]`.
#[allow(clippy::too_many_arguments)]
pub fn lunge_probe_tip(
    tip_origin: [f32; 3],
    ahead: [f32; 3],
    side: [f32; 3],
    up: [f32; 3],
    range: f32,
    width: f32,
    height: f32,
    offset: [f32; 2],
) -> [f32; 3] {
    let across = width * offset[0];
    let lift = height * offset[1];
    std::array::from_fn(|i| tip_origin[i] + ahead[i] * range + side[i] * across + up[i] * lift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swing_with_no_size_sends_one_trace() {
        assert_eq!(lunge_probe_total(0.0, 0.0), 1);
        assert_eq!(lunge_probe_total(10.0, 0.0), 9);
        assert_eq!(lunge_probe_total(0.0, 10.0), 9);
    }

    #[test]
    fn a_corner_trace_ends_out_and_up() {
        let end = lunge_probe_tip(
            [0.0, 0.0, 60.0],
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            64.0,
            10.0,
            8.0,
            LUNGE_PROBE_SPREAD[5],
        );
        assert_eq!(end, [64.0, -10.0, 68.0]);
    }
}
