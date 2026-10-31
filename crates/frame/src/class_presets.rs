//! The stock loadouts a game offers: a table the game registers at startup,
//! so no game's weapon or perk names live here.

use std::sync::RwLock;

use crate::run_state::ServerKitSlot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KitTemplate {
    pub name: &'static str,
    pub primary: &'static str,
    pub main_gun_parts: &'static [&'static str],
    pub secondary: &'static str,
    pub side_gun_parts: &'static [&'static str],
    pub lethal: &'static str,
    pub tactical: &'static str,
    /// Perk references; a game that fills them in later leaves them empty.
    pub perks: [&'static str; 3],
}

impl From<&KitTemplate> for ServerKitSlot {
    fn from(preset: &KitTemplate) -> Self {
        let owned = |items: &[&str]| items.iter().map(|item| (*item).to_owned()).collect();
        let text = |s: &str| s.to_owned();
        Self {
            main_gun_parts: owned(preset.main_gun_parts),
            side_gun_parts: owned(preset.side_gun_parts),
            perks: preset.perks.map(str::to_owned),
            name: text(preset.name),
            primary: text(preset.primary),
            secondary: text(preset.secondary),
            lethal: text(preset.lethal),
            tactical: text(preset.tactical),
        }
    }
}

/// A table of stock loadouts that can be swapped for another.
struct PresetTable(RwLock<&'static [KitTemplate]>);

impl PresetTable {
    const fn empty() -> Self {
        Self(RwLock::new(&[]))
    }

    fn get(&self) -> &'static [KitTemplate] {
        let read = self.0.read();
        *read.unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set(&self, table: &'static [KitTemplate]) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = table;
    }
}

static REGISTERED: PresetTable = PresetTable::empty();

/// Makes `table` the stock loadouts, presets first. Until a game does this
/// there are none.
pub fn register_showcase(table: &'static [KitTemplate]) {
    REGISTERED.set(table);
}

/// The stock loadouts the game registered, presets first.
pub fn showcase_classes() -> &'static [KitTemplate] {
    REGISTERED.get()
}

/// `count` distinct stock loadouts, chosen and ordered by `seed` (the same
/// seed always gives the same pick).
pub fn pick_showcase(seed: u64, count: usize) -> Vec<&'static KitTemplate> {
    pick_from(showcase_classes(), seed, count)
}

/// [`pick_showcase`] over any table.
pub(crate) fn pick_from(
    table: &'static [KitTemplate],
    seed: u64,
    count: usize,
) -> Vec<&'static KitTemplate> {
    let mut order: Vec<&'static KitTemplate> = table.iter().collect();
    let mut state = seed;
    for i in (1..order.len()).rev() {
        let j = (next_random(&mut state) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order.truncate(count);
    order
}

/// One step of splitmix64.
fn next_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

/// A table of made-up classes for tests, here and in the crates that read
/// the registered table.
#[cfg(test)]
pub(crate) mod fixture {
    use super::KitTemplate;

    const NO_ATTACHMENTS: &[&str] = &[];

    const fn class(name: &'static str, primary: &'static str) -> KitTemplate {
        KitTemplate {
            name,
            primary,
            main_gun_parts: &["sight"],
            secondary: "",
            side_gun_parts: NO_ATTACHMENTS,
            lethal: "test:lethal",
            tactical: "test:tactical",
            perks: ["perk_a", "perk_b", "perk_c"],
        }
    }

    pub static SIX: [KitTemplate; 6] = [
        class("alpha", "test:gun_a"),
        class("bravo", "test:gun_b"),
        class("charlie", "test:gun_c"),
        class("delta", "test:gun_d"),
        class("echo", "test:gun_e"),
        class("foxtrot", "test:gun_f"),
    ];
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::fixture::SIX;
    use super::*;

    fn names(picked: &[&KitTemplate]) -> Vec<&'static str> {
        picked.iter().map(|class| class.name).collect()
    }

    #[test]
    fn a_table_nobody_registered_into_is_empty() {
        let table = PresetTable::empty();
        assert!(table.get().is_empty());
    }

    #[test]
    fn setting_a_table_replaces_the_one_before() {
        let table = PresetTable::empty();
        table.set(&SIX);
        assert_eq!(table.get().len(), 6);
        table.set(&SIX[..2]);
        assert_eq!(
            names(&table.get().iter().collect::<Vec<_>>()),
            ["alpha", "bravo"]
        );
    }

    #[test]
    fn a_slot_made_from_a_preset_keeps_every_field() {
        let preset = &SIX[0];
        let slot = ServerKitSlot::from(preset);
        assert_eq!(slot.name, "alpha");
        assert_eq!(slot.primary, "test:gun_a");
        assert_eq!(slot.main_gun_parts, ["sight"]);
        assert_eq!(slot.secondary, "");
        assert!(slot.side_gun_parts.is_empty());
        assert_eq!(slot.lethal, "test:lethal");
        assert_eq!(slot.tactical, "test:tactical");
        assert_eq!(
            slot.perks,
            ["perk_a", "perk_b", "perk_c"].map(str::to_owned)
        );
    }

    #[test]
    fn the_same_seed_picks_the_same_classes() {
        assert_eq!(
            names(&pick_from(&SIX, 42, 4)),
            names(&pick_from(&SIX, 42, 4))
        );
    }

    #[test]
    fn a_pick_is_a_prefix_of_the_full_shuffle() {
        let full = names(&pick_from(&SIX, 7, 6));
        assert_eq!(names(&pick_from(&SIX, 7, 3)), full[..3]);
    }

    #[test]
    fn a_full_pick_is_a_permutation_of_the_table() {
        let picked: HashSet<&str> = names(&pick_from(&SIX, 99, 6)).into_iter().collect();
        let table: HashSet<&str> = SIX.iter().map(|c| c.name).collect();
        assert_eq!(picked, table);
    }

    #[test]
    fn a_pick_never_repeats_a_class() {
        for seed in 0..50 {
            let picked = names(&pick_from(&SIX, seed, 4));
            let unique: HashSet<&str> = picked.iter().copied().collect();
            assert_eq!(unique.len(), picked.len(), "seed {seed}");
        }
    }

    #[test]
    fn asking_for_more_than_exist_gives_all_of_them() {
        assert_eq!(pick_from(&SIX, 1, 99).len(), 6);
        assert!(pick_from(&SIX, 1, 0).is_empty());
    }

    #[test]
    fn picking_from_an_empty_table_gives_nothing() {
        assert!(pick_from(&[], 5, 3).is_empty());
    }

    #[test]
    fn different_seeds_give_different_orders() {
        let orders: HashSet<Vec<&str>> = (0..20)
            .map(|seed| names(&pick_from(&SIX, seed, 6)))
            .collect();
        assert!(orders.len() > 5);
    }

    /// The only test that reads or writes the process-wide table, so the
    /// others in this crate cannot race it.
    #[test]
    fn the_registered_table_is_what_the_game_reads_back() {
        use crate::run_state::ServerKitRoster;

        register_showcase(&SIX);
        assert_eq!(showcase_classes().len(), 6);
        assert_eq!(pick_showcase(3, 4), pick_from(&SIX, 3, 4));
        let slots = ServerKitRoster::default().slots;
        assert_eq!(slots.len(), 5);
        assert_eq!(slots[0], ServerKitSlot::from(&SIX[0]));
        register_showcase(&[]);
        assert!(showcase_classes().is_empty());
        assert!(ServerKitRoster::default().slots.is_empty());
    }
}
