//! Which of a model's surfaces each level of detail draws.

use core::ops::Range;

/// The surfaces level `tier` draws, as a range into the model's surface lists.
///
/// `span` holds a (first surface, surface count) pair per level. A model that
/// records no spans at all draws every surface at level 0 and nothing at the
/// coarser levels. A level past the last one reads as the last one, and a span
/// that runs past the end of the surfaces is cut short there.
pub(crate) fn detail_facets(span: [(u16, u16); 4], facet_tally: usize, tier: u8) -> Range<usize> {
    if !span.iter().any(|&(_, n)| n != 0) {
        return if tier == 0 { 0..facet_tally } else { 0..0 };
    }
    let (first, count) = span[usize::from(tier).min(span.len() - 1)];
    let first = usize::from(first).min(facet_tally);
    first..first.saturating_add(usize::from(count)).min(facet_tally)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_with_no_spans_draws_everything_at_level_zero_only() {
        let none = [(0, 0); 4];
        assert_eq!(detail_facets(none, 7, 0), 0..7);
        assert_eq!(detail_facets(none, 7, 1), 0..0);
        assert_eq!(detail_facets(none, 7, 9), 0..0);
    }

    #[test]
    fn each_level_reads_its_own_span() {
        let span = [(0, 3), (3, 2), (5, 1), (0, 0)];
        assert_eq!(detail_facets(span, 6, 0), 0..3);
        assert_eq!(detail_facets(span, 6, 1), 3..5);
        assert_eq!(detail_facets(span, 6, 2), 5..6);
    }

    #[test]
    fn a_level_past_the_last_reads_as_the_last() {
        let span = [(0, 3), (3, 2), (5, 1), (6, 2)];
        assert_eq!(detail_facets(span, 8, 3), 6..8);
        assert_eq!(detail_facets(span, 8, 200), 6..8);
    }

    #[test]
    fn a_span_past_the_end_is_cut_short() {
        let span = [(2, 10), (0, 0), (0, 0), (0, 0)];
        assert_eq!(detail_facets(span, 5, 0), 2..5);
        assert_eq!(detail_facets([(9, 4), (0, 0), (0, 0), (0, 0)], 5, 0), 5..5);
    }
}
