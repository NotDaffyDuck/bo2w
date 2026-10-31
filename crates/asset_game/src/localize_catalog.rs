//! Localized text, looked up by key.

use std::collections::HashMap;
use std::path::Path;

use bevy::prelude::Resource;

use crate::ZoneTitleKind;

/// The localized zones a multiplayer match reads strings from.
pub const VERSUS_LANGUAGE_ZONES: &[&str] = &[
    "localized_code_post_gfx_mp",
    "localized_common_mp",
    "localized_ui_mp",
];

/// Localized text by key. Keys are kept upper-case and so match whatever
/// their case; when two sources give a key, the one added first stays.
#[derive(Clone, Debug, Default, Resource)]
pub struct TranslationStringIndex {
    by_key: HashMap<String, String>,
}

impl TranslationStringIndex {
    /// A catalog of `(key, text)` rows. Rows with an empty key are dropped.
    pub fn from_rows<'a>(rows: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let by_key = rows
            .into_iter()
            .filter(|(key, _)| !key.is_empty())
            .fold(HashMap::new(), |mut taken, (key, text)| {
                taken.entry(key.to_uppercase()).or_insert_with(|| text.to_owned());
                taken
            });
        Self { by_key }
    }

    /// Adds a row under an already upper-cased key unless the key is taken.
    fn keep_first(&mut self, upper_key: String, words: String) {
        self.by_key.entry(upper_key).or_insert(words);
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
    pub fn merge_in(&mut self, other: TranslationStringIndex) {
        other.by_key.into_iter().for_each(|(key, text)| self.keep_first(key, text));
    }
}

/// Reads the localized strings of one zone file. Black Ops II zones do not
/// give their strings up yet, so every known zone is an error saying so.
pub fn load_translation_lane(path: &Path) -> Result<TranslationStringIndex, String> {
    let shown = path.display();
    let game = asset_transport::title_of_zone_path(path).ok_or_else(|| format!("{shown}: not a known zone"))?;
    Err(match game {
        ZoneTitleKind::T6 => format!("{shown}: Black Ops II localized strings are not read from zones yet"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_whatever_their_case() {
        let catalog = TranslationStringIndex::from_rows([("Menu_Play", "Play")]);
        assert_eq!(catalog.text("MENU_PLAY"), Some("Play"));
        assert_eq!(catalog.text("menu_play"), Some("Play"));
        assert_eq!(catalog.text("menu_quit"), None);
    }

    #[test]
    fn rows_with_no_key_are_dropped_and_the_first_text_wins() {
        let catalog = TranslationStringIndex::from_rows([("", "orphan"), ("a", "first"), ("A", "second")]);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.text("a"), Some("first"));
    }

    #[test]
    fn a_path_that_is_no_zone_is_named_in_the_error() {
        let path = Path::new("no_such_dir/no_such.zone");
        let err = load_translation_lane(path).unwrap_err();
        assert!(err.ends_with(": not a known zone"), "{err}");
        assert!(err.contains("no_such.zone"));
    }

    #[test]
    fn absorbing_never_replaces_what_is_there() {
        let mut ours = TranslationStringIndex::from_rows([("a", "ours")]);
        ours.merge_in(TranslationStringIndex::from_rows([("a", "theirs"), ("b", "new")]));
        assert_eq!(ours.text("a"), Some("ours"));
        assert_eq!(ours.text("b"), Some("new"));
        assert!(!ours.is_empty());
    }
}
