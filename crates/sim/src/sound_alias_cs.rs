//! Config-string tables. The host numbers a name once, and a snapshot then carries the number ("sound 7") rather than
//! the name each time.
//!
//! One table type serves sound aliases, effect names and HUD materials: numbers 1 to 255, with 0 standing for "none".
//! The localized HUD strings have a table of their own that counts from 1 and never hands a number out twice.

/// The slot count of the name tables. Slot 0 is reserved and never assigned.
pub const CS_SOUNDALIASES_SLOTS: usize = 0x100;

const _: () = assert!(CS_SOUNDALIASES_SLOTS <= 1 << u8::BITS);

/// The numbered names of a name table, in the form a snapshot sends them.
pub type SoundAliasCsOccupied = Vec<(u8, String)>;

/// One entry of a name table.
#[derive(Clone, Debug, Default)]
struct Slot {
    name: Option<String>,
    /// Value of the lookup counter at the latest lookup of this name. A full table gives up the lowest one.
    last_asked: u64,
}

/// A name table with up to 255 numbered slots. When it is full the name that was asked for longest ago is dropped,
/// since a Zombies game names many more sounds than fit.
#[derive(Clone, Debug)]
pub struct SoundAliasCs {
    slots: Vec<Slot>,
    asks: u64,
}

impl Default for SoundAliasCs {
    fn default() -> Self {
        Self {
            slots: vec![Slot::default(); CS_SOUNDALIASES_SLOTS],
            asks: 0,
        }
    }
}

/// Tables are equal when the same names hold the same numbers; how recently they were asked for is not compared.
impl PartialEq for SoundAliasCs {
    fn eq(&self, other: &Self) -> bool {
        let names = |table: &Self| table.slots.iter().map(|slot| slot.name.clone()).collect::<Vec<_>>();
        names(self) == names(other)
    }
}

impl Eq for SoundAliasCs {}

impl SoundAliasCs {
    /// The number of `name`. A new name takes the lowest empty slot, or the slot of the name asked for longest ago
    /// when none is empty. The empty name is number 0.
    pub fn index(&mut self, name: &str) -> u8 {
        if name.is_empty() {
            return 0;
        }
        self.asks += 1;
        let now = self.asks;
        let numbered = &mut self.slots[1..];

        let position = match numbered.iter().position(|slot| slot.name.as_deref() == Some(name)) {
            Some(known) => known,
            None => {
                let chosen = Self::slot_to_fill(numbered);
                numbered[chosen].name = Some(name.to_owned());
                chosen
            }
        };
        numbered[position].last_asked = now;
        Self::number(position)
    }

    /// Which numbered slot a new name goes to: the first empty one, else the least recently asked for.
    fn slot_to_fill(numbered: &[Slot]) -> usize {
        if let Some(empty) = numbered.iter().position(|slot| slot.name.is_none()) {
            return empty;
        }
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            diag::info!(
                Sim,
                "sound alias table full ({CS_SOUNDALIASES_SLOTS}); reusing the longest-unused slots"
            );
        });
        let mut oldest = 0;
        for (position, slot) in numbered.iter().enumerate() {
            if slot.last_asked < numbered[oldest].last_asked {
                oldest = position;
            }
        }
        oldest
    }

    /// Converts a position in the numbered part of the table to its slot number.
    fn number(position: usize) -> u8 {
        u8::try_from(position + 1).expect("the table has at most 255 numbered slots")
    }

    /// The name held by slot `index`, when there is one.
    pub fn name(&self, index: u8) -> Option<&str> {
        self.slots.get(usize::from(index))?.name.as_deref()
    }

    /// All numbered names, ascending by number.
    pub fn occupied(&self) -> SoundAliasCsOccupied {
        let mut list = Vec::new();
        for (position, slot) in self.slots[1..].iter().enumerate() {
            if let Some(name) = &slot.name {
                list.push((Self::number(position), name.clone()));
            }
        }
        list
    }

    /// Makes the table equal to the numbered names from a snapshot. Entries numbered 0 are skipped.
    pub fn adopt_occupied(&mut self, occupied: &[(u8, String)]) {
        *self = Self::default();
        for &(number, ref name) in occupied {
            if number != 0 {
                self.slots[usize::from(number)].name = Some(name.clone());
            }
        }
    }
}

/// Looks `index` up in the numbered names of a snapshot.
pub fn name_in_occupied(occupied: &[(u8, String)], index: u8) -> Option<&str> {
    occupied
        .iter()
        .find_map(|(number, name)| (*number == index).then_some(name.as_str()))
}

/// HUD materials use the same numbering as sounds.
pub type HudMaterialCs = SoundAliasCs;
pub type HudMaterialCsOccupied = SoundAliasCsOccupied;

/// Materials the HUD always draws by name, whatever the scripts mention. They are numbered before anything else.
pub const REQUIRED_HUD_MATERIALS: &[&str] = &["damage_feedback"];

/// Effect names use the same numbering as sounds.
pub type EffectNameCs = SoundAliasCs;
pub type EffectNameCsOccupied = SoundAliasCsOccupied;

/// The slot count of the HUD string table, counting the unused slot 0.
pub const CS_LOCALIZED_STRINGS_SLOTS: usize = 0x200;

/// Marks a string as plain text rather than a localized reference.
pub const HUD_STRING_PLAIN: char = '\u{15}';

/// Separates the parts of a string that has arguments.
pub const HUD_PRINT_ARG_SEPARATOR: char = '\u{16}';

/// The numbered HUD strings, in the form a snapshot sends them.
pub type HudStringCsOccupied = Vec<(u16, String)>;

/// The HUD strings, numbered from 1 in order of first use. Unlike the name tables it never gives a slot up: when
/// it is full, new strings get no number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudStringCs {
    /// String number `n` is stored at `strings[n - 1]`.
    strings: Vec<String>,
}

impl HudStringCs {
    /// The number of `text`: 0 for the empty string, else its existing number or the next free one. `None` when the
    /// table has no room.
    pub fn index(&mut self, text: &str) -> Option<i32> {
        if text.is_empty() {
            return Some(0);
        }
        if let Some(known) = self.strings.iter().position(|held| held == text) {
            return i32::try_from(known + 1).ok();
        }
        if self.strings.len() + 1 >= CS_LOCALIZED_STRINGS_SLOTS {
            return None;
        }
        self.strings.push(text.to_owned());
        i32::try_from(self.strings.len()).ok()
    }

    /// Makes the table equal to the numbered strings from a snapshot. Numbers that were not sent become empty
    /// strings; 0 and numbers past the table are skipped.
    pub fn adopt_occupied(&mut self, occupied: &[(u16, String)]) {
        self.strings.clear();
        for (raw, text) in occupied {
            let number = usize::from(*raw);
            if number == 0 || number >= CS_LOCALIZED_STRINGS_SLOTS {
                continue;
            }
            if number > self.strings.len() {
                self.strings.resize(number, String::new());
            }
            self.strings[number - 1].clone_from(text);
        }
    }

    /// All strings with their numbers, ascending. A gap left by an adoption appears as an empty string.
    pub fn occupied(&self) -> HudStringCsOccupied {
        self.strings
            .iter()
            .enumerate()
            .map(|(at, text)| (at as u16 + 1, text.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_at(table: &SoundAliasCs, number: u8) -> &str {
        table.name(number).unwrap_or("<none>")
    }

    fn pairs(items: &[(u8, &str)]) -> SoundAliasCsOccupied {
        items.iter().map(|&(number, name)| (number, name.to_owned())).collect()
    }

    #[test]
    fn the_empty_name_is_number_zero_and_uses_no_slot() {
        let mut table = SoundAliasCs::default();
        assert_eq!(table.index(""), 0);
        assert!(table.occupied().is_empty());
        assert_eq!(table.name(0), None);
    }

    #[test]
    fn numbers_run_up_from_one_in_the_order_of_asking() {
        let mut table = SoundAliasCs::default();
        assert_eq!(table.index("shot"), 1);
        assert_eq!(table.index("reload"), 2);
        assert_eq!(table.index("shot"), 1, "a second ask returns the same number");
        assert_eq!(name_at(&table, 2), "reload");
        assert_eq!(table.name(3), None);
    }

    #[test]
    fn the_listing_starts_at_the_lowest_slot() {
        let mut table = SoundAliasCs::default();
        for name in ["a", "b", "c"] {
            table.index(name);
        }
        assert_eq!(table.occupied(), pairs(&[(1, "a"), (2, "b"), (3, "c")]));
    }

    #[test]
    fn an_adopted_list_replaces_the_table_and_new_names_take_the_gaps() {
        let mut table = SoundAliasCs::default();
        table.index("old");
        table.adopt_occupied(&pairs(&[(3, "three"), (0, "ignored"), (5, "five")]));
        assert_eq!(table.occupied(), pairs(&[(3, "three"), (5, "five")]));
        let given: Vec<u8> = ["first", "second", "three", "fourth", "sixth"]
            .into_iter()
            .map(|name| table.index(name))
            .collect();
        assert_eq!(given, [1, 2, 3, 4, 6]);
    }

    #[test]
    fn a_full_table_drops_the_name_asked_for_longest_ago() {
        let mut table = SoundAliasCs::default();
        let numbered = CS_SOUNDALIASES_SLOTS - 1;
        for n in 0..numbered {
            assert_eq!(usize::from(table.index(&format!("sound{n}"))), n + 1);
        }
        assert_eq!(table.occupied().len(), numbered);
        table.index("sound0");
        assert_eq!(table.index("new"), 2, "sound0 was just asked again, so sound1 is the oldest");
        assert_eq!(name_at(&table, 2), "new");
        assert_eq!(name_at(&table, 1), "sound0");
        assert_eq!(table.occupied().len(), numbered);
    }

    #[test]
    fn after_an_adoption_the_lowest_slot_goes_first() {
        let mut table = SoundAliasCs::default();
        let all: Vec<(u8, String)> = (1..=255u8).map(|n| (n, format!("s{n}"))).collect();
        table.adopt_occupied(&all);
        assert_eq!([table.index("extra"), table.index("extra2")], [1, 2]);
    }

    #[test]
    fn tables_compare_by_their_numbered_names_alone() {
        let (mut left, mut right) = (SoundAliasCs::default(), SoundAliasCs::default());
        for name in ["x", "y"] {
            left.index(name);
            right.index(name);
        }
        right.index("x");
        assert_eq!(left, right);
        right.index("z");
        assert_ne!(left, right);
    }

    #[test]
    fn a_snapshot_list_is_searched_by_number() {
        let list = pairs(&[(2, "two"), (9, "nine")]);
        assert_eq!(name_in_occupied(&list, 9), Some("nine"));
        assert_eq!(name_in_occupied(&list, 3), None);
        assert_eq!(name_in_occupied(&[], 0), None);
    }

    #[test]
    fn hud_strings_count_from_one_and_the_empty_string_is_zero() {
        let mut strings = HudStringCs::default();
        assert_eq!(strings.index(""), Some(0));
        assert_eq!(strings.index("hello"), Some(1));
        assert_eq!(strings.index("world"), Some(2));
        assert_eq!(strings.index("hello"), Some(1));
        assert_eq!(strings.occupied(), vec![(1, "hello".to_owned()), (2, "world".to_owned())]);
    }

    #[test]
    fn a_full_hud_string_table_takes_no_new_strings() {
        let mut strings = HudStringCs::default();
        for n in 1..CS_LOCALIZED_STRINGS_SLOTS {
            assert_eq!(strings.index(&format!("text{n}")), Some(n as i32));
        }
        assert_eq!(strings.index("one too many"), None);
        assert_eq!(strings.index("text1"), Some(1), "known strings are still found");
    }

    #[test]
    fn adopting_hud_strings_fills_gaps_with_empty_ones() {
        let mut strings = HudStringCs::default();
        strings.index("dropped");
        let sent = [(3, "third".to_owned()), (0, "zero".to_owned()), (9999, "far".to_owned())];
        strings.adopt_occupied(&sent);
        let expected = vec![(1, String::new()), (2, String::new()), (3, "third".to_owned())];
        assert_eq!(strings.occupied(), expected);
        assert_eq!(strings.index("third"), Some(3));
        assert_eq!(strings.index("next"), Some(4));
    }

    #[test]
    fn the_string_markers_are_control_characters_21_and_22() {
        assert_eq!(u32::from(HUD_STRING_PLAIN), 0x15);
        assert_eq!(u32::from(HUD_PRINT_ARG_SEPARATOR), 0x16);
    }

    #[test]
    fn damage_feedback_is_a_required_material() {
        assert_eq!(REQUIRED_HUD_MATERIALS, ["damage_feedback"]);
    }
}
