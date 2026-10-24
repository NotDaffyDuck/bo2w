/// A BO2 weapon group, as mp/statstable.csv names it (column 2;
/// facts/weapons-classes-t6.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UniqueGroup {
    Assault,
    Smg,
    Lmg,
    Sniper,
    Cqb,
    Pistol,
    Launcher,
    Special,
}

impl UniqueGroup {
    #[must_use]
    pub fn item_group(self) -> &'static str {
        match self {
            Self::Assault => "weapon_assault",
            Self::Smg => "weapon_smg",
            Self::Lmg => "weapon_lmg",
            Self::Sniper => "weapon_sniper",
            Self::Cqb => "weapon_cqb",
            Self::Pistol => "weapon_pistol",
            Self::Launcher => "weapon_launcher",
            Self::Special => "weapon_special",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniqueWeapon {
    pub key: &'static str,
    pub group: UniqueGroup,
}

/// BO2's MP weapons by group (the statstable's rows; the _dw pistols share
/// their gun's stem).
pub const UNIQUE_WEAPONS: &[UniqueWeapon] = &[
    UniqueWeapon {
        key: "t6:weapon/xm8_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/scar_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/an94_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/sig556_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/type95_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/sa58_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/saritch_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/hk416_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/tar21_mp",
        group: UniqueGroup::Assault,
    },
    UniqueWeapon {
        key: "t6:weapon/mp7_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/evoskorpion_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/pdw57_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/qcw05_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/insas_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/vector_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/peacekeeper_mp",
        group: UniqueGroup::Smg,
    },
    UniqueWeapon {
        key: "t6:weapon/mk48_mp",
        group: UniqueGroup::Lmg,
    },
    UniqueWeapon {
        key: "t6:weapon/qbb95_mp",
        group: UniqueGroup::Lmg,
    },
    UniqueWeapon {
        key: "t6:weapon/lsat_mp",
        group: UniqueGroup::Lmg,
    },
    UniqueWeapon {
        key: "t6:weapon/hamr_mp",
        group: UniqueGroup::Lmg,
    },
    UniqueWeapon {
        key: "t6:weapon/ballista_mp",
        group: UniqueGroup::Sniper,
    },
    UniqueWeapon {
        key: "t6:weapon/svu_mp",
        group: UniqueGroup::Sniper,
    },
    UniqueWeapon {
        key: "t6:weapon/dsr50_mp",
        group: UniqueGroup::Sniper,
    },
    UniqueWeapon {
        key: "t6:weapon/as50_mp",
        group: UniqueGroup::Sniper,
    },
    UniqueWeapon {
        key: "t6:weapon/870mcs_mp",
        group: UniqueGroup::Cqb,
    },
    UniqueWeapon {
        key: "t6:weapon/srm1216_mp",
        group: UniqueGroup::Cqb,
    },
    UniqueWeapon {
        key: "t6:weapon/saiga12_mp",
        group: UniqueGroup::Cqb,
    },
    UniqueWeapon {
        key: "t6:weapon/ksg_mp",
        group: UniqueGroup::Cqb,
    },
    UniqueWeapon {
        key: "t6:weapon/kard_mp",
        group: UniqueGroup::Pistol,
    },
    UniqueWeapon {
        key: "t6:weapon/fnp45_mp",
        group: UniqueGroup::Pistol,
    },
    UniqueWeapon {
        key: "t6:weapon/beretta93r_mp",
        group: UniqueGroup::Pistol,
    },
    UniqueWeapon {
        key: "t6:weapon/judge_mp",
        group: UniqueGroup::Pistol,
    },
    UniqueWeapon {
        key: "t6:weapon/fiveseven_mp",
        group: UniqueGroup::Pistol,
    },
    UniqueWeapon {
        key: "t6:weapon/smaw_mp",
        group: UniqueGroup::Launcher,
    },
    UniqueWeapon {
        key: "t6:weapon/fhj18_mp",
        group: UniqueGroup::Launcher,
    },
    UniqueWeapon {
        key: "t6:weapon/usrpg_mp",
        group: UniqueGroup::Launcher,
    },
    UniqueWeapon {
        key: "t6:weapon/m32_mp",
        group: UniqueGroup::Launcher,
    },
    UniqueWeapon {
        key: "t6:weapon/riotshield_mp",
        group: UniqueGroup::Special,
    },
    UniqueWeapon {
        key: "t6:weapon/crossbow_mp",
        group: UniqueGroup::Special,
    },
    UniqueWeapon {
        key: "t6:weapon/knife_ballistic_mp",
        group: UniqueGroup::Special,
    },
    UniqueWeapon {
        key: "t6:weapon/minigun_mp",
        group: UniqueGroup::Special,
    },
];

#[must_use]
pub fn family_stem(name: &str) -> String {
    let stem = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit(':')
        .next()
        .unwrap_or(name);
    let stem = stem.split('+').next().unwrap_or(stem);
    let stem = stem.strip_suffix("_mp").unwrap_or(stem);
    stem.strip_suffix("_dw").unwrap_or(stem).to_owned()
}

#[cfg(test)]
mod tests {
    use crate::observation::WeaponClass;

    #[test]
    fn bo2_guns_get_their_group_range() {
        assert_eq!(WeaponClass::from_script_name("dsr50_mp+swayreduc"), WeaponClass::Sniper);
        assert_eq!(WeaponClass::from_script_name("ksg_mp"), WeaponClass::Shotgun);
        assert_eq!(WeaponClass::from_script_name("hamr_mp"), WeaponClass::Lmg);
        assert_eq!(WeaponClass::from_script_name("kard_dw_mp"), WeaponClass::Smg);
        assert_eq!(WeaponClass::from_script_name("an94_mp"), WeaponClass::Assault);
    }
}
