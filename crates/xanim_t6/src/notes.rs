/// What one leaf update fires (`XAnimProcessClientNotify`): the indices of
/// the notes, in the order they fire, and whether the synthetic "end"
/// notetrack fires after them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FiredNotes {
    pub notes: Vec<usize>,
    pub end: bool,
}

/// The notes (0..1 times, in stored order) an update from `old` to `cur`
/// fires. A note fires when `old < note <= cur`; one exactly at `old` only
/// when `old == 0` or on the leaf's first update. A looped anim that
/// wrapped (`cur < old`) fires the notes after `old` to the end, then those
/// from the start up to `cur`. A non-looped anim reaching 1.0 fires the
/// notes after `old`, then "end"; when `old` was already 1.0 only "end"
/// fires. A non-looped anim going backwards fires nothing.
pub fn fired_notes(
    note_times: &[f32],
    old: f32,
    cur: f32,
    wraps: bool,
    first_update: bool,
) -> FiredNotes {
    let at_old_fires = old == 0.0 || first_update;
    let after_old = |t: f32| t > old || (t == old && at_old_fires);
    let mut fired = FiredNotes::default();
    if cur < old {
        if wraps {
            fired.notes.extend((0..note_times.len()).filter(|&i| after_old(note_times[i])));
            fired.notes.extend((0..note_times.len()).filter(|&i| note_times[i] <= cur));
        }
        return fired;
    }
    if !wraps && cur >= 1.0 {
        fired.end = true;
        if old >= 1.0 {
            return fired;
        }
    }
    fired.notes.extend((0..note_times.len()).filter(|&i| {
        let t = note_times[i];
        after_old(t) && t <= cur
    }));
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: [f32; 4] = [0.0, 0.25, 0.5, 1.0];

    #[test]
    fn notes_in_the_step_fire() {
        assert_eq!(fired_notes(&NOTES, 0.1, 0.5, false, false).notes, [1, 2]);
        // At old: only from 0 or on the first update.
        assert_eq!(fired_notes(&NOTES, 0.25, 0.4, false, false).notes, [] as [usize; 0]);
        assert_eq!(fired_notes(&NOTES, 0.25, 0.4, false, true).notes, [1]);
        assert_eq!(fired_notes(&NOTES, 0.0, 0.3, true, false).notes, [0, 1]);
    }

    #[test]
    fn a_wrap_fires_the_tail_then_the_head() {
        let fired = fired_notes(&NOTES, 0.4, 0.3, true, false);
        assert_eq!(fired.notes, [2, 3, 0, 1]);
        assert!(!fired.end);
        assert_eq!(fired_notes(&NOTES, 0.4, 0.3, false, false), FiredNotes::default());
    }

    #[test]
    fn the_end_fires_end_once_the_notes_are_done() {
        let fired = fired_notes(&NOTES, 0.4, 1.0, false, false);
        assert_eq!(fired, FiredNotes { notes: vec![2, 3], end: true });
        let again = fired_notes(&NOTES, 1.0, 1.0, false, false);
        assert_eq!(again, FiredNotes { notes: vec![], end: true });
        assert!(!fired_notes(&NOTES, 0.4, 1.0, true, false).end);
    }
}
