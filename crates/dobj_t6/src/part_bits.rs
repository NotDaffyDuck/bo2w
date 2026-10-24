/// The most parts (bones) one DObj holds over all its models; part
/// indices run 0..=159 and `numBones` is a byte.
pub const DOBJ_MAX_PARTS: usize = 160;

/// A DObj part set: 5 words, part `k` in word `k >> 5` under the mask
/// `0x8000_0000 >> (k & 31)` (part 0 is the top bit of word 0). The anim
/// masks, the skeleton masks and the hide bits all use this layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartBits([u32; PartBits::WORDS]);

impl PartBits {
    pub const WORDS: usize = 5;
    pub const CAPACITY: usize = DOBJ_MAX_PARTS;

    pub const NONE: Self = Self([0; Self::WORDS]);
    pub const ALL: Self = Self([u32::MAX; Self::WORDS]);

    pub const fn from_words(words: [u32; Self::WORDS]) -> Self {
        Self(words)
    }

    pub const fn words(&self) -> &[u32; Self::WORDS] {
        &self.0
    }

    /// Parts `0..count` (capped at 160).
    pub fn first(count: usize) -> Self {
        let mut bits = Self::NONE;
        for part in 0..count.min(Self::CAPACITY) {
            bits.set(part);
        }
        bits
    }

    /// Sets part `part`; a part past 159 is out of range and ignored.
    pub fn set(&mut self, part: usize) {
        if let Some((word, mask)) = slot(part) {
            self.0[word] |= mask;
        }
    }

    pub fn clear(&mut self, part: usize) {
        if let Some((word, mask)) = slot(part) {
            self.0[word] &= !mask;
        }
    }

    /// Whether part `part` is in the set; a part past 159 never is.
    pub fn get(&self, part: usize) -> bool {
        slot(part).is_some_and(|(word, mask)| self.0[word] & mask != 0)
    }

    #[must_use]
    pub fn or(&self, other: &Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] | other.0[i]))
    }

    #[must_use]
    pub fn and(&self, other: &Self) -> Self {
        Self(core::array::from_fn(|i| self.0[i] & other.0[i]))
    }

    #[must_use]
    pub fn not(&self) -> Self {
        Self(core::array::from_fn(|i| !self.0[i]))
    }

    pub fn is_all(&self) -> bool {
        self.0.iter().all(|w| *w == u32::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|w| *w == 0)
    }

    /// The parts in the set, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..Self::CAPACITY).filter(|part| self.get(*part))
    }
}

fn slot(part: usize) -> Option<(usize, u32)> {
    (part < PartBits::CAPACITY).then(|| (part >> 5, 0x8000_0000u32 >> (part & 31)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_zero_is_the_top_bit_of_word_zero() {
        let mut bits = PartBits::NONE;
        bits.set(0);
        bits.set(33);
        bits.set(159);
        assert_eq!(bits.words(), &[0x8000_0000, 0x4000_0000, 0, 0, 1]);
        assert!(bits.get(33) && !bits.get(32));
        assert_eq!(bits.iter().collect::<Vec<_>>(), [0, 33, 159]);
    }

    #[test]
    fn parts_past_159_are_out_of_range() {
        let mut bits = PartBits::NONE;
        bits.set(160);
        assert!(bits.is_empty());
        assert!(!PartBits::ALL.get(160));
        assert_eq!(PartBits::first(500), PartBits::ALL);
        assert_eq!(PartBits::first(2).words(), &[0xc000_0000, 0, 0, 0, 0]);
    }

    #[test]
    fn set_operations() {
        let a = PartBits::first(40);
        let b = PartBits::first(8).not();
        assert_eq!(a.and(&b).iter().count(), 32);
        assert!(a.or(&a.not()).is_all());
        let mut c = a;
        c.clear(0);
        assert!(!c.get(0) && c.get(1));
    }
}
