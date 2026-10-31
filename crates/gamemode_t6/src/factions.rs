use crate::Team;

/// One BO2 teamset (`_teamset_<name>.gsc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Faction {
    pub name: &'static str,
    pub team_prefix: &'static str,
    pub team_name: &'static str,
    pub team_color: &'static str,
    pub faction_name: &'static str,
    pub voice: &'static str,
}

/// gm-3.
pub const SEALS: Faction = Faction {
    name: "seals",
    team_prefix: "vox_st",
    team_name: "MPUI_SEALS_SHORT",
    team_color: "0.6 0.64 0.69",
    faction_name: "usa_seals",
    voice: "vox_st6_",
};

/// gm-4.
pub const PMC: Faction = Faction {
    name: "pmc",
    team_prefix: "vox_pm",
    team_name: "MPUI_PMC_SHORT",
    team_color: "0.65 0.57 0.41",
    faction_name: "rus_pmc",
    voice: "vox_pmc_",
};

/// gm-2.
pub const DEFAULT_ALLIES_FACTION: Faction = SEALS;

pub const DEFAULT_AXIS_FACTION: Faction = PMC;

pub const ATTACKERS: Team = Team::Allies;

pub const DEFENDERS: Team = Team::Axis;
