/// The cells a camera may see, as the dynamic-entity pass was handed them: a
/// bit array over cell numbers (bit 0 of word 0 is cell 0), or "all of them".
#[derive(Clone, Copy, Debug)]
pub struct AdmittedCellVis<'a> {
    pub words: &'a [u32],
    pub cell_count: usize,
    pub vis_all: bool,
}

impl AdmittedCellVis<'_> {
    /// Cells past the end of the array are not visible (unless `vis_all`).
    pub fn contains(self, cell: usize) -> bool {
        self.vis_all
            || self
                .words
                .get(cell / 32)
                .is_some_and(|word| (word >> (cell % 32)) & 1 == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vis(words: &[u32], vis_all: bool) -> AdmittedCellVis<'_> {
        AdmittedCellVis {
            words,
            cell_count: words.len() * 32,
            vis_all,
        }
    }

    #[test]
    fn bits_count_from_the_low_end_of_each_word() {
        let words = [0b101, 1 << 31];
        let v = vis(&words, false);
        assert!(v.contains(0) && !v.contains(1) && v.contains(2));
        assert!(!v.contains(32) && v.contains(63));
    }

    #[test]
    fn a_cell_past_the_array_is_not_visible() {
        assert!(!vis(&[u32::MAX], false).contains(32));
        assert!(!vis(&[], false).contains(0));
    }

    #[test]
    fn vis_all_admits_everything_even_with_no_words() {
        let v = vis(&[], true);
        assert!(v.contains(0) && v.contains(10_000));
    }
}
