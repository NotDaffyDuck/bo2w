//! The loadout the class-select screen shows and the classes a host serves.

use bevy::prelude::Resource;

use crate::class_presets::{KitTemplate, showcase_classes};

/// One weapon offered on the class-select screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomClassGunChoice {
    pub key: String,
    /// The group the weapon sits in, when the catalog names one.
    pub loot_bundle: Option<String>,
    pub attachments: Vec<String>,
}

impl From<String> for CustomClassGunChoice {
    fn from(key: String) -> Self {
        Self {
            key,
            ..Self::default()
        }
    }
}

impl From<&str> for CustomClassGunChoice {
    fn from(key: &str) -> Self {
        key.to_owned().into()
    }
}

/// What the class-select screen offers, handed over when it opens.
#[derive(Resource, Clone, Debug, Default)]
pub struct KitPickerPassover {
    /// Set when the screen has a new offer it has not shown yet.
    pub pending: bool,
    pub primary: Vec<CustomClassGunChoice>,
    pub secondary: Vec<CustomClassGunChoice>,
    pub lethal: Vec<CustomClassGunChoice>,
    pub tactical: Vec<CustomClassGunChoice>,
    /// Pairs of options that cannot be chosen together.
    pub excluded: Vec<(String, String)>,
    /// Per option, why it is locked (`None` when it is open).
    pub locked_because: Vec<Option<String>>,
}

/// One class a host serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerKitSlot {
    pub name: String,
    pub primary: String,
    pub main_gun_parts: Vec<String>,
    pub secondary: String,
    pub side_gun_parts: Vec<String>,
    pub lethal: String,
    pub tactical: String,
    pub perks: [String; 3],
}

/// The classes a host serves. Starts as the first five stock loadouts the
/// game registered (none, until it registers any).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct ServerKitRoster {
    pub slots: Vec<ServerKitSlot>,
}

impl ServerKitRoster {
    /// How many stock loadouts a host serves until it is told otherwise.
    const STOCK_SLOTS: usize = 5;

    fn stock(table: &[KitTemplate]) -> Self {
        Self {
            slots: table
                .iter()
                .take(Self::STOCK_SLOTS)
                .map(ServerKitSlot::from)
                .collect(),
        }
    }
}

impl Default for ServerKitRoster {
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
        let offer = CustomClassGunChoice::from("rifle_a");
        assert_eq!(offer.key, "rifle_a");
        assert_eq!(offer.loot_bundle, None);
        assert!(offer.attachments.is_empty());
        assert_eq!(CustomClassGunChoice::from(String::from("rifle_a")), offer);
    }

    #[test]
    fn a_fresh_handoff_has_nothing_pending() {
        let handoff = KitPickerPassover::default();
        assert!(!handoff.pending);
        assert!(handoff.primary.is_empty() && handoff.tactical.is_empty());
        assert!(handoff.excluded.is_empty() && handoff.locked_because.is_empty());
    }

    #[test]
    fn a_host_serves_the_first_five_stock_classes_in_order() {
        let slots = ServerKitRoster::stock(&SIX).slots;
        let names: Vec<&str> = slots.iter().map(|slot| slot.name.as_str()).collect();
        assert_eq!(names, ["alpha", "bravo", "charlie", "delta", "echo"]);
        for (slot, preset) in slots.iter().zip(&SIX) {
            assert_eq!(slot, &ServerKitSlot::from(preset));
        }
    }

    #[test]
    fn a_host_with_fewer_stock_classes_serves_all_of_them() {
        assert_eq!(ServerKitRoster::stock(&SIX[..2]).slots.len(), 2);
    }

    #[test]
    fn with_no_stock_classes_a_host_serves_none() {
        assert!(ServerKitRoster::stock(&[]).slots.is_empty());
    }
}
