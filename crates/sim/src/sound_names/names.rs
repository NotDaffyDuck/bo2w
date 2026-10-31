//! The name table: up to 255 numbered names that drop the least recently asked for when full.

use std::sync::Once;

/// The slot count of the name tables. Slot 0 is reserved and never assigned.
pub const SOUND_ALIAS_TABLE_CAPACITY: usize = 0x100;

const _: () = assert!(SOUND_ALIAS_TABLE_CAPACITY <= 1 << u8::BITS);

/// One entry of a name table.
#[derive(Clone, Debug, Default)]
struct Entry {
    label: Option<String>,
    /// Value of the table's ask counter the last time this name was asked for. A full table gives up the lowest.
    last_asked: u64,
}

/// A name table with up to 255 numbered slots. When it is full the name that was asked for longest ago is dropped,
/// since a Zombies game names many more sounds than fit.
#[derive(Clone, Debug)]
pub struct SoundAliasConfigTable {
    slots: Vec<Entry>,
    asks: u64,
}

impl Default for SoundAliasConfigTable {
    fn default() -> Self {
        Self { slots: vec![Entry::default(); SOUND_ALIAS_TABLE_CAPACITY], asks: 0 }
    }
}

/// Tables are equal when the same names hold the same numbers; how recently they were asked for is not compared.
impl PartialEq for SoundAliasConfigTable {
    fn eq(&self, rhs: &Self) -> bool {
        self.slots.iter().map(|slot| &slot.label).eq(rhs.slots.iter().map(|slot| &slot.label))
    }
}

impl Eq for SoundAliasConfigTable {}

impl SoundAliasConfigTable {
    /// The number of `name`. A new name takes the lowest empty slot, or the slot of the name asked for longest ago
    /// when none is empty. The empty name is number 0.
    pub fn number_of(&mut self, alias: &str) -> u8 {
        if alias.is_empty() {
            return 0;
        }
        self.asks += 1;
        let now = self.asks;
        let numbered = &mut self.slots[1..];

        let at = match numbered.iter().position(|entry| entry.label.as_deref() == Some(alias)) {
            Some(known) => known,
            None => {
                let target = Self::place_for_new(numbered);
                numbered[target].label = Some(alias.to_owned());
                target
            }
        };
        numbered[at].last_asked = now;
        Self::number_at(at)
    }

    /// Which numbered slot a new name goes to: the first empty one, else the one asked for longest ago.
    fn place_for_new(numbered: &[Entry]) -> usize {
        if let Some(empty) = numbered.iter().position(|entry| entry.label.is_none()) {
            return empty;
        }
        static WARNED: Once = Once::new();
        WARNED.call_once(|| {
            diag::info!(Sim, "every sound alias slot ({SOUND_ALIAS_TABLE_CAPACITY}) is taken; the longest-unused ones get reused");
        });
        // `min_by_key` keeps the first of equal keys, so ties go to the lowest number.
        numbered
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.last_asked)
            .map_or(0, |(at, _)| at)
    }

    /// The slot number of position `at` in the numbered part of the table.
    fn number_at(at: usize) -> u8 {
        u8::try_from(at + 1).expect("the table has at most 255 numbered slots")
    }

    /// The name held by `slot`, when there is one.
    pub fn label_of(&self, slot: u8) -> Option<&str> {
        self.slots.get(usize::from(slot))?.label.as_deref()
    }

    /// All numbered names, ascending by number.
    pub fn occupied(&self) -> Vec<(u8, String)> {
        self.slots[1..]
            .iter()
            .enumerate()
            .filter_map(|(at, entry)| Some((Self::number_at(at), entry.label.clone()?)))
            .collect()
    }

    /// Makes the table equal to the numbered names from a snapshot. Entries numbered 0 are skipped.
    pub fn take_listed(&mut self, listed: &[(u8, String)]) {
        *self = Self::default();
        for (slot, label) in listed.iter().filter(|(slot, _)| *slot != 0) {
            self.slots[usize::from(*slot)].label = Some(label.clone());
        }
    }
}

/// Looks `index` up in the numbered names of a snapshot.
pub fn label_in_listed(listed: &[(u8, String)], wanted: u8) -> Option<&str> {
    let (_, name) = listed.iter().find(|(number, _)| *number == wanted)?;
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_at(table: &SoundAliasConfigTable, number: u8) -> &str {
        table.label_of(number).unwrap_or("<none>")
    }

    fn pairs(items: &[(u8, &str)]) -> Vec<(u8, String)> {
        items.iter().map(|&(number, name)| (number, name.to_owned())).collect()
    }

    #[test]
    fn the_empty_name_is_number_zero_and_uses_no_slot() {
        let mut table = SoundAliasConfigTable::default();
        assert_eq!(table.number_of(""), 0);
        assert!(table.occupied().is_empty());
        assert_eq!(table.label_of(0), None);
    }

    #[test]
    fn numbers_run_up_from_one_in_the_order_of_asking() {
        let mut table = SoundAliasConfigTable::default();
        assert_eq!(table.number_of("shot"), 1);
        assert_eq!(table.number_of("reload"), 2);
        assert_eq!(table.number_of("shot"), 1, "a second ask returns the same number");
        assert_eq!(name_at(&table, 2), "reload");
        assert_eq!(table.label_of(3), None);
    }

    #[test]
    fn the_listing_starts_at_the_lowest_slot() {
        let mut table = SoundAliasConfigTable::default();
        for name in ["a", "b", "c"] {
            table.number_of(name);
        }
        assert_eq!(table.occupied(), pairs(&[(1, "a"), (2, "b"), (3, "c")]));
    }

    #[test]
    fn an_adopted_list_replaces_the_table_and_new_names_take_the_gaps() {
        let mut table = SoundAliasConfigTable::default();
        table.number_of("old");
        table.take_listed(&pairs(&[(3, "three"), (0, "ignored"), (5, "five")]));
        assert_eq!(table.occupied(), pairs(&[(3, "three"), (5, "five")]));
        let given: Vec<u8> = ["first", "second", "three", "fourth", "sixth"]
            .into_iter()
            .map(|name| table.number_of(name))
            .collect();
        assert_eq!(given, [1, 2, 3, 4, 6]);
    }

    #[test]
    fn a_full_table_drops_the_name_asked_for_longest_ago() {
        let mut table = SoundAliasConfigTable::default();
        let numbered = SOUND_ALIAS_TABLE_CAPACITY - 1;
        for n in 0..numbered {
            assert_eq!(usize::from(table.number_of(&format!("sound{n}"))), n + 1);
        }
        assert_eq!(table.occupied().len(), numbered);
        table.number_of("sound0");
        assert_eq!(table.number_of("new"), 2, "sound0 was just asked again, so sound1 is the oldest");
        assert_eq!(name_at(&table, 2), "new");
        assert_eq!(name_at(&table, 1), "sound0");
        assert_eq!(table.occupied().len(), numbered);
    }

    #[test]
    fn after_an_adoption_the_lowest_slot_goes_first() {
        let mut table = SoundAliasConfigTable::default();
        let all: Vec<(u8, String)> = (1..=255u8).map(|n| (n, format!("s{n}"))).collect();
        table.take_listed(&all);
        assert_eq!([table.number_of("extra"), table.number_of("extra2")], [1, 2]);
    }

    #[test]
    fn tables_compare_by_their_numbered_names_alone() {
        let (mut left, mut right) = (SoundAliasConfigTable::default(), SoundAliasConfigTable::default());
        for name in ["x", "y"] {
            left.number_of(name);
            right.number_of(name);
        }
        right.number_of("x");
        assert_eq!(left, right);
        right.number_of("z");
        assert_ne!(left, right);
    }

    #[test]
    fn a_snapshot_list_is_searched_by_number() {
        let list = pairs(&[(2, "two"), (9, "nine")]);
        assert_eq!(label_in_listed(&list, 9), Some("nine"));
        assert_eq!(label_in_listed(&list, 3), None);
        assert_eq!(label_in_listed(&[], 0), None);
    }
}
