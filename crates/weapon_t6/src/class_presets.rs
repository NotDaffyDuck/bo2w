//! BO2's ten stock classes: the five preset classes, then the five a reset
//! custom class holds. Their perks are not here: the stats table's "default
//! classes" column names them, and the session fills them in from there.

use frame::KitTemplate;

/// A loadout slot with nothing fitted.
const NO_ATTACHMENTS: &[&str] = &[];

/// A class with a primary, a secondary and the shared equipment of a stock
/// custom class: `frag_grenade` and `concussion_grenade`.
const fn custom(
    name: &'static str,
    primary: &'static str,
    secondary: &'static str,
) -> KitTemplate {
    KitTemplate {
        name,
        primary,
        main_gun_parts: &[],
        secondary,
        side_gun_parts: NO_ATTACHMENTS,
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [""; 3],
    }
}

pub static CLASSES: [KitTemplate; 10] = [
    KitTemplate {
        name: "class_smg",
        primary: "t6:weapon/evoskorpion_mp",
        main_gun_parts: &["extclip", "grip"],
        secondary: "t6:weapon/beretta93r_mp",
        side_gun_parts: &["steadyaim"],
        lethal: "t6:weapon/satchel_charge_mp",
        tactical: "t6:weapon/willy_pete_mp",
        perks: [""; 3],
    },
    KitTemplate {
        name: "class_cqb",
        primary: "t6:weapon/srm1216_mp",
        main_gun_parts: &["steadyaim"],
        secondary: "t6:weapon/crossbow_mp",
        side_gun_parts: NO_ATTACHMENTS,
        lethal: "t6:weapon/hatchet_mp",
        tactical: "t6:weapon/tactical_insertion_mp",
        perks: [""; 3],
    },
    KitTemplate {
        name: "class_assault",
        primary: "t6:weapon/hk416_mp",
        main_gun_parts: &["reflex", "dualclip"],
        secondary: "t6:weapon/fhj18_mp",
        side_gun_parts: NO_ATTACHMENTS,
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/proximity_grenade_mp",
        perks: [""; 3],
    },
    KitTemplate {
        name: "class_lmg",
        primary: "t6:weapon/lsat_mp",
        main_gun_parts: &["rangefinder", "fastads", "fmj"],
        secondary: "",
        side_gun_parts: NO_ATTACHMENTS,
        lethal: "t6:weapon/sticky_grenade_mp",
        tactical: "t6:weapon/trophy_system_mp",
        perks: [""; 3],
    },
    KitTemplate {
        name: "class_sniper",
        primary: "t6:weapon/as50_mp",
        main_gun_parts: &["swayreduc", "vzoom"],
        secondary: "t6:weapon/kard_mp",
        side_gun_parts: &["dw"],
        lethal: "t6:weapon/bouncingbetty_mp",
        tactical: "t6:weapon/sensor_grenade_mp",
        perks: [""; 3],
    },
    custom(
        "class_custom_smg",
        "t6:weapon/mp7_mp",
        "t6:weapon/fiveseven_mp",
    ),
    custom(
        "class_custom_cqb",
        "t6:weapon/870mcs_mp",
        "t6:weapon/smaw_mp",
    ),
    custom(
        "class_custom_assault",
        "t6:weapon/tar21_mp",
        "t6:weapon/smaw_mp",
    ),
    custom(
        "class_custom_lmg",
        "t6:weapon/mk48_mp",
        "t6:weapon/smaw_mp",
    ),
    custom(
        "class_custom_sniper",
        "t6:weapon/svu_mp",
        "t6:weapon/fiveseven_mp",
    ),
];

/// Makes these the stock loadouts the menus, the console and the match
/// setup offer.
pub fn register() {
    frame::register_showcase(&CLASSES);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn there_are_ten_stock_classes_with_distinct_names() {
        assert_eq!(CLASSES.len(), 10);
        let unique: HashSet<&str> = CLASSES.iter().map(|class| class.name).collect();
        assert_eq!(unique.len(), 10);
    }

    #[test]
    fn the_five_presets_come_before_the_five_custom_classes() {
        assert!(CLASSES[..5].iter().all(|c| !c.name.contains("custom")));
        assert!(
            CLASSES[5..]
                .iter()
                .all(|c| c.name.starts_with("class_custom_"))
        );
    }

    #[test]
    fn every_class_has_a_primary_and_equipment_and_leaves_its_perks_to_the_stats_table() {
        for class in &CLASSES {
            assert!(!class.primary.is_empty(), "{}", class.name);
            assert!(!class.lethal.is_empty() && !class.tactical.is_empty());
            assert!(class.perks.iter().all(|p| p.is_empty()), "{}", class.name);
        }
    }

    #[test]
    fn every_weapon_is_a_t6_catalog_key() {
        for class in &CLASSES {
            for key in [class.primary, class.secondary, class.lethal, class.tactical] {
                assert!(
                    key.is_empty() || (key.starts_with("t6:weapon/") && key.ends_with("_mp")),
                    "{}: {key}",
                    class.name
                );
            }
        }
    }

    #[test]
    fn only_the_lmg_preset_carries_no_secondary() {
        let bare: Vec<&str> = CLASSES
            .iter()
            .filter(|class| class.secondary.is_empty())
            .map(|class| class.name)
            .collect();
        assert_eq!(bare, ["class_lmg"]);
    }

    #[test]
    fn the_custom_classes_share_their_equipment() {
        for class in &CLASSES[5..] {
            assert_eq!(class.lethal, "t6:weapon/frag_grenade_mp", "{}", class.name);
            assert_eq!(class.tactical, "t6:weapon/concussion_grenade_mp");
            assert!(class.main_gun_parts.is_empty());
            assert!(class.side_gun_parts.is_empty());
        }
    }

    #[test]
    fn the_smg_preset_carries_its_attachments() {
        let smg = &CLASSES[0];
        assert_eq!(smg.name, "class_smg");
        assert_eq!(smg.main_gun_parts, ["extclip", "grip"]);
        assert_eq!(smg.side_gun_parts, ["steadyaim"]);
    }

    /// The only test that writes the process-wide table, so the others in
    /// this crate cannot race it.
    #[test]
    fn registering_makes_the_bo2_classes_the_stock_ones() {
        register();
        assert_eq!(frame::showcase_classes(), &CLASSES[..]);
        assert_eq!(frame::pick_showcase(11, 10).len(), 10);
        assert_eq!(frame::ServerKitRoster::default().slots.len(), 5);
    }
}
