//! Content packs a lobby can need, as one bit each in `ContentFlags`.
//!
//! The core only knows the shape: a pack has a bit, the map-name prefix that
//! selects it (`prefix:map`) and a short label for logs. The game crate that
//! ships the packs installs the table as a `ContentCatalog` resource.

use bevy::prelude::Resource;
use master_protocol::ContentFlags;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentPack {
    /// Exactly one bit set.
    pub bit: u8,
    /// The text before `:` in a map name that belongs to this pack.
    pub prefix: &'static str,
    pub label: &'static str,
}

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentCatalog {
    packs: Vec<ContentPack>,
    /// The pack a map name without a prefix belongs to.
    unprefixed: Option<&'static str>,
}

impl ContentCatalog {
    pub fn new(packs: Vec<ContentPack>, unprefixed: Option<&'static str>) -> Self {
        Self { packs, unprefixed }
    }

    pub fn packs(&self) -> &[ContentPack] {
        &self.packs
    }

    fn by_prefix(&self, prefix: &str) -> Option<&ContentPack> {
        self.packs.iter().find(|pack| pack.prefix == prefix)
    }

    /// The bits a host of `map` asks every joiner to hold.
    pub fn mask_for_map(&self, map: &str) -> Result<ContentFlags, String> {
        let prefix = match map.split_once(':') {
            Some((prefix, _)) => Some(prefix),
            None => self.unprefixed,
        };
        prefix
            .and_then(|prefix| self.by_prefix(prefix))
            .map(|pack| ContentFlags(pack.bit))
            .ok_or_else(|| {
                let shown = prefix.unwrap_or("");
                format!("unknown content namespace `{shown}` in map `{map}`")
            })
    }

    /// The bits of every pack `installed` says is on this PC.
    pub fn held(&self, installed: impl Fn(&ContentPack) -> bool) -> ContentFlags {
        ContentFlags(
            self.packs
                .iter()
                .filter(|pack| installed(pack))
                .fold(0, |bits, pack| bits | pack.bit),
        )
    }

    /// Labels of the set bits in table order, then `unknown` for bits no pack owns.
    pub fn describe(&self, flags: ContentFlags) -> String {
        let mut known = 0u8;
        let mut words: Vec<&str> = Vec::new();
        for pack in &self.packs {
            known |= pack.bit;
            if flags.0 & pack.bit != 0 {
                words.push(pack.label);
            }
        }
        if flags.0 & !known != 0 {
            words.push("unknown");
        }
        words.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_packs() -> ContentCatalog {
        ContentCatalog::new(
            vec![
                ContentPack { bit: 0b0100, prefix: "old", label: "old" },
                ContentPack { bit: 0b1000, prefix: "new", label: "new" },
            ],
            Some("new"),
        )
    }

    #[test]
    fn unprefixed_map_uses_the_default_pack() {
        assert_eq!(two_packs().mask_for_map("mp_any").unwrap(), ContentFlags(0b1000));
        assert_eq!(two_packs().mask_for_map("old:zone").unwrap(), ContentFlags(0b0100));
    }

    #[test]
    fn unknown_prefix_is_refused_with_the_map_named() {
        let error = two_packs().mask_for_map("odd:zone").unwrap_err();
        assert_eq!(error, "unknown content namespace `odd` in map `odd:zone`");
        let empty = ContentCatalog::default().mask_for_map("bare").unwrap_err();
        assert_eq!(empty, "unknown content namespace `` in map `bare`");
    }

    #[test]
    fn labels_follow_table_order_and_flag_strays() {
        let catalog = two_packs();
        assert_eq!(catalog.describe(ContentFlags(0b1100)), "old,new");
        assert_eq!(catalog.describe(ContentFlags(0b1000_0000)), "unknown");
        assert_eq!(catalog.describe(ContentFlags(0)), "");
    }

    #[test]
    fn held_collects_installed_packs() {
        let catalog = two_packs();
        assert_eq!(catalog.held(|pack| pack.prefix == "new"), ContentFlags(0b1000));
        assert_eq!(catalog.held(|_| true), ContentFlags(0b1100));
        assert_eq!(catalog.held(|_| false), ContentFlags(0));
    }
}
