//! Config-string tables. The host numbers a name once, and a snapshot then carries the number ("sound 7") rather than
//! the name each time.
//!
//! One table type ([`names`]) serves sound aliases, effect names and HUD materials: numbers 1 to 255, with 0 standing
//! for "none". The localized HUD strings ([`strings`]) have a table of their own that counts from 1 and never hands a
//! number out twice.

mod names;
mod strings;

pub use self::names::{SOUND_ALIAS_TABLE_CAPACITY, SoundAliasConfigTable, label_in_listed};
pub use self::strings::{
    CS_LOCALIZED_STRINGS_SLOTS, HUD_PRINT_ARG_SEPARATOR, HUD_STRING_PLAIN, HudStringCs,
};

/// Materials the HUD always draws by name, whatever the scripts mention. They are numbered before anything else.
pub const MANDATORY_HUD_IMAGES: &[&str] = &["damage_feedback"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_feedback_is_a_required_material() {
        assert_eq!(MANDATORY_HUD_IMAGES, ["damage_feedback"]);
    }
}
