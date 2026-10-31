//! Cell bit sets in two layouts: the walk's own visible-cell set (bit `n`
//! is the low-end bit `n % 32` of word `n / 32`) and the map's stored cell
//! tables, which count from the high end of each word.

const WORD_BITS: usize = u32::BITS as usize;
const TOP_BIT: u32 = 1 << 31;

/// Word index and mask for bit `index`, counting from each word's low end.
#[inline]
fn low_first(index: usize) -> (usize, u32) {
    (index / WORD_BITS, 1u32 << (index % WORD_BITS))
}

/// Word index and mask for bit `index`, counting from each word's high end.
#[inline]
fn high_first(index: usize) -> (usize, u32) {
    (index / WORD_BITS, TOP_BIT >> (index % WORD_BITS))
}

#[inline]
fn test_bit(bits: &[u32], (word, mask): (usize, u32)) -> bool {
    matches!(bits.get(word), Some(w) if w & mask != 0)
}

#[inline]
fn raise_bit(bits: &mut [u32], (word, mask): (usize, u32)) {
    if let Some(w) = bits.get_mut(word) {
        *w |= mask;
    }
}

/// How many 32-bit words hold `count` cell bits.
#[must_use]
pub fn word_span(count: usize) -> usize {
    count.div_ceil(WORD_BITS)
}

/// Whether bit `index` of a walk-layout set (low end first) is set. Out of
/// range reads as unset. The map's own stored tables count from the high
/// end instead: read those with `msb_get`.
#[inline]
#[must_use]
pub fn bit_get(bitset: &[u32], ix: usize) -> bool {
    test_bit(bitset, low_first(ix))
}

/// Sets bit `index` of a walk-layout set; out of range is ignored.
#[inline]
pub fn bit_set(bit_row: &mut [u32], slot_ix: usize) {
    raise_bit(bit_row, low_first(slot_ix));
}

/// Whether bit `index` of a stored cell table (high end first) is set.
#[inline]
#[must_use]
pub fn top_bit_get(flag_words: &[u32], bit_ix: usize) -> bool {
    test_bit(flag_words, high_first(bit_ix))
}

/// Sets bit `index` of a stored cell table; out of range is ignored.
#[inline]
pub fn top_bit_set(cell_mask: &mut [u32], pos_ix: usize) {
    raise_bit(cell_mask, high_first(pos_ix));
}

/// The indices of the set bits below `flag_total` of a stored cell table, ascending.
#[must_use]
pub fn msb_iter(bits: &[u32], flag_total: usize) -> HighBitFirstBitWalker<'_> {
    HighBitFirstBitWalker { unread: bits, next_base: 0, word_origin: 0, pending: 0, cap: flag_total }
}

/// Ascending set-bit indices of a high-end-first table; see `msb_iter`.
#[derive(Clone, Copy, Debug)]
pub struct HighBitFirstBitWalker<'a> {
    /// Words not yet loaded.
    unread: &'a [u32],
    /// Index of the first bit of the next word to load.
    next_base: usize,
    /// Index of the first bit of the loaded word.
    word_origin: usize,
    /// Bits of the loaded word not yet handed out.
    pending: u32,
    cap: usize,
}

impl Iterator for HighBitFirstBitWalker<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        while self.pending == 0 {
            let (&word, tail) = self.unread.split_first()?;
            if self.next_base >= self.cap && self.next_base > 0 {
                return None;
            }
            self.unread = tail;
            self.word_origin = self.next_base;
            self.next_base += WORD_BITS;
            self.pending = word;
        }
        let offset = self.pending.leading_zeros() as usize;
        self.pending ^= TOP_BIT >> offset;
        let found = self.word_origin + offset;
        if found < self.cap {
            Some(found)
        } else {
            // Everything after this lies past the limit too.
            self.pending = 0;
            self.unread = &[];
            None
        }
    }
}

/// A walk-layout cell bit set over words the caller owns.
#[derive(Debug)]
pub struct VisibilityBitView<'a> {
    cells: &'a mut [u32],
}

impl<'a> VisibilityBitView<'a> {
    pub fn new(cells: &'a mut [u32]) -> Self {
        VisibilityBitView { cells }
    }

    /// Unsets every bit.
    pub fn clear(&mut self) {
        self.cells.iter_mut().for_each(|w| *w = 0);
    }

    pub fn set(&mut self, cell: usize) {
        bit_set(self.cells, cell);
    }

    #[must_use]
    pub fn get(&self, cell: usize) -> bool {
        bit_get(self.cells, cell)
    }

    /// How many bits are set.
    #[must_use]
    pub fn set_total(&self) -> u32 {
        self.cells.iter().fold(0, |total, w| total + w.count_ones())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_fill_words_from_the_low_end() {
        let mut mask_words = [0u32; 2];
        let mut vis = VisibilityBitView::new(&mut mask_words);
        vis.set(0);
        vis.set(33);
        vis.set(64);
        assert!(vis.get(33) && !vis.get(32));
        assert_eq!(vis.set_total(), 2);
        assert_eq!(mask_words, [1, 2]);
        assert_eq!(word_span(33), 2);
    }

    #[test]
    fn msb_bits_fill_words_from_the_high_end() {
        let mut cell_words = [0u32; 2];
        top_bit_set(&mut cell_words, 0);
        top_bit_set(&mut cell_words, 33);
        top_bit_set(&mut cell_words, 64);
        assert_eq!(cell_words, [0x8000_0000, 0x4000_0000]);
        assert!(top_bit_get(&cell_words, 33) && !top_bit_get(&cell_words, 1) && !top_bit_get(&cell_words, 64));
    }

    #[test]
    fn msb_iter_walks_set_bits_below_the_count() {
        let mut lit_words = [0u32; 3];
        for i in [0, 5, 31, 32, 63, 64, 70] {
            top_bit_set(&mut lit_words, i);
        }
        let got: alloc::vec::Vec<usize> = msb_iter(&lit_words, 66).collect();
        assert_eq!(got, [0, 5, 31, 32, 63, 64]);
        assert_eq!(msb_iter(&[], 10).count(), 0);
    }
}