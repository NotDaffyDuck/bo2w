/// The weapon groups a class picker lists, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacAuthoredCategory {
    Assault,
    Smg,
    Lmg,
    Sniper,
    Riot,
    MachinePistol,
    Shotgun,
    Pistol,
    Projectile,
    Cqb,
    Special,
}

impl CacAuthoredCategory {
    /// The heading to show when the localized one is missing.
    #[must_use]
    pub fn menu_label(self) -> &'static str {
        match self {
            Self::Assault => "ASSAULT RIFLES",
            Self::Smg => "SUB MACHINE GUNS",
            Self::Lmg => "LIGHT MACHINE GUNS",
            Self::Sniper => "SNIPER RIFLES",
            Self::Riot => "RIOT SHIELD",
            Self::MachinePistol => "MACHINE PISTOLS",
            Self::Shotgun => "SHOTGUNS",
            Self::Pistol => "HANDGUNS",
            Self::Projectile => "ROCKETS",
            Self::Cqb => "CQB",
            Self::Special => "SPECIAL WEAPONS",
        }
    }

    /// The localized heading's key; the last two groups have none.
    #[must_use]
    pub fn loc_key(self) -> Option<&'static str> {
        match self {
            Self::Assault => Some("@MENU_ASSAULT_RIFLES_CAPS"),
            Self::Smg => Some("@MENU_SMGS_CAPS"),
            Self::Lmg => Some("@MENU_LMGS_CAPS"),
            Self::Sniper => Some("@MENU_SNIPER_RIFLES_CAPS"),
            Self::Riot => Some("@MENU_RIOT_SHIELD_CAPS"),
            Self::MachinePistol => Some("@MENU_MACHINE_PISTOLS_CAPS"),
            Self::Shotgun => Some("@MENU_SHOTGUNS_CAPS"),
            Self::Pistol => Some("@MENU_HANDGUNS_CAPS"),
            Self::Projectile => Some("@MENU_ROCKETS_CAPS"),
            Self::Cqb | Self::Special => None,
        }
    }

    /// A short id for picker folders.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Assault => "assault",
            Self::Smg => "smg",
            Self::Lmg => "lmg",
            Self::Sniper => "sniper",
            Self::Shotgun => "shotgun",
            Self::Riot => "riot",
            Self::MachinePistol => "machine_pistol",
            Self::Pistol => "pistol",
            Self::Projectile => "projectile",
            Self::Cqb => "cqb",
            Self::Special => "special",
        }
    }
}

/// The group a stats-table item group (`weapon_smg` and so on) belongs to.
#[must_use]
pub fn cac_category_from_item_group(group: &str) -> Option<CacAuthoredCategory> {
    use CacAuthoredCategory as Cat;
    Some(match group.strip_prefix("weapon_")? {
        "assault" => Cat::Assault,
        "smg" => Cat::Smg,
        "lmg" => Cat::Lmg,
        "sniper" => Cat::Sniper,
        "shotgun" => Cat::Shotgun,
        "riot" => Cat::Riot,
        "machine_pistol" => Cat::MachinePistol,
        "pistol" => Cat::Pistol,
        "projectile" | "launcher" => Cat::Projectile,
        "cqb" => Cat::Cqb,
        "special" => Cat::Special,
        _ => return None,
    })
}
