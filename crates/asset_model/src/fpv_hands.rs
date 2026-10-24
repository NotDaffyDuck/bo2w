//! Which arms model a first-person weapon wears.

use asset_core::AssetNamespace;

use crate::{FpvMeshCatalog, SoldierKit, arms_for_body, is_arms_model};

/// The arms Black Ops II gives the first player (the map script's
/// `setviewmodel`); on Nuketown that is CIA, in a suit.
const DEFAULT_ARMS: &str = "c_zom_suit_viewhands";

/// The arms chosen for a weapon, and what chose them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FpvHands {
    /// The soldier kit names them, or they match the kit's body.
    FromKit {
        namespace: AssetNamespace,
        name: String,
    },
    /// The weapon's own definition names them.
    FromWeaponDef {
        namespace: AssetNamespace,
        name: String,
    },
    /// Neither said, so these are the map's stand-in.
    GameDefault {
        namespace: AssetNamespace,
        name: String,
    },
    /// No arms are loaded.
    Unresolved,
}

impl FpvHands {
    pub fn key(&self) -> Option<(AssetNamespace, &str)> {
        match self {
            Self::FromKit { namespace, name }
            | Self::FromWeaponDef { namespace, name }
            | Self::GameDefault { namespace, name } => Some((*namespace, name)),
            Self::Unresolved => None,
        }
    }

    /// Picks arms from what the catalog holds for the map's namespace: the
    /// kit's, then the weapon's, then the default.
    pub fn resolve(
        catalog: &FpvMeshCatalog,
        map_ns: AssetNamespace,
        kit: Option<&SoldierKit>,
        weapon_hand: Option<&str>,
        weapon_ns: AssetNamespace,
    ) -> Self {
        if let Some(name) = kit.and_then(|kit| kit_arms(catalog, map_ns, kit)) {
            return Self::FromKit {
                namespace: map_ns,
                name,
            };
        }
        let weapon_arms = weapon_hand
            .filter(|name| weapon_ns == map_ns && catalog.contains(map_ns, name));
        if let Some(name) = weapon_arms {
            return Self::FromWeaponDef {
                namespace: map_ns,
                name: name.to_owned(),
            };
        }
        match default_arms(catalog, map_ns) {
            Some(name) => Self::GameDefault {
                namespace: map_ns,
                name,
            },
            None => Self::Unresolved,
        }
    }
}

/// The kit's arms if it names some the catalog holds, else the pair that goes
/// with its body.
fn kit_arms(catalog: &FpvMeshCatalog, map_ns: AssetNamespace, kit: &SoldierKit) -> Option<String> {
    let named = kit
        .arms
        .as_deref()
        .filter(|name| catalog.contains(map_ns, name));
    if let Some(name) = named {
        return Some(name.to_owned());
    }
    let candidates: Vec<&str> = catalog
        .names_in(map_ns)
        .filter(|name| is_arms_model(name))
        .collect();
    arms_for_body(&kit.body, &candidates).filter(|name| catalog.contains(map_ns, name))
}

/// The stand-in arms: the game's own if loaded; failing that, on a
/// multiplayer map, whichever faction arms sort first (until a class script
/// picks a side, none is more right than another).
fn default_arms(catalog: &FpvMeshCatalog, map_ns: AssetNamespace) -> Option<String> {
    if map_ns != AssetNamespace::T6 {
        return None;
    }
    if catalog.contains(map_ns, DEFAULT_ARMS) {
        return Some(DEFAULT_ARMS.to_owned());
    }
    catalog
        .names_in(map_ns)
        .filter(|name| name.ends_with("_viewhands"))
        .min()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FpvMeshBuild;
    use crate::fpv_mesh_catalog::test_support::skel;

    const T6: AssetNamespace = AssetNamespace::T6;

    fn catalog_of(names: &[&str]) -> FpvMeshCatalog {
        let mut build = FpvMeshBuild::default();
        for name in names {
            build.insert_in(T6, skel(name, &["tag_origin"], true), None);
        }
        build.publish()
    }

    fn picked(hands: &FpvHands) -> Option<&str> {
        hands.key().map(|(_, name)| name)
    }

    #[test]
    fn the_weapons_hands_win_over_the_default() {
        let catalog = catalog_of(&[DEFAULT_ARMS, "c_usa_mp_seal6_viewhands"]);
        let hands = FpvHands::resolve(&catalog, T6, None, Some("c_usa_mp_seal6_viewhands"), T6);
        assert!(matches!(hands, FpvHands::FromWeaponDef { .. }));
        assert_eq!(picked(&hands), Some("c_usa_mp_seal6_viewhands"));
    }

    #[test]
    fn a_weapon_from_another_namespace_is_ignored() {
        let catalog = catalog_of(&[DEFAULT_ARMS, "other_viewhands"]);
        let hands = FpvHands::resolve(&catalog, T6, None, Some("other_viewhands"), AssetNamespace::T5);
        assert!(matches!(hands, FpvHands::GameDefault { .. }));
        assert_eq!(picked(&hands), Some(DEFAULT_ARMS));
    }

    #[test]
    fn without_the_default_the_first_faction_arms_sort_in() {
        let catalog = catalog_of(&["c_usa_mp_seal6_viewhands", "c_rus_mp_spetsnaz_viewhands"]);
        let hands = FpvHands::resolve(&catalog, T6, None, None, T6);
        assert_eq!(picked(&hands), Some("c_rus_mp_spetsnaz_viewhands"));
    }

    #[test]
    fn no_arms_loaded_is_unresolved() {
        let hands = FpvHands::resolve(&catalog_of(&["viewmodel_gun"]), T6, None, None, T6);
        assert_eq!(hands, FpvHands::Unresolved);
        assert_eq!(hands.key(), None);
    }

    #[test]
    fn a_kit_naming_loaded_arms_wins() {
        let catalog = catalog_of(&["viewmodel_marines_arms", DEFAULT_ARMS]);
        let kit = SoldierKit {
            body: "body".into(),
            head: None,
            arms: Some("viewmodel_marines_arms".into()),
        };
        let hands = FpvHands::resolve(&catalog, T6, Some(&kit), None, T6);
        assert!(matches!(hands, FpvHands::FromKit { .. }));
        assert_eq!(picked(&hands), Some("viewmodel_marines_arms"));
    }
}
