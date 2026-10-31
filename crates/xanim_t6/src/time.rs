/// Where a non-looped anim parks on the step that reaches its end:
/// 0.9999999 (the f32 just under 1 the game stores, bits `0x3f7ffffe`).
pub const NONLOOP_END_PARK: f32 = f32::from_bits(0x3f7f_fffe);

/// The time a looped anim wraps back to: `loopEntryTime`, 0 for every anim
/// at load.
pub const LOOP_ENTRY_TIME: f32 = 0.0;

/// One leaf's time step (`XAnimUpdateTimeAndNotetrack`). `prev_t` and
/// `cycle` are the leaf's 0..1 time and cycle count at the frame start,
/// `frequency` is the anim's `framerate / numframes`. The step is `rate *
/// frequency * dt`. A looped anim wraps past 1 by `1 - loopEntryTime`,
/// counting a cycle each time; a non-looped one parks at
/// [`NONLOOP_END_PARK`], then 1.0 on the next step that reaches the end.
///
/// `None`: nothing changes, either because the step is 0 or because the
/// time would run backwards by more than the cycles gained (the game drops
/// that update, so a negative rate does not play).
pub fn advance_leaf_time(
    prev_t: f32,
    cycle: i16,
    rate: f32,
    hz: f32,
    dt: f32,
    repeats: bool,
) -> Option<(f32, i16)> {
    let step = dt * hz * rate;
    if !step.is_finite() || step == 0.0 {
        return None;
    }
    let mut time = prev_t + step;
    let mut laps: i16 = 0;
    match (time >= 1.0, repeats) {
        (false, _) => {}
        (true, true) => {
            while time >= 1.0 {
                time -= 1.0 - LOOP_ENTRY_TIME;
                laps = laps.wrapping_add(1);
            }
        }
        (true, false) => {
            time = if prev_t < NONLOOP_END_PARK {
                NONLOOP_END_PARK
            } else {
                1.0
            };
        }
    }
    // A step that runs time backwards further than the laps it gained is dropped.
    if prev_t - time > f32::from(laps) {
        None
    } else {
        Some((time, cycle.wrapping_add(laps)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looped_time_wraps_and_counts_cycles() {
        assert_eq!(
            advance_leaf_time(0.25, 0, 1.0, 2.0, 0.25, true),
            Some((0.75, 0))
        );
        assert_eq!(
            advance_leaf_time(0.75, 3, 1.0, 2.0, 0.25, true),
            Some((0.25, 4))
        );
        assert_eq!(
            advance_leaf_time(0.5, 0, 1.0, 1.0, 2.0, true),
            Some((0.5, 2))
        );
    }

    #[test]
    fn non_looped_time_parks_then_ends() {
        let park = advance_leaf_time(0.9, 0, 1.0, 1.0, 0.5, false);
        assert_eq!(park, Some((NONLOOP_END_PARK, 0)));
        assert_eq!(
            advance_leaf_time(NONLOOP_END_PARK, 0, 1.0, 1.0, 0.5, false),
            Some((1.0, 0))
        );
        assert!((NONLOOP_END_PARK - 0.999_999_9).abs() < 1e-8);
    }

    #[test]
    fn no_step_or_a_backwards_step_changes_nothing() {
        assert_eq!(advance_leaf_time(0.5, 0, 0.0, 1.0, 0.1, true), None);
        assert_eq!(advance_leaf_time(0.5, 0, -1.0, 1.0, 0.1, true), None);
        assert_eq!(advance_leaf_time(0.5, 0, -1.0, 1.0, 0.1, false), None);
    }
}
