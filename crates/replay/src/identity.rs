//! What a recording says about the match it came from.
//!
//! Playing a recording back is only meaningful against the same content and
//! the same random-number scheme, so the header carries both, plus the seed
//! the match started from and the name of the zone (map) it was played on.

use sim::RANDOM_STREAM_LAYOUT_VERSION;

/// Bytes the zone name takes in a header, including its zero padding.
pub const REGION_LABEL_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordedGameLabel {
    /// Which random-number scheme the match drew with (see
    /// [`RANDOM_STREAM_LAYOUT_VERSION`]); 0 means the current one.
    pub random_stream_layout: u32,
    /// The seed the world grew from.
    pub world_seed: u64,
    /// A digest of the loaded content, so a recording is not replayed against
    /// different maps or weapons.
    pub content_fingerprint: u64,
    /// Map name, zero padded; see [`RecordedGameLabel::map_name`].
    pub map_bytes: [u8; REGION_LABEL_CAPACITY],
}

impl RecordedGameLabel {
    pub fn from_world(source: &sim::SimWorld) -> Self {
        Self {
            random_stream_layout: RANDOM_STREAM_LAYOUT_VERSION,
            world_seed: source.root_seed(),
            content_fingerprint: source.content_fingerprint(),
            map_bytes: [0; REGION_LABEL_CAPACITY],
        }
    }

    pub fn from_world_on_zone(source: &sim::SimWorld, map: &str) -> Self {
        let mut label = Self::from_world(source);
        label.assign_region_label(map);
        label
    }

    /// Stores `map` with everything but letters, digits and `_` removed,
    /// cut to leave room for the zero that ends it.
    pub fn assign_region_label(&mut self, map: &str) {
        let mut field = [0u8; REGION_LABEL_CAPACITY];
        let kept = map
            .bytes()
            .filter(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .take(REGION_LABEL_CAPACITY - 1);
        for (slot, kept_byte) in field.iter_mut().zip(kept) {
            *slot = kept_byte;
        }
        self.map_bytes = field;
    }

    /// The map the match was played on, if the header names one.
    pub fn map_name(&self) -> Option<&str> {
        let used = self.map_bytes.iter().take_while(|&&b| b != 0).count();
        core::str::from_utf8(&self.map_bytes[..used]).ok().filter(|text| !text.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_identity_names_no_zone() {
        let identity = RecordedGameLabel::default();
        assert_eq!(identity.map_name(), None);
        assert_eq!(identity.map_bytes, [0; REGION_LABEL_CAPACITY]);
    }

    #[test]
    fn a_zone_is_stored_and_read_back() {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label("mp_nuketown_2020");
        assert_eq!(identity.map_name(), Some("mp_nuketown_2020"));
    }

    #[test]
    fn a_zone_loses_characters_a_file_name_would_not_keep() {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label("../mp nuke-town!");
        assert_eq!(identity.map_name(), Some("mpnuketown"));
    }

    #[test]
    fn a_zone_made_of_nothing_usable_names_no_zone() {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label("mp_x");
        identity.assign_region_label("-- --");
        assert_eq!(identity.map_name(), None);
        assert_eq!(identity.map_bytes, [0; REGION_LABEL_CAPACITY]);
    }

    #[test]
    fn a_long_zone_is_cut_to_31_so_a_zero_ends_it() {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label(&"z".repeat(50));
        assert_eq!(identity.map_name(), Some("z".repeat(31).as_str()));
        assert_eq!(identity.map_bytes[REGION_LABEL_CAPACITY - 1], 0);
    }

    #[test]
    fn setting_a_shorter_zone_clears_what_the_longer_one_left() {
        let mut identity = RecordedGameLabel::default();
        identity.assign_region_label("a_rather_long_zone_name");
        identity.assign_region_label("ab");
        assert_eq!(identity.map_name(), Some("ab"));
        assert!(identity.map_bytes[2..].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn a_header_field_with_no_zero_still_reads_in_full() {
        let identity = RecordedGameLabel {
            map_bytes: [b'q'; REGION_LABEL_CAPACITY],
            ..RecordedGameLabel::default()
        };
        assert_eq!(
            identity.map_name(),
            Some("q".repeat(REGION_LABEL_CAPACITY).as_str())
        );
    }

    #[test]
    fn a_zone_that_is_not_text_names_no_zone() {
        let mut raw = [0u8; REGION_LABEL_CAPACITY];
        raw[0] = 0xFF;
        raw[1] = 0xFE;
        let identity = RecordedGameLabel {
            map_bytes: raw,
            ..RecordedGameLabel::default()
        };
        assert_eq!(identity.map_name(), None);
    }

    #[test]
    fn a_world_gives_its_seed_digest_and_the_current_scheme() {
        let world = sim::SimWorld::new();
        let identity = RecordedGameLabel::from_world(&world);
        assert_eq!(identity.random_stream_layout, RANDOM_STREAM_LAYOUT_VERSION);
        assert_eq!(identity.world_seed, world.root_seed());
        assert_eq!(identity.content_fingerprint, world.content_fingerprint());
        assert_eq!(identity.map_name(), None);
    }

    #[test]
    fn a_world_on_a_zone_carries_the_zone_too() {
        let world = sim::SimWorld::new();
        let identity = RecordedGameLabel::from_world_on_zone(&world, "zm_transit");
        assert_eq!(identity.map_name(), Some("zm_transit"));
        assert_eq!(identity.world_seed, world.root_seed());
    }
}
