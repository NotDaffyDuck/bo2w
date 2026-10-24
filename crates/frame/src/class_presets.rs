use crate::HostClassSlot;

#[derive(Debug, Clone, Copy)]
pub struct ClassPreset {
    pub name: &'static str,
    pub primary: &'static str,
    pub primary_attachments: &'static [&'static str],
    pub secondary: &'static str,
    pub secondary_attachments: &'static [&'static str],
    pub lethal: &'static str,
    pub tactical: &'static str,
    pub perks: [&'static str; 3],
    pub deathstreak: &'static str,
}

impl From<&ClassPreset> for HostClassSlot {
    fn from(preset: &ClassPreset) -> Self {
        let owned = |list: &[&str]| list.iter().map(|value| (*value).to_owned()).collect();
        Self {
            name: preset.name.to_owned(),
            primary: preset.primary.to_owned(),
            primary_attachments: owned(preset.primary_attachments),
            secondary: preset.secondary.to_owned(),
            secondary_attachments: owned(preset.secondary_attachments),
            lethal: preset.lethal.to_owned(),
            tactical: preset.tactical.to_owned(),
            perks: preset.perks.map(str::to_owned),
            deathstreak: preset.deathstreak.to_owned(),
        }
    }
}

pub fn showcase_classes() -> &'static [ClassPreset] {
    &SHOWCASE
}

/// BO2's ten default classes, as its mp/statstable.csv puts them together
/// (facts/weapons-classes-t6.md): the five preset classes, then the five a
/// reset custom class holds. TODO(fact): the perks and deathstreak are the
/// legacy menu's MW2 catalog, not BO2's.
const SHOWCASE: [ClassPreset; 10] = [
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
    ClassPreset {
        name: "class_custom_smg",
        primary: "t6:weapon/mp7_mp",
        primary_attachments: &[],
        secondary: "t6:weapon/fiveseven_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [
            "specialty_marathon",
            "specialty_coldblooded",
            "specialty_heartbreaker",
        ],
        deathstreak: "specialty_finalstand",
    },
    ClassPreset {
        name: "class_custom_cqb",
        primary: "t6:weapon/870mcs_mp",
        primary_attachments: &[],
        secondary: "t6:weapon/smaw_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [
            "specialty_marathon",
            "specialty_lightweight",
            "specialty_extendedmelee",
        ],
        deathstreak: "specialty_grenadepulldeath",
    },
    ClassPreset {
        name: "class_custom_assault",
        primary: "t6:weapon/tar21_mp",
        primary_attachments: &[],
        secondary: "t6:weapon/smaw_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [
            "specialty_fastreload",
            "specialty_lightweight",
            "specialty_bulletaccuracy",
        ],
        deathstreak: "specialty_copycat",
    },
    ClassPreset {
        name: "class_custom_lmg",
        primary: "t6:weapon/mk48_mp",
        primary_attachments: &[],
        secondary: "t6:weapon/smaw_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [
            "specialty_scavenger",
            "specialty_hardline",
            "specialty_bulletaccuracy",
        ],
        deathstreak: "specialty_copycat",
    },
    ClassPreset {
        name: "class_custom_sniper",
        primary: "t6:weapon/svu_mp",
        primary_attachments: &[],
        secondary: "t6:weapon/fiveseven_mp",
        secondary_attachments: &[],
        lethal: "t6:weapon/frag_grenade_mp",
        tactical: "t6:weapon/concussion_grenade_mp",
        perks: [
            "specialty_marathon",
            "specialty_coldblooded",
            "specialty_heartbreaker",
        ],
        deathstreak: "specialty_finalstand",
    },
];

pub fn pick_showcase(seed: u64, count: usize) -> Vec<&'static ClassPreset> {
    let mut order: Vec<&'static ClassPreset> = SHOWCASE.iter().collect();
    let mut state = seed;
    for i in (1..order.len()).rev() {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        order.swap(i, (z % (i as u64 + 1)) as usize);
    }
    order.truncate(count);
    order
}
