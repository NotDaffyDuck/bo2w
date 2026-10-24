/// Words for `count` cell bits, 32 to a word.
#[must_use]
pub fn words_for_bits(count: usize) -> usize {
    count.div_ceil(32)
}

/// Whether bit `index` is set: cell `n` is bit `n % 32` of word `n / 32`, the layout of
/// the walk's visible-cell bits. These are this port's own per-frame set, not a BO2 table;
/// every reader decodes them LSB first. BO2's stored cell tables (cellCasterBits,
/// sceneEntCellBits, dynEntCellBits) are MSB first: read those with `msb_get`.
#[inline]
#[must_use]
pub fn bit_get(bits: &[u32], index: usize) -> bool {
    bits.get(index / 32)
        .is_some_and(|w| w & (1 << (index % 32)) != 0)
}

#[inline]
pub fn bit_set(bits: &mut [u32], index: usize) {
    if let Some(w) = bits.get_mut(index / 32) {
        *w |= 1 << (index % 32);
    }
}

/// BO2's cell-table order: bit `index` is mask `0x8000_0000 >> (index & 31)` of word
/// `index >> 5`.
#[inline]
#[must_use]
pub fn msb_get(bits: &[u32], index: usize) -> bool {
    bits.get(index >> 5)
        .is_some_and(|w| w & (0x8000_0000 >> (index & 31)) != 0)
}

#[inline]
pub fn msb_set(bits: &mut [u32], index: usize) {
    if let Some(w) = bits.get_mut(index >> 5) {
        *w |= 0x8000_0000 >> (index & 31);
    }
}

/// The set bits below `bit_count` of an MSB-first cell table, lowest index first.
#[must_use]
pub fn msb_iter(bits: &[u32], bit_count: usize) -> MsbBitIter<'_> {
    MsbBitIter {
        words: bits,
        bit_count,
        word: 0,
        remaining: bits.first().copied().unwrap_or(0),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MsbBitIter<'a> {
    words: &'a [u32],
    bit_count: usize,
    word: usize,
    remaining: u32,
}

impl Iterator for MsbBitIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.remaining != 0 {
                let bit = self.remaining.leading_zeros() as usize;
                self.remaining &= !(0x8000_0000 >> bit);
                let index = self.word * 32 + bit;
                if index >= self.bit_count {
                    self.remaining = 0;
                    self.word = self.words.len();
                    return None;
                }
                return Some(index);
            }
            self.word = self.word.saturating_add(1);
            if self.word >= self.words.len() || self.word * 32 >= self.bit_count {
                return None;
            }
            self.remaining = self.words[self.word];
        }
    }
}

/// A cell bit set over caller-owned words, in the walk's layout.
#[derive(Debug)]
pub struct VisBits<'a> {
    words: &'a mut [u32],
}

impl<'a> VisBits<'a> {
    pub fn new(words: &'a mut [u32]) -> Self {
        Self { words }
    }

    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    pub fn set(&mut self, index: usize) {
        bit_set(self.words, index);
    }

    #[must_use]
    pub fn get(&self, index: usize) -> bool {
        bit_get(self.words, index)
    }

    #[must_use]
    pub fn count_ones(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_fill_words_from_the_low_end() {
        let mut words = [0u32; 2];
        let mut vis = VisBits::new(&mut words);
        vis.set(0);
        vis.set(33);
        vis.set(64);
        assert!(vis.get(33) && !vis.get(32));
        assert_eq!(vis.count_ones(), 2);
        assert_eq!(words, [1, 2]);
        assert_eq!(words_for_bits(33), 2);
    }

    #[test]
    fn msb_bits_fill_words_from_the_high_end() {
        let mut words = [0u32; 2];
        msb_set(&mut words, 0);
        msb_set(&mut words, 33);
        msb_set(&mut words, 64);
        assert_eq!(words, [0x8000_0000, 0x4000_0000]);
        assert!(msb_get(&words, 33) && !msb_get(&words, 1) && !msb_get(&words, 64));
    }

    #[test]
    fn msb_iter_walks_set_bits_below_the_count() {
        let mut words = [0u32; 3];
        for i in [0, 5, 31, 32, 63, 64, 70] {
            msb_set(&mut words, i);
        }
        let got: alloc::vec::Vec<usize> = msb_iter(&words, 66).collect();
        assert_eq!(got, [0, 5, 31, 32, 63, 64]);
        assert_eq!(msb_iter(&[], 10).count(), 0);
    }
}
