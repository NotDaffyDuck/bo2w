//! Localized text, looked up by key.

use std::collections::HashMap;
use std::path::Path;

use bevy::prelude::Resource;

use crate::ZoneGame;

/// The localized zones a multiplayer match reads strings from.
pub const MP_LOCALIZED_ZONES: &[&str] = &[
    "localized_code_post_gfx_mp",
    "localized_common_mp",
    "localized_ui_mp",
];

/// Localized text by key. Keys are kept upper-case and so match whatever
/// their case; when two sources give a key, the one added first stays.
#[derive(Clone, Debug, Default, Resource)]
pub struct LocalizeCatalog {
    by_key: HashMap<String, String>,
}

impl LocalizeCatalog {
    /// A catalog of `(key, text)` rows. Rows with an empty key are dropped.
    pub fn from_rows<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut catalog = Self::default();
        for (key, text) in rows.into_iter().filter(|(key, _)| !key.is_empty()) {
            catalog
                .by_key
                .entry(key.to_uppercase())
                .or_insert_with(|| text.to_owned());
        }
        catalog
    }

    /// The text for `key`, if there is any.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.by_key.get(&key.to_uppercase()).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Takes in every row of `other` whose key is not already here.
    pub fn absorb(&mut self, other: LocalizeCatalog) {
        for (key, text) in other.by_key {
            self.by_key.entry(key).or_insert(text);
        }
    }
}

/// Reads the localized strings of one zone file. Black Ops II zones do not
/// give their strings up yet, so every known zone is an error saying so.
pub fn load_localize_catalog_in_lane(path: &Path) -> Result<LocalizeCatalog, String> {
    let Some(game) = asset_transport::zone_game_for_path(path) else {
        return Err(format!("{}: not a known zone", path.display()));
    };
    match game {
        ZoneGame::T6 => Err(format!(
            "{}: Black Ops II localized strings are not read from zones yet",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_whatever_their_case() {
        let catalog = LocalizeCatalog::from_rows([("Menu_Play", "Play")]);
        assert_eq!(catalog.text("MENU_PLAY"), Some("Play"));
        assert_eq!(catalog.text("menu_play"), Some("Play"));
        assert_eq!(catalog.text("menu_quit"), None);
    }

    #[test]
    fn rows_with_no_key_are_dropped_and_the_first_text_wins() {
        let catalog = LocalizeCatalog::from_rows([("", "orphan"), ("a", "first"), ("A", "second")]);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.text("a"), Some("first"));
    }

    #[test]
    fn absorbing_never_replaces_what_is_there() {
        let mut ours = LocalizeCatalog::from_rows([("a", "ours")]);
        ours.absorb(LocalizeCatalog::from_rows([("a", "theirs"), ("b", "new")]));
        assert_eq!(ours.text("a"), Some("ours"));
        assert_eq!(ours.text("b"), Some("new"));
        assert!(!ours.is_empty());
    }
}
