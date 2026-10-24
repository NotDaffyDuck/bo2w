//! The small identifiers the simulation passes around (who died, which shot, which life) and the match's own seeded
//! random number source.

/// Declares an identifier made of one number that can be stepped on and wraps round instead of overflowing.
macro_rules! stepping_id {
    ($(#[$meta:meta])* $name:ident($number:ty)) => {
        $(#[$meta])*
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub $number);

        impl $name {
            /// The identifier after this one; the largest number is followed by zero.
            pub fn next(self) -> Self {
                let Self(number) = self;
                Self(number.wrapping_add(1))
            }
        }
    };
}

stepping_id!(
    /// Counts a client's lives; each spawn gets the next one.
    LifeSequence(u32)
);
stepping_id!(
    /// Numbers the events the match publishes, in the order they happened.
    EventSequence(u32)
);
stepping_id!(
    /// Numbers the actions a client has sent.
    ActionSequence(u32)
);
stepping_id!(
    /// Names one trigger pull.
    ShotId(u32)
);
stepping_id!(
    /// Names one thrown or launched projectile.
    ProjectileId(u32)
);

impl EventSequence {
    /// Whether this event came after `other`. The numbers wrap, so "after" means at most half the number range ahead.
    pub fn is_newer_than(self, other: Self) -> bool {
        let ahead = self.0.wrapping_sub(other.0);
        (1..=i32::MAX as u32).contains(&ahead)
    }
}

/// Names one pellet of a shot (a shotgun fires several).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PelletId(pub u16);

/// Names a script-spawned model by its place in the order the map's scripts created them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScriptModelId(pub(crate) u32);

impl ScriptModelId {
    /// The id for a number read off the wire.
    pub const fn from_wire(ordinal: u32) -> Self {
        Self(ordinal)
    }

    /// The number to put on the wire.
    pub const fn to_wire(self) -> u32 {
        let Self(ordinal) = self;
        ordinal
    }

    /// The id for the n-th model the map's scripts create; the same number as on the wire.
    pub const fn from_authored_source_ordinal(ordinal: u32) -> Self {
        Self::from_wire(ordinal)
    }
}

/// What dealt a damage: a shot, a projectile, an explosion (named by the model that went off) or a melee blow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DamageSource {
    Shot(ShotId),
    Projectile(ProjectileId),
    Radius(ScriptModelId),
    Melee,
}

/// Which part of its life the match is in.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    #[default]
    Warmup,
    Playing,
    Intermission,
    PostGame,
}

/// The version of the way random streams are seeded. Replays store it so a changed scheme is noticed.
pub const RNG_DOMAIN_SCHEME: u32 = 1;

/// The separate random streams a match keeps, so that drawing from one never moves another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum RngDomain {
    Spawn = 1,
    Combat = 2,
    Bot = 3,
}

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const DOMAIN_OFFSET: u64 = 0xA5A5_A5A5_5A5A_5A5A;
const XORSHIFT_OUT_MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// The seed of one domain's stream, derived from the match's root seed (a splitmix64 finish over root and domain).
fn domain_seed(root: u64, domain: RngDomain) -> u64 {
    let mixed_in = (domain as u64).wrapping_mul(GOLDEN_GAMMA);
    let z = root.wrapping_add(mixed_in).wrapping_add(DOMAIN_OFFSET);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// One step of the xorshift64 generator state.
fn xorshift_step(mut x: u64) -> u64 {
    x ^= x >> 12;
    x ^= x << 25;
    x ^ (x >> 27)
}

/// A seeded xorshift64* stream that also counts its draws, so a saved match can be put back at the same point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchRng {
    seed: u64,
    draws: u64,
    state: u64,
}

impl MatchRng {
    /// A stream that starts from `seed`.
    pub fn new(seed: u64) -> Self {
        Self { seed, draws: 0, state: seed ^ GOLDEN_GAMMA }
    }

    /// The stream of `domain` for a match whose root seed is `root`.
    pub fn from_root(root: u64, domain: RngDomain) -> Self {
        Self::new(domain_seed(root, domain))
    }

    /// The seed the stream was made with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// How many numbers have been drawn since the stream was created.
    pub fn draws(&self) -> u64 {
        self.draws
    }

    /// Sets the draw count back to a saved value. The stream's position is not moved with it; restoring a match
    /// replays the draws itself.
    pub(crate) fn restore_draws(&mut self, draws: u64) {
        self.draws = draws;
    }

    /// The next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        self.draws = self.draws.wrapping_add(1);
        self.state = xorshift_step(self.state);
        (self.state.wrapping_mul(XORSHIFT_OUT_MULTIPLIER) >> 32) as u32
    }

    /// A number in `0..len`; `len` must not be zero.
    pub fn next_index(&mut self, len: usize) -> usize {
        debug_assert!(len > 0);
        self.next_u32() as usize % len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_four(seed: u64) -> [u32; 4] {
        let mut rng = MatchRng::new(seed);
        [rng.next_u32(), rng.next_u32(), rng.next_u32(), rng.next_u32()]
    }

    #[test]
    fn the_stream_gives_the_same_numbers_it_always_has() {
        assert_eq!(first_four(0), [0x0d83_b3e2, 0x54c4_4c79, 0xa845_f342, 0x7d6e_0b87]);
        assert_eq!(first_four(1), [0x102a_ceb9, 0x24b8_9d23, 0xb584_971a, 0x1df7_614b]);
        assert_eq!(first_four(0xDEAD_BEEF), [0x5169_0e95, 0x7a74_bc48, 0x8fe8_fd5b, 0xf7c0_015f]);
    }

    #[test]
    fn each_domain_of_a_root_seed_gets_its_own_fixed_seed() {
        assert_eq!(MatchRng::from_root(12345, RngDomain::Spawn).seed(), 0xb03c_483f_268f_4850);
        assert_eq!(MatchRng::from_root(12345, RngDomain::Combat).seed(), 0x3062_ddac_4f5e_19a8);
        assert_eq!(MatchRng::from_root(12345, RngDomain::Bot).seed(), 0xd2be_2f9a_a241_caa2);
        assert_eq!(MatchRng::from_root(0, RngDomain::Spawn).seed(), 0xcf4e_1e4f_fd3b_ac82);
    }

    #[test]
    fn a_stream_from_a_root_is_the_stream_of_its_derived_seed() {
        let mut from_root = MatchRng::from_root(7, RngDomain::Combat);
        let mut from_seed = MatchRng::new(domain_seed(7, RngDomain::Combat));
        for _ in 0..8 {
            assert_eq!(from_root.next_u32(), from_seed.next_u32());
        }
    }

    #[test]
    fn every_draw_is_counted_and_restoring_only_resets_the_count() {
        let mut rng = MatchRng::new(5);
        assert_eq!(rng.draws(), 0);
        rng.next_u32();
        rng.next_index(10);
        assert_eq!(rng.draws(), 2);
        let mut twin = rng.clone();
        rng.restore_draws(40);
        assert_eq!(rng.draws(), 40);
        assert_eq!(rng.next_u32(), twin.next_u32());
    }

    #[test]
    fn an_index_is_the_next_number_reduced_into_range() {
        let mut by_index = MatchRng::new(9);
        let mut by_number = MatchRng::new(9);
        for len in [1usize, 2, 7, 100] {
            let index = by_index.next_index(len);
            assert!(index < len);
            assert_eq!(index, by_number.next_u32() as usize % len);
        }
    }

    #[test]
    fn identifiers_step_on_and_wrap_to_zero() {
        assert_eq!(LifeSequence(4).next(), LifeSequence(5));
        assert_eq!(ShotId(u32::MAX).next(), ShotId(0));
        assert_eq!(ProjectileId(u32::MAX).next(), ProjectileId(0));
        assert_eq!(ActionSequence(u32::MAX).next(), ActionSequence(0));
        assert_eq!(EventSequence(u32::MAX).next(), EventSequence(0));
    }

    #[test]
    fn a_later_event_is_newer_even_across_the_wrap() {
        assert!(EventSequence(6).is_newer_than(EventSequence(5)));
        assert!(!EventSequence(5).is_newer_than(EventSequence(6)));
        assert!(!EventSequence(5).is_newer_than(EventSequence(5)));
        assert!(EventSequence(2).is_newer_than(EventSequence(u32::MAX - 1)));
        assert!(!EventSequence(u32::MAX - 1).is_newer_than(EventSequence(2)));
    }

    #[test]
    fn newer_stops_at_half_the_range() {
        assert!(EventSequence(0x7fff_ffff).is_newer_than(EventSequence(0)));
        assert!(!EventSequence(0x8000_0000).is_newer_than(EventSequence(0)));
    }

    #[test]
    fn a_script_model_id_keeps_its_ordinal_either_way_it_is_made() {
        assert_eq!(ScriptModelId::from_wire(9).to_wire(), 9);
        assert_eq!(ScriptModelId::from_authored_source_ordinal(9), ScriptModelId::from_wire(9));
    }

    #[test]
    fn damage_sources_sort_by_kind_then_by_id() {
        let shot = DamageSource::Shot(ShotId(9));
        let projectile = DamageSource::Projectile(ProjectileId(0));
        let radius = DamageSource::Radius(ScriptModelId::from_wire(0));
        assert!(shot < projectile && projectile < radius && radius < DamageSource::Melee);
        assert!(DamageSource::Shot(ShotId(1)) < DamageSource::Shot(ShotId(2)));
    }

    #[test]
    fn a_match_starts_in_warmup_and_the_domain_numbers_are_fixed() {
        assert_eq!(MatchPhase::default(), MatchPhase::Warmup);
        assert_eq!((RngDomain::Spawn as u64, RngDomain::Combat as u64, RngDomain::Bot as u64), (1, 2, 3));
        assert_eq!(RNG_DOMAIN_SCHEME, 1);
    }
}
