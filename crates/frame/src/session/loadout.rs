//! The loadout the class-select screen shows and the classes a host serves.

use bevy::prelude::Resource;

use crate::class_presets::{ClassPreset, showcase_classes};

/// One weapon offered on the class-select screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacWeaponOffer {
    pub key: String,
    /// The group the weapon sits in, when the catalog names one.
    pub item_group: Option<String>,
    pub attachments: Vec<String>,
}

impl From<String> for CacWeaponOffer {
    fn from(key: String) -> Self {
        Self {
            key,
            ..Self::default()
        }
    }
}

impl From<&str> for CacWeaponOffer {
    fn from(key: &str) -> Self {
        key.to_owned().into()
    }
}

/// What the class-select screen offers, handed over when it opens.
#[derive(Resource, Clone, Debug, Default)]
pub struct ClassSelectHandoff {
    /// Set when the screen has a new offer it has not shown yet.
    pub pending: bool,
    pub primary: Vec<CacWeaponOffer>,
    pub secondary: Vec<CacWeaponOffer>,
    pub lethal: Vec<CacWeaponOffer>,
    pub tactical: Vec<CacWeaponOffer>,
    /// Pairs of options that cannot be chosen together.
    pub excluded: Vec<(String, String)>,
    /// Per option, why it is locked (`None` when it is open).
    pub lock_reasons: Vec<Option<String>>,
}

/// One class a host serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostClassSlot {
    pub name: String,
    pub primary: String,
    pub primary_attachments: Vec<String>,
    pub secondary: String,
    pub secondary_attachments: Vec<String>,
    pub lethal: String,
    pub tactical: String,
    pub perks: [String; 3],
    pub deathstreak: String,
}

/// The classes a host serves. Starts as the first five stock loadouts the
/// game registered (none, until it registers any).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct HostClassLoadouts {
    pub slots: Vec<HostClassSlot>,
}

impl HostClassLoadouts {
    /// How many stock loadouts a host serves until it is told otherwise.
    const STOCK_SLOTS: usize = 5;

    fn stock(table: &[ClassPreset]) -> Self {
        Self {
            slots: table
                .iter()
                .take(Self::STOCK_SLOTS)
                .map(HostClassSlot::from)
                .collect(),
        }
    }
}

impl Default for HostClassLoadouts {
    fn default() -> Self {
        Self::stock(showcase_classes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class_presets::fixture::SIX;

    #[test]
    fn an_offer_made_from_a_key_has_no_group_and_no_attachments() {
        let offer = CacWeaponOffer::from("rifle_a");
        assert_eq!(offer.key, "rifle_a");
        assert_eq!(offer.item_group, None);
        assert!(offer.attachments.is_empty());
        assert_eq!(CacWeaponOffer::from(String::from("rifle_a")), offer);
    }

    #[test]
    fn a_fresh_handoff_has_nothing_pending() {
        let handoff = ClassSelectHandoff::default();
        assert!(!handoff.pending);
        assert!(handoff.primary.is_empty() && handoff.tactical.is_empty());
        assert!(handoff.excluded.is_empty() && handoff.lock_reasons.is_empty());
    }

    #[test]
    fn a_host_serves_the_first_five_stock_classes_in_order() {
        let slots = HostClassLoadouts::stock(&SIX).slots;
        let names: Vec<&str> = slots.iter().map(|slot| slot.name.as_str()).collect();
        assert_eq!(names, ["alpha", "bravo", "charlie", "delta", "echo"]);
        for (slot, preset) in slots.iter().zip(&SIX) {
            assert_eq!(slot, &HostClassSlot::from(preset));
        }
    }

    #[test]
    fn a_host_with_fewer_stock_classes_serves_all_of_them() {
        assert_eq!(HostClassLoadouts::stock(&SIX[..2]).slots.len(), 2);
    }

    #[test]
    fn with_no_stock_classes_a_host_serves_none() {
        assert!(HostClassLoadouts::stock(&[]).slots.is_empty());
    }
}
