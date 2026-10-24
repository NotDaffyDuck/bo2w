/// `XAnimCalcRelDeltaParts`' translation: the root-motion move of one leaf
/// update from 0..1 time `start` to `end`, `(pos(end) - pos(start)) *
/// weight`, where `pos` samples the anim's delta track. When the leaf
/// wrapped (`end < start`) one whole span, `pos(1) - pos(0)`, is added.
///
/// The game then turns the xy into the start yaw frame (inferred reading of
/// the calls); that turn is the caller's.
pub fn rel_delta_trans(pos: impl Fn(f32) -> [f32; 3], start: f32, end: f32, weight: f32) -> [f32; 3] {
    let (from, to) = (pos(start), pos(end));
    let span = if end < start {
        let (head, tail) = (pos(0.0), pos(1.0));
        core::array::from_fn(|i| tail[i] - head[i])
    } else {
        [0.0; 3]
    };
    core::array::from_fn(|i| (to[i] - from[i] + span[i]) * weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(t: f32) -> [f32; 3] {
        // Starts 2 units out and walks 10 over the anim.
        [2.0 + 10.0 * t, 0.0, 1.0]
    }

    #[test]
    fn the_move_is_the_difference_times_the_weight() {
        assert_eq!(rel_delta_trans(walk, 0.25, 0.75, 0.5), [2.5, 0.0, 0.0]);
    }

    #[test]
    fn a_wrap_adds_one_whole_span() {
        // 0.75 -> 1.0 is 2.5, 0.0 -> 0.25 is 2.5.
        assert_eq!(rel_delta_trans(walk, 0.75, 0.25, 1.0), [5.0, 0.0, 0.0]);
    }
}
