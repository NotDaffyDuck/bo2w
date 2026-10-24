//! What a recording says about the match it came from.
//!
//! Playing a recording back is only meaningful against the same content and
//! the same random-number scheme, so the header carries both, plus the seed
//! the match started from and the name of the zone (map) it was played on.

use sim::{RNG_DOMAIN_SCHEME, SimWorld};

/// Bytes the zone name takes in a header, including its zero padding.
pub const ZONE_FIELD_LEN: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MatchRecordIdentity {
    /// Which random-number scheme the match drew with (see
    /// [`RNG_DOMAIN_SCHEME`]); 0 means the current one.
    pub rng_scheme: u32,
    pub root_seed: u64,
    /// A digest of the loaded content, so a recording is not replayed against
    /// different maps or weapons.
    pub content_digest: u64,
    /// Zone name, zero padded; see [`MatchRecordIdentity::zone_name`].
    pub zone: [u8; ZONE_FIELD_LEN],
}

impl MatchRecordIdentity {
    pub fn from_world(world: &SimWorld) -> Self {
        Self {
            rng_scheme: RNG_DOMAIN_SCHEME,
            root_seed: world.root_seed(),
            content_digest: world.content_digest(),
            zone: [0; ZONE_FIELD_LEN],
        }
    }

    pub fn from_world_on_zone(world: &SimWorld, zone: &str) -> Self {
        let mut identity = Self::from_world(world);
        identity.set_zone(zone);
        identity
    }

    /// Stores `zone` with everything but letters, digits and `_` removed,
    /// cut to leave room for the zero that ends it.
    pub fn set_zone(&mut self, zone: &str) {
        let mut field = [0u8; ZONE_FIELD_LEN];
        let kept = zone
            .bytes()
            .filter(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .take(ZONE_FIELD_LEN - 1);
        for (slot, byte) in field.iter_mut().zip(kept) {
            *slot = byte;
        }
        self.zone = field;
    }

    /// The zone the match was played on, if the header names one.
    pub fn zone_name(&self) -> Option<&str> {
        let end = self
            .zone
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(ZONE_FIELD_LEN);
        core::str::from_utf8(&self.zone[..end])
            .ok()
            .filter(|name| !name.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_identity_names_no_zone() {
        let identity = MatchRecordIdentity::default();
        assert_eq!(identity.zone_name(), None);
        assert_eq!(identity.zone, [0; ZONE_FIELD_LEN]);
    }

    #[test]
    fn a_zone_is_stored_and_read_back() {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone("mp_nuketown_2020");
        assert_eq!(identity.zone_name(), Some("mp_nuketown_2020"));
    }

    #[test]
    fn a_zone_loses_characters_a_file_name_would_not_keep() {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone("../mp nuke-town!");
        assert_eq!(identity.zone_name(), Some("mpnuketown"));
    }

    #[test]
    fn a_zone_made_of_nothing_usable_names_no_zone() {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone("mp_x");
        identity.set_zone("-- --");
        assert_eq!(identity.zone_name(), None);
        assert_eq!(identity.zone, [0; ZONE_FIELD_LEN]);
    }

    #[test]
    fn a_long_zone_is_cut_to_31_so_a_zero_ends_it() {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone(&"z".repeat(50));
        assert_eq!(identity.zone_name(), Some("z".repeat(31).as_str()));
        assert_eq!(identity.zone[ZONE_FIELD_LEN - 1], 0);
    }

    #[test]
    fn setting_a_shorter_zone_clears_what_the_longer_one_left() {
        let mut identity = MatchRecordIdentity::default();
        identity.set_zone("a_rather_long_zone_name");
        identity.set_zone("ab");
        assert_eq!(identity.zone_name(), Some("ab"));
        assert!(identity.zone[2..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn a_header_field_with_no_zero_still_reads_in_full() {
        let identity = MatchRecordIdentity {
            zone: [b'q'; ZONE_FIELD_LEN],
            ..MatchRecordIdentity::default()
        };
        assert_eq!(
            identity.zone_name(),
            Some("q".repeat(ZONE_FIELD_LEN).as_str())
        );
    }

    #[test]
    fn a_zone_that_is_not_text_names_no_zone() {
        let mut zone = [0u8; ZONE_FIELD_LEN];
        zone[0] = 0xFF;
        zone[1] = 0xFE;
        let identity = MatchRecordIdentity {
            zone,
            ..MatchRecordIdentity::default()
        };
        assert_eq!(identity.zone_name(), None);
    }

    #[test]
    fn a_world_gives_its_seed_digest_and_the_current_scheme() {
        let world = SimWorld::new();
        let identity = MatchRecordIdentity::from_world(&world);
        assert_eq!(identity.rng_scheme, RNG_DOMAIN_SCHEME);
        assert_eq!(identity.root_seed, world.root_seed());
        assert_eq!(identity.content_digest, world.content_digest());
        assert_eq!(identity.zone_name(), None);
    }

    #[test]
    fn a_world_on_a_zone_carries_the_zone_too() {
        let world = SimWorld::new();
        let identity = MatchRecordIdentity::from_world_on_zone(&world, "zm_transit");
        assert_eq!(identity.zone_name(), Some("zm_transit"));
        assert_eq!(identity.root_seed, world.root_seed());
    }
}
