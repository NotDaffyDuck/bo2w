//! Classes and loadouts: what a player picked, and the catalog the picks index into.

use crate::input::ClassId;

pub use super::reasons::{ClassRejectReason, ConfigurationChangeRejectReason, GiveRejectReason};

/// How many classes a player can build for themselves.
pub const PERSONAL_CLASS_SLOTS: usize = 5;

/// The death-streak perks a class can carry, picked by 1-based position (0 means none).
pub const CLASS_CATALOG_DEATHSTREAKS: [&str; 4] =
    ["specialty_copycat", "specialty_combathigh", "specialty_grenadepulldeath", "specialty_finalstand"];

/// The perks a class can carry, picked by 1-based position.
pub const CLASS_CATALOG_PERKS: [&str; 16] = [
    "specialty_bulletdamage",
    "specialty_fastreload",
    "specialty_coldblooded",
    "specialty_lightweight",
    "specialty_scavenger",
    "specialty_hardline",
    "specialty_heartbreaker",
    "specialty_marathon",
    "specialty_explosivedamage",
    "specialty_extendedmelee",
    "specialty_bulletaccuracy",
    "specialty_bling",
    "specialty_onemanarmy",
    "specialty_localjammer",
    "specialty_detectexplosive",
    "specialty_pistoldeath",
];

/// The loadout a player is carrying: a class and the revision of it they spawned with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadoutSpec {
    pub class_id: ClassId,
    pub revision: u32,
    pub primary: u32,
    pub secondary: u32,
    pub primary_attachments: [u32; 4],
    pub secondary_attachments: [u32; 4],
    pub lethal: u32,
    pub tactical: u32,
    pub perks: [u32; 3],
}

/// A full class as the authority holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassDef {
    pub id: ClassId,
    pub revision: u32,
    pub primary: u32,
    pub secondary: u32,
    pub primary_attachments: [u32; 4],
    pub secondary_attachments: [u32; 4],
    pub lethal: u32,
    pub tactical: u32,
    pub perks: [u32; 3],
    pub deathstreak: String,
    pub locked: bool,
}

impl ClassDef {
    /// A class with two guns and nothing else.
    pub fn primary_secondary(id: ClassId, revision: u32, primary: u32, secondary: u32) -> Self {
        Self {
            id,
            revision,
            primary,
            secondary,
            primary_attachments: [0; 4],
            secondary_attachments: [0; 4],
            lethal: 0,
            tactical: 0,
            perks: [0; 3],
            deathstreak: String::new(),
            locked: false,
        }
    }

    /// Primary, secondary, lethal and tactical, in that order.
    pub fn weapon_slot_ids(&self) -> [u32; 4] {
        [self.primary, self.secondary, self.lethal, self.tactical]
    }
}

/// A class a player built themselves, stored as catalog picks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PersonalClass {
    pub weapons: [u32; 4],
    pub perks: [u32; 3],
    pub deathstreak: u8,
}

impl PersonalClass {
    /// The class as a definition. `None` for revision 0 (never saved) or a death-streak pick off the catalog.
    pub fn definition(self, id: ClassId, revision: u32) -> Option<ClassDef> {
        if revision == 0 {
            return None;
        }
        let [primary, secondary, lethal, tactical] = self.weapons;
        let deathstreak = match usize::from(self.deathstreak) {
            0 => String::new(),
            pick => CLASS_CATALOG_DEATHSTREAKS.get(pick - 1)?.to_string(),
        };
        Some(ClassDef {
            lethal,
            tactical,
            perks: self.perks,
            deathstreak,
            ..ClassDef::primary_secondary(id, revision, primary, secondary)
        })
    }
}
