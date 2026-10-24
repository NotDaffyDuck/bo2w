//! BO2's ten stock classes: the five preset classes, then the five a reset
//! custom class holds. The perks and deathstreak come from the legacy menu
//! catalog and are unverified.

use frame::ClassPreset;

/// A class with a primary, a secondary and the shared equipment of a stock
/// custom class: `frag_grenade` and `concussion_grenade`.
const fn custom(
    name: &'static str,
    primary: &'static str,
    secondary: &'static str,
    perks: [&'static str; 3],
    deathstreak: &'static str,
) -> ClassPreset {
    ClassPreset {
        name,
        primary,
        primary_attachments: &[],
        secondary,
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks,
        deathstreak,
    }
}

pub static CLASSES: [ClassPreset; 10] = [
    ClassPreset {
        name: "class_smg",
        primary: "t6:weapon/evoskorpion_mp",
        primary_attachments: &["extclip", "grip"],
        secondary: "t6:weapon/beretta93r_mp",
        secondary_attachments: &["steadyaim"],
        lethal: "t6:weapon/satchel_charge_mp",
        tactical: "t6:weapon/willy_pete_mp",
        perks: [
            "specialty_marathon",
            "specialty_lightweight",
            "specialty_extendedmelee",
        ],
        deathstreak: "specialty_finalstand",
    },
    ClassPreset {
        name: "class_cqb",
        primary: "t6:weapon/srm1216_mp",
        primary_attachments: &["steadyaim"],
        secondary: "t6:weapon/crossbow_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/hatchet_mp",
        tactical: "t6:weapon/tactical_insertion_mp",
        perks: [
            "specialty_marathon",
            "specialty_coldblooded",
            "specialty_extendedmelee",
        ],
        deathstreak: "specialty_combathigh",
    },
    ClassPreset {
        name: "class_assault",
        primary: "t6:weapon/hk416_mp",
        primary_attachments: &["reflex", "dualclip"],
        secondary: "t6:weapon/fhj18_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/proximity_grenade_mp",
        perks: [
            "specialty_fastreload",
            "specialty_bulletdamage",
            "specialty_bulletaccuracy",
        ],
        deathstreak: "specialty_combathigh",
    },
    ClassPreset {
        name: "class_lmg",
        primary: "t6:weapon/lsat_mp",
        primary_attachments: &["rangefinder", "fastads", "fmj"],
        secondary: "",
        secondary_attachments: &[],
        lethal: "t6:weapon/sticky_grenade_mp",
        tactical: "t6:weapon/trophy_system_mp",
        perks: [
            "specialty_bling",
            "specialty_bulletdamage",
            "specialty_detectexplosive",
        ],
        deathstreak: "specialty_copycat",
    },
    ClassPreset {
        name: "class_sniper",
        primary: "t6:weapon/as50_mp",
        primary_attachments: &["swayreduc", "vzoom"],
        secondary: "t6:weapon/kard_mp",
        secondary_attachments: &["dw"],
        lethal: "t6:weapon/bouncingbetty_mp",
        tactical: "t6:weapon/sensor_grenade_mp",
        perks: [
            "specialty_bling",
            "specialty_coldblooded",
            "specialty_localjammer",
        ],
        deathstreak: "specialty_copycat",
    },
    custom(
        "class_custom_smg",
        "t6:weapon/mp7_mp",
        "t6:weapon/fiveseven_mp",
        [
            "specialty_marathon",
            "specialty_coldblooded",
            "specialty_heartbreaker",
        ],
        "specialty_finalstand",
    ),
    custom(
        "class_custom_cqb",
        "t6:weapon/870mcs_mp",
        "t6:weapon/smaw_mp",
        [
            "specialty_marathon",
            "specialty_lightweight",
            "specialty_extendedmelee",
        ],
        "specialty_grenadepulldeath",
    ),
    custom(
        "class_custom_assault",
        "t6:weapon/tar21_mp",
        "t6:weapon/smaw_mp",
        [
            "specialty_fastreload",
            "specialty_lightweight",
            "specialty_bulletaccuracy",
        ],
        "specialty_copycat",
    ),
    custom(
        "class_custom_lmg",
        "t6:weapon/mk48_mp",
        "t6:weapon/smaw_mp",
        [
            "specialty_scavenger",
            "specialty_hardline",
            "specialty_bulletaccuracy",
        ],
        "specialty_copycat",
    ),
    custom(
        "class_custom_sniper",
        "t6:weapon/svu_mp",
        "t6:weapon/fiveseven_mp",
        [
            "specialty_marathon",
            "specialty_coldblooded",
            "specialty_heartbreaker",
        ],
        "specialty_finalstand",
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
    fn every_class_has_a_primary_and_the_three_perk_slots_filled() {
        for class in &CLASSES {
            assert!(!class.primary.is_empty(), "{}", class.name);
            assert!(!class.lethal.is_empty() && !class.tactical.is_empty());
            assert!(class.perks.iter().all(|p| !p.is_empty()), "{}", class.name);
            assert!(!class.deathstreak.is_empty(), "{}", class.name);
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
            assert!(class.primary_attachments.is_empty());
            assert!(class.secondary_attachments.is_empty());
        }
    }

    #[test]
    fn the_smg_preset_carries_its_attachments() {
        let smg = &CLASSES[0];
        assert_eq!(smg.name, "class_smg");
        assert_eq!(smg.primary_attachments, ["extclip", "grip"]);
        assert_eq!(smg.secondary_attachments, ["steadyaim"]);
    }

    /// The only test that writes the process-wide table, so the others in
    /// this crate cannot race it.
    #[test]
    fn registering_makes_the_bo2_classes_the_stock_ones() {
        register();
        assert_eq!(frame::showcase_classes(), &CLASSES[..]);
        assert_eq!(frame::pick_showcase(11, 10).len(), 10);
        assert_eq!(frame::HostClassLoadouts::default().slots.len(), 5);
    }
}
