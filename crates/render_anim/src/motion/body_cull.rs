/// The cells a camera may see, as the dynamic-entity pass was handed them: a
/// bit array over cell numbers (bit 0 of word 0 is cell 0), or "all of them".
#[derive(Clone, Copy, Debug)]
pub struct CellVisAdmit<'a> {
    pub tokens: &'a [u32],
    pub slot_count: usize,
    pub sight_all: bool,
}

impl CellVisAdmit<'_> {
    /// Cells past the end of the array are not visible (unless `sight_all`).
    pub fn holds_cell(self, cell_in: usize) -> bool {
        self.sight_all
            || self
                .tokens
                .get(cell_in / 32)
                .is_some_and(|word| (word >> (cell_in % 32)) & 1 == 1)
    }
}

mod tests {
    #![cfg(test)]
    use super::*;

    fn vis(words: &[u32], sight_all: bool) -> CellVisAdmit<'_> {
        CellVisAdmit {
            tokens: words,
            slot_count: words.len() * 32,
            sight_all,
        }
    }

    #[test]
    fn bits_count_from_the_low_end_of_each_word() {
        let words = [0b101, 1 << 31];
        let v = vis(&words, false);
        assert!(v.holds_cell(0) && !v.holds_cell(1) && v.holds_cell(2));
        assert!(!v.holds_cell(32) && v.holds_cell(63));
    }

    #[test]
    fn a_cell_past_the_array_is_not_visible() {
        assert!(!vis(&[u32::MAX], false).holds_cell(32));
        assert!(!vis(&[], false).holds_cell(0));
    }

    #[test]
    fn vis_all_admits_everything_even_with_no_words() {
        let v = vis(&[], true);
        assert!(v.holds_cell(0) && v.holds_cell(10_000));
    }
}
