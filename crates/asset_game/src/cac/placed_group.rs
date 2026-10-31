/// The weapon groups a class picker lists, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoadoutPlacedGroup {
    Assault,
    Smg,
    Lmg,
    Sniper,
    Riot,
    AutoSidearm,
    Shotgun,
    Pistol,
    Thrown,
    Cqb,
    Special,
}

impl LoadoutPlacedGroup {
    /// The heading to show when the localized one is missing.
    #[must_use]
    pub fn screen_caption(self) -> &'static str {
        match self {
            LoadoutPlacedGroup::Assault => "ASSAULT RIFLES",
            LoadoutPlacedGroup::Smg => "SUB MACHINE GUNS",
            LoadoutPlacedGroup::Lmg => "LIGHT MACHINE GUNS",
            LoadoutPlacedGroup::Sniper => "SNIPER RIFLES",
            LoadoutPlacedGroup::Riot => "RIOT SHIELD",
            LoadoutPlacedGroup::AutoSidearm => "MACHINE PISTOLS",
            LoadoutPlacedGroup::Shotgun => "SHOTGUNS",
            LoadoutPlacedGroup::Pistol => "HANDGUNS",
            LoadoutPlacedGroup::Thrown => "ROCKETS",
            LoadoutPlacedGroup::Cqb => "CQB",
            LoadoutPlacedGroup::Special => "SPECIAL WEAPONS",
        }
    }

    /// The localized heading's key; the last two groups have none.
    #[must_use]
    pub fn text_lookup_token(self) -> Option<&'static str> {
        match self {
            Self::Assault => Some("@MENU_ASSAULT_RIFLES_CAPS"),
            Self::Smg => Some("@MENU_SMGS_CAPS"),
            Self::Lmg => Some("@MENU_LMGS_CAPS"),
            Self::Sniper => Some("@MENU_SNIPER_RIFLES_CAPS"),
            Self::Riot => Some("@MENU_RIOT_SHIELD_CAPS"),
            Self::AutoSidearm => Some("@MENU_MACHINE_PISTOLS_CAPS"),
            Self::Shotgun => Some("@MENU_SHOTGUNS_CAPS"),
            Self::Pistol => Some("@MENU_HANDGUNS_CAPS"),
            Self::Thrown => Some("@MENU_ROCKETS_CAPS"),
            Self::Cqb | Self::Special => None,
        }
    }

    /// A short id for picker folders.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            LoadoutPlacedGroup::Assault => "assault",
            LoadoutPlacedGroup::Smg => "smg",
            LoadoutPlacedGroup::Lmg => "lmg",
            LoadoutPlacedGroup::Sniper => "sniper",
            LoadoutPlacedGroup::Shotgun => "shotgun",
            LoadoutPlacedGroup::Riot => "riot",
            LoadoutPlacedGroup::AutoSidearm => "machine_pistol",
            LoadoutPlacedGroup::Pistol => "pistol",
            LoadoutPlacedGroup::Thrown => "projectile",
            LoadoutPlacedGroup::Cqb => "cqb",
            LoadoutPlacedGroup::Special => "special",
        }
    }
}

/// The group a stats-table item group (`weapon_smg` and so on) belongs to.
#[must_use]
pub fn loadout_category_of_group(band: &str) -> Option<LoadoutPlacedGroup> {
    Some(match band.strip_prefix("weapon_")? {
        "assault" => LoadoutPlacedGroup::Assault,
        "smg" => LoadoutPlacedGroup::Smg,
        "lmg" => LoadoutPlacedGroup::Lmg,
        "sniper" => LoadoutPlacedGroup::Sniper,
        "shotgun" => LoadoutPlacedGroup::Shotgun,
        "riot" => LoadoutPlacedGroup::Riot,
        "machine_pistol" => LoadoutPlacedGroup::AutoSidearm,
        "pistol" => LoadoutPlacedGroup::Pistol,
        "projectile" | "launcher" => LoadoutPlacedGroup::Thrown,
        "cqb" => LoadoutPlacedGroup::Cqb,
        "special" => LoadoutPlacedGroup::Special,
        _ => return None,
    })
}
