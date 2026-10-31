use super::LoadoutClassSpec;
use crate::tick_feed::LoadoutClass;

/// How many classes a player can build for themselves.
pub const PERSONAL_CLASS_SLOTS: usize = 5;

/// A class a player built, kept as the picks they made: four weapon ids (primary, secondary, lethal, tactical) and
/// three perk numbers from the stats table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PersonalClass {
    pub weapons: [u32; 4],
    pub perks: [u32; 3],
}

impl PersonalClass {
    /// This class as a full definition, or `None` when revision 0 says it was never saved.
    pub fn definition(self, id: LoadoutClass, revision: u32) -> Option<LoadoutClassSpec> {
        let saved = revision != 0;
        let [primary, side_gun, lethal, tactical] = self.weapons;
        saved.then(|| LoadoutClassSpec { lethal, tactical, perks: self.perks, ..LoadoutClassSpec::primary_secondary(id, revision, primary, side_gun) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsaved_class_has_no_definition() {
        let class = PersonalClass { weapons: [1, 2, 3, 4], perks: [5, 6, 7] };
        assert_eq!(class.definition(LoadoutClass::default(), 0), None);
    }

    #[test]
    fn a_saved_class_carries_every_pick() {
        let class = PersonalClass { weapons: [1, 2, 3, 4], perks: [5, 6, 7] };
        let def = class.definition(LoadoutClass::default(), 2).expect("revision 2 is saved");
        assert_eq!(def.gun_slot_ids(), [1, 2, 3, 4]);
        assert_eq!(def.perks, [5, 6, 7]);
        assert_eq!(def.revision, 2);
    }
}
