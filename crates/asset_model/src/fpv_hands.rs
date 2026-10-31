//! Which arms model a first-person weapon wears.

use asset_core::AssetCategoryScope;

use crate::FirstPersonMeshRoster;
use crate::TrooperOutfit;
use crate::arms_matching_body;
use crate::is_arms_model;

/// The arms Black Ops II gives the first player (the map script's
/// `setviewmodel`); on Nuketown that is CIA, in a suit.
const DEFAULT_ARMS: &str = "c_zom_suit_viewhands";

/// The arms chosen for a weapon, and what chose them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstPersonArms {
    /// The soldier kit names them, or they match the kit's body.
    FromKit {
        namespace: AssetCategoryScope,
        name: String,
    },
    /// The weapon's own definition names them.
    DerivedFromWeaponSpec {
        namespace: AssetCategoryScope,
        name: String,
    },
    /// Neither said, so these are the map's stand-in.
    StockDefault {
        namespace: AssetCategoryScope,
        name: String,
    },
    /// No arms are loaded.
    NothingLoaded,
}

impl FirstPersonArms {
    pub fn key(&self) -> Option<(AssetCategoryScope, &str)> {
        match self {
            FirstPersonArms::FromKit { namespace: space, name: label }
            | Self::DerivedFromWeaponSpec { namespace: space, name: label }
            | Self::StockDefault { namespace: space, name: label } => Some((*space, label.as_str())),
            Self::NothingLoaded => None,
        }
    }

    /// Picks arms from what the set holds for the map's namespace: the kit's,
    /// then the weapon's, then the stand-in.
    pub fn choose(
        loaded: &FirstPersonMeshRoster,
        space: AssetCategoryScope,
        soldier: Option<&TrooperOutfit>,
        gun_hands: Option<&str>,
        gun_space: AssetCategoryScope,
    ) -> Self {
        let from_kit = soldier.and_then(|kit| kit_arms(loaded, space, kit));
        let from_gun = match gun_hands {
            Some(label) if gun_space == space && loaded.holds(space, label) => Some(label),
            _ => None,
        };
        let stand_in = || default_arms(loaded, space);
        match (from_kit, from_gun) {
            (Some(label), _) => FirstPersonArms::FromKit { namespace: space, name: label },
            (None, Some(own)) => Self::DerivedFromWeaponSpec { namespace: space, name: own.to_owned() },
            (None, None) => match stand_in() {
                Some(label) => Self::StockDefault { namespace: space, name: label },
                None => Self::NothingLoaded,
            },
        }
    }
}

/// The kit's arms if it names some the catalog holds, else the pair that goes
/// with its body.
fn kit_arms(loaded: &FirstPersonMeshRoster, space: AssetCategoryScope, kit: &TrooperOutfit) -> Option<String> {
    if let Some(label) = kit.limbs.as_deref().filter(|label| loaded.holds(space, label)) {
        return Some(label.to_owned());
    }
    let offered: Vec<&str> = loaded.labels_in(space).filter(|label| is_arms_model(label)).collect();
    arms_matching_body(&kit.torso, &offered).filter(|label| loaded.holds(space, label))
}

/// The stand-in arms: the game's own if loaded; failing that, on a
/// multiplayer map, whichever faction arms sort first (until a class script
/// picks a side, none is more right than another).
fn default_arms(loaded: &FirstPersonMeshRoster, space: AssetCategoryScope) -> Option<String> {
    if space != AssetCategoryScope::T6 {
        return None;
    }
    if loaded.holds(space, DEFAULT_ARMS) {
        return Some(DEFAULT_ARMS.to_owned());
    }
    let first_faction = loaded.labels_in(space).filter(|label| label.ends_with("_viewhands")).min();
    // bo2zm: a zone that carries only a Zombies Declassified-style pair draws its players'
    // `viewmodel_*_arms`.
    let first_any = || loaded.labels_in(space).filter(|label| is_arms_model(label)).min();
    first_faction.or_else(first_any).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FpvMeshBuild;
    use crate::fpv_mesh_catalog::test_support::bone_rig;

    const T6: AssetCategoryScope = AssetCategoryScope::T6;

    fn catalog_of(labels: &[&str]) -> FirstPersonMeshRoster {
        let mut build = FpvMeshBuild::default();
        for label in labels {
            build.put_into_namespace(T6, bone_rig(label, &["tag_origin"], true), None);
        }
        build.publish()
    }

    fn picked(hands: &FirstPersonArms) -> Option<&str> {
        hands.key().map(|(_, name)| name)
    }

    #[test]
    fn the_weapons_hands_win_over_the_default() {
        let catalog = catalog_of(&[DEFAULT_ARMS, "c_usa_mp_seal6_viewhands"]);
        let hands = FirstPersonArms::choose(&catalog, T6, None, Some("c_usa_mp_seal6_viewhands"), T6);
        assert!(matches!(hands, FirstPersonArms::DerivedFromWeaponSpec { .. }));
        assert_eq!(picked(&hands), Some("c_usa_mp_seal6_viewhands"));
    }

    #[test]
    fn a_weapon_from_another_namespace_is_ignored() {
        let catalog = catalog_of(&[DEFAULT_ARMS, "other_viewhands"]);
        let hands = FirstPersonArms::choose(&catalog, T6, None, Some("other_viewhands"), AssetCategoryScope::T5);
        assert!(matches!(hands, FirstPersonArms::StockDefault { .. }));
        assert_eq!(picked(&hands), Some(DEFAULT_ARMS));
    }

    #[test]
    fn without_the_default_the_first_faction_arms_sort_in() {
        let catalog = catalog_of(&["c_usa_mp_seal6_viewhands", "c_rus_mp_spetsnaz_viewhands"]);
        let hands = FirstPersonArms::choose(&catalog, T6, None, None, T6);
        assert_eq!(picked(&hands), Some("c_rus_mp_spetsnaz_viewhands"));
    }

    #[test]
    fn no_arms_loaded_is_unresolved() {
        let hands = FirstPersonArms::choose(&catalog_of(&["viewmodel_gun"]), T6, None, None, T6);
        assert_eq!(hands, FirstPersonArms::NothingLoaded);
        assert_eq!(hands.key(), None);
    }

    #[test]
    fn a_kit_naming_loaded_arms_wins() {
        let catalog = catalog_of(&["viewmodel_marines_arms", DEFAULT_ARMS]);
        let kit = TrooperOutfit {
            torso: "body".into(),
            skull: None,
            limbs: Some("viewmodel_marines_arms".into()),
        };
        let hands = FirstPersonArms::choose(&catalog, T6, Some(&kit), None, T6);
        assert!(matches!(hands, FirstPersonArms::FromKit { .. }));
        assert_eq!(picked(&hands), Some("viewmodel_marines_arms"));
    }
}
