use crate::tick_feed::LoadoutClass;

/// A class the way the authority stores it. `locked` classes cannot be edited by the player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadoutClassSpec {
    pub no: crate::tick_feed::LoadoutClass,
    pub revision: u32,
    pub primary: u32,
    pub secondary: u32,
    pub primary_attachments: [u32; 4],
    pub secondary_attachments: [u32; 4],
    pub lethal: u32,
    pub tactical: u32,
    pub perks: [u32; 3],
    pub locked: bool,
}

impl LoadoutClassSpec {
    /// A class holding two guns and nothing else.
    pub fn primary_secondary(seat: LoadoutClass, revision: u32, first_gun: u32, second_gun: u32) -> Self {
        Self { no: seat, revision, primary: first_gun, secondary: second_gun, ..Self::default() }
    }

    /// The weapon ids by slot: primary, secondary, lethal, tactical.
    pub fn gun_slot_ids(&self) -> [u32; 4] {
        let Self { primary: gun_a, secondary: gun_b, lethal: grenade, tactical: gadget, .. } = *self;
        [gun_a, gun_b, grenade, gadget]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_guns_leave_the_rest_empty() {
        let class = LoadoutClassSpec::primary_secondary(LoadoutClass::default(), 3, 11, 22);
        assert_eq!(class.gun_slot_ids(), [11, 22, 0, 0]);
        assert_eq!(class.perks, [0; 3]);
        assert!(!class.locked);
        assert_eq!(class.revision, 3);
    }
}
