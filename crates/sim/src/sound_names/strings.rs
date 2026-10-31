//! The localized HUD strings: numbered from 1 in order of first use, never given up.

/// The slot count of the HUD string table, counting the unused slot 0.
pub const CS_LOCALIZED_STRINGS_SLOTS: usize = 0x200;

/// Marks a string as plain text rather than a localized reference.
pub const HUD_STRING_PLAIN: char = '\u{15}';

/// Separates the parts of a string that has arguments.
pub const HUD_PRINT_ARG_SEPARATOR: char = '\u{16}';

/// The HUD strings, numbered from 1 in order of first use. Unlike the name tables it never gives a slot up: when
/// it is full, new strings get no number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudStringCs {
    /// String number `n` is stored at `held[n - 1]`.
    held: Vec<String>,
}

impl HudStringCs {
    /// The number of `text`: 0 for the empty string, else its existing number or the next free one. `None` when the
    /// table has no room.
    pub fn number_of(&mut self, wanted: &str) -> Option<i32> {
        if str::is_empty(wanted) {
            return Some(i32::default());
        }
        let at = match self.held.iter().position(|s| s == wanted) {
            Some(known) => known,
            None if self.held.len() + 1 >= CS_LOCALIZED_STRINGS_SLOTS => return None,
            None => {
                self.held.push(wanted.to_owned());
                self.held.len() - 1
            }
        };
        i32::try_from(at + 1).ok()
    }

    /// Makes the table equal to the numbered strings from a snapshot. Numbers that were not sent become empty
    /// strings; 0 and numbers past the table are skipped.
    pub fn take_listed(&mut self, sent: &[(u16, String)]) {
        self.held = Vec::new();
        let usable = sent.iter().filter(|(n, _)| (1..CS_LOCALIZED_STRINGS_SLOTS).contains(&usize::from(*n)));
        for (slot, wanted) in usable {
            let slot = usize::from(*slot);
            if self.held.len() < slot {
                self.held.resize(slot, String::default());
            }
            self.held[slot - 1].clone_from(wanted);
        }
    }

    /// All strings with their numbers, ascending. A gap left by an adoption appears as an empty string.
    pub fn occupied(&self) -> Vec<(u16, String)> {
        (1u16..).zip(&self.held).map(|(number, text)| (number, text.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_strings_count_from_one_and_the_empty_string_is_zero() {
        let mut strings = HudStringCs::default();
        assert_eq!(strings.number_of(""), Some(0));
        assert_eq!(strings.number_of("hello"), Some(1));
        assert_eq!(strings.number_of("world"), Some(2));
        assert_eq!(strings.number_of("hello"), Some(1));
        assert_eq!(strings.occupied(), vec![(1, "hello".to_owned()), (2, "world".to_owned())]);
    }

    #[test]
    fn a_full_hud_string_table_takes_no_new_strings() {
        let mut strings = HudStringCs::default();
        for n in 1..CS_LOCALIZED_STRINGS_SLOTS {
            assert_eq!(strings.number_of(&format!("text{n}")), Some(n as i32));
        }
        assert_eq!(strings.number_of("one too many"), None);
        assert_eq!(strings.number_of("text1"), Some(1), "known strings are still found");
    }

    #[test]
    fn adopting_hud_strings_fills_gaps_with_empty_ones() {
        let mut strings = HudStringCs::default();
        strings.number_of("dropped");
        let sent = [(3, "third".to_owned()), (0, "zero".to_owned()), (9999, "far".to_owned())];
        strings.take_listed(&sent);
        let expected = vec![(1, String::default()), (2, String::default()), (3, "third".to_owned())];
        assert_eq!(strings.occupied(), expected);
        assert_eq!(strings.number_of("third"), Some(3));
        assert_eq!(strings.number_of("next"), Some(4));
    }

    #[test]
    fn the_string_markers_are_control_characters_21_and_22() {
        assert_eq!(u32::from(HUD_STRING_PLAIN), 0x15);
        assert_eq!(u32::from(HUD_PRINT_ARG_SEPARATOR), 0x16);
    }
}
