//! The match's seeded random streams: one root seed, a separate stream per domain.

/// The version of the way random streams are seeded. Replays store it so a changed scheme is noticed.
pub const RANDOM_STREAM_LAYOUT_VERSION: u32 = 1;

/// The separate random streams a match keeps, so that drawing from one never moves another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum RandomStreamKind {
    Spawn = 1,
    Combat = 2,
    Bot = 3,
}

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const DOMAIN_OFFSET: u64 = 0xA5A5_A5A5_5A5A_5A5A;
const OUTPUT_SCRAMBLE: u64 = 0x2545_F491_4F6C_DD1D;

/// The seed of one domain's stream: a splitmix64 finish over the root seed and the domain's number.
fn seed_of_domain(root: u64, domain: RandomStreamKind) -> u64 {
    let z = root
        .wrapping_add((domain as u64).wrapping_mul(GOLDEN_GAMMA))
        .wrapping_add(DOMAIN_OFFSET);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z >> 31) ^ z
}

/// A seeded xorshift64* stream that also counts its draws, so a saved match can be put back at the same point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchRandom {
    seed: u64,
    drawn: u64,
    state: u64,
}

impl MatchRandom {
    /// A stream that starts from `seed`.
    pub fn new(initial_seed: u64) -> Self {
        Self { seed: initial_seed, drawn: 0, state: initial_seed ^ GOLDEN_GAMMA }
    }

    /// The stream of `domain` for a match whose root seed is `root`.
    pub fn from_root(root_seed: u64, domain: RandomStreamKind) -> Self {
        Self::new(seed_of_domain(root_seed, domain))
    }

    /// The seed the stream was made with.
    pub fn sown_with(&self) -> u64 {
        self.seed
    }

    /// How many numbers have been drawn since the stream was created.
    pub fn draws(&self) -> u64 {
        self.drawn
    }

    /// Sets the draw count back to a saved value. The stream's position does not move with it: restoring a match
    /// replays the draws itself.
    pub(crate) fn set_drawn(&mut self, saved: u64) {
        self.drawn = saved;
    }

    /// The next 32 random bits.
    pub fn draw_word(&mut self) -> u32 {
        let mut s = self.state;
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        (self.state, self.drawn) = (s, self.drawn.wrapping_add(1));
        (s.wrapping_mul(OUTPUT_SCRAMBLE) >> 32) as u32
    }

    /// A number in `0..len`; `len` must not be zero.
    pub fn below(&mut self, bound: usize) -> usize {
        debug_assert!(bound != 0);
        self.draw_word() as usize % bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_four(seed: u64) -> [u32; 4] {
        let mut rng = MatchRandom::new(seed);
        std::array::from_fn(|_| rng.draw_word())
    }

    #[test]
    fn the_stream_gives_the_same_numbers_it_always_has() {
        assert_eq!(first_four(0), [0x0d83_b3e2, 0x54c4_4c79, 0xa845_f342, 0x7d6e_0b87]);
        assert_eq!(first_four(1), [0x102a_ceb9, 0x24b8_9d23, 0xb584_971a, 0x1df7_614b]);
        assert_eq!(first_four(0xDEAD_BEEF), [0x5169_0e95, 0x7a74_bc48, 0x8fe8_fd5b, 0xf7c0_015f]);
    }

    #[test]
    fn each_domain_of_a_root_seed_gets_its_own_fixed_seed() {
        assert_eq!(MatchRandom::from_root(12345, RandomStreamKind::Spawn).sown_with(), 0xb03c_483f_268f_4850);
        assert_eq!(MatchRandom::from_root(12345, RandomStreamKind::Combat).sown_with(), 0x3062_ddac_4f5e_19a8);
        assert_eq!(MatchRandom::from_root(12345, RandomStreamKind::Bot).sown_with(), 0xd2be_2f9a_a241_caa2);
        assert_eq!(MatchRandom::from_root(0, RandomStreamKind::Spawn).sown_with(), 0xcf4e_1e4f_fd3b_ac82);
    }

    #[test]
    fn a_stream_from_a_root_is_the_stream_of_its_derived_seed() {
        let mut from_root = MatchRandom::from_root(7, RandomStreamKind::Combat);
        let mut from_seed = MatchRandom::new(seed_of_domain(7, RandomStreamKind::Combat));
        for _ in 0..8 {
            assert_eq!(from_root.draw_word(), from_seed.draw_word());
        }
    }

    #[test]
    fn every_draw_is_counted_and_restoring_only_resets_the_count() {
        let mut rng = MatchRandom::new(5);
        assert_eq!(rng.draws(), 0);
        rng.draw_word();
        rng.below(10);
        assert_eq!(rng.draws(), 2);
        let mut twin = rng.clone();
        rng.set_drawn(40);
        assert_eq!(rng.draws(), 40);
        assert_eq!(rng.draw_word(), twin.draw_word());
    }

    #[test]
    fn an_index_is_the_next_number_reduced_into_range() {
        let mut by_index = MatchRandom::new(9);
        let mut by_number = MatchRandom::new(9);
        for len in [1usize, 2, 7, 100] {
            let index = by_index.below(len);
            assert!(index < len);
            assert_eq!(index, by_number.draw_word() as usize % len);
        }
    }

    #[test]
    fn the_domain_numbers_and_scheme_are_fixed() {
        assert_eq!((RandomStreamKind::Spawn as u64, RandomStreamKind::Combat as u64, RandomStreamKind::Bot as u64), (1, 2, 3));
        assert_eq!(RANDOM_STREAM_LAYOUT_VERSION, 1);
    }
}
