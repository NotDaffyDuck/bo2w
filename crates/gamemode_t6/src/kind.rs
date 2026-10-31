use crate::{Team, dem, dm, domination, tdm};

/// The BO2 game types we run (gm-6). Wire tags 0-6 are unchanged from the old crate; 7 was
/// Sabotage, which BO2 does not have, and is Hardpoint now.
///
/// The wire tag of a mode is its position in this list, so new modes only ever go at the end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RulesetKind {
    #[default]
    FreeForAll,
    Demolition,
    Domination,
    TeamDeathmatch,
    SearchAndDestroy,
    CaptureTheFlag,
    Headquarters,
    Hardpoint,
}

/// Every mode, indexed by its wire tag.
const BY_TAG: [RulesetKind; 8] = [
    RulesetKind::FreeForAll,
    RulesetKind::Demolition,
    RulesetKind::Domination,
    RulesetKind::TeamDeathmatch,
    RulesetKind::SearchAndDestroy,
    RulesetKind::CaptureTheFlag,
    RulesetKind::Headquarters,
    RulesetKind::Hardpoint,
];

/// Menu names, indexed by wire tag. TODO(fact): the BO2 localized names (MPUI_*).
const MENU_NAMES: [&str; 8] = [
    dm::MODE_TITLE,
    dem::MODE_TITLE,
    domination::MODE_TITLE,
    tdm::MODE_TITLE,
    "SEARCH AND DESTROY",
    "CAPTURE THE FLAG",
    "HEADQUARTERS",
    "HARDPOINT",
];

/// Grid spawn classnames of domination; the only grid it has.
const DOM_GRID: &[&str] = &[domination::SPAWN_POINT_CLASS];

impl RulesetKind {
    /// BO2 tokens first (gm-6); the MW2 names war, dd and ffa stay accepted for old command lines.
    pub fn parse(words: &str) -> Option<Self> {
        let kind = match words.trim() {
            "dm" | "ffa" | "freeforall" | "free_for_all" | "deathmatch" => Self::FreeForAll,
            "dem" | "dd" | "demolition" => Self::Demolition,
            "dom" | "domination" => Self::Domination,
            "tdm" | "war" | "teamdeathmatch" | "team_deathmatch" => Self::TeamDeathmatch,
            "sd" | "searchanddestroy" | "search_and_destroy" => Self::SearchAndDestroy,
            "ctf" | "capturetheflag" | "capture_the_flag" => Self::CaptureTheFlag,
            "hq" | "headquarters" => Self::Headquarters,
            "koth" | "hardpoint" => Self::Hardpoint,
            _ => return None,
        };
        Some(kind)
    }

    /// Like [`Self::parse`] but accepts any ASCII letter case. Names longer than 32 bytes
    /// are never a mode, so they are refused without being copied.
    pub fn parse_ascii_ignore_case(words: &str) -> Option<Self> {
        let name = words.trim().as_bytes();
        let mut lowered = [0u8; 32];
        let room = lowered.get_mut(..name.len())?;
        room.copy_from_slice(name);
        room.make_ascii_lowercase();
        Self::parse(core::str::from_utf8(room).ok()?)
    }

    /// A BO2 game type token, with the ones we have no ruleset of our own for mapped to the
    /// ruleset that carries their spawns and teams (their own gsc runs in the T6 path):
    /// - conf.gsc sets `level.teambased` and spawns at `gettdmstartspawnname` / `mp_tdm_spawn`:
    ///   team deathmatch.
    /// - gun, oic, shrp and sas.gsc never set `level.teambased`, spawn at `mp_dm_spawn` and
    ///   their gamesettings say `teamCount 1`: free for all.
    /// - oneflag.gsc sets `level.teambased` and spawns at `mp_ctf_spawn_allies` / `_axis`:
    ///   capture the flag.
    ///
    /// [`Self::parse`] stays the menu's list.
    pub fn for_t6_token(words: &str) -> Option<Self> {
        const T6_ONLY: [(&str, RulesetKind); 8] = [
            ("conf", RulesetKind::TeamDeathmatch),
            ("kc", RulesetKind::TeamDeathmatch),
            ("confirmed", RulesetKind::TeamDeathmatch),
            ("gun", RulesetKind::FreeForAll),
            ("oic", RulesetKind::FreeForAll),
            ("shrp", RulesetKind::FreeForAll),
            ("sas", RulesetKind::FreeForAll),
            ("oneflag", RulesetKind::CaptureTheFlag),
        ];
        let word = words.trim();
        Self::parse_ascii_ignore_case(words)
            .or_else(|| T6_ONLY.iter().find(|(w, _)| w.eq_ignore_ascii_case(word)).map(|&(_, kind)| kind))
    }

    /// gm-6: the BO2 gametype token.
    pub fn token(self) -> &'static str {
        self.script_tokens()[0]
    }

    /// The gametype script names to look for, BO2's first. The later names are the MW2 scripts
    /// that still stand in where a BO2 one is missing.
    pub fn script_tokens(self) -> &'static [&'static str] {
        match self {
            Self::FreeForAll => &[dm::MODE_TOKEN],
            Self::Demolition => &[dem::MODE_TOKEN, "dd"],
            Self::Domination => &[domination::MODE_TOKEN],
            Self::TeamDeathmatch => &[tdm::MODE_TOKEN, "war"],
            Self::SearchAndDestroy => &["sd"],
            Self::CaptureTheFlag => &["ctf"],
            Self::Headquarters => &["hq", "koth"],
            Self::Hardpoint => &["koth"],
        }
    }

    pub fn net_code(self) -> u8 {
        self as u8
    }

    pub fn from_net_code(tag: u8) -> Option<Self> {
        BY_TAG.get(usize::from(tag)).copied()
    }

    pub fn shown_label(self) -> &'static str {
        MENU_NAMES[self as usize]
    }

    pub fn has_teams(self) -> bool {
        self != Self::FreeForAll
    }

    /// gm-13: the score limit we start with. TODO(fact): BO2 reads it from playlist data.
    pub fn default_score_limit(self) -> i32 {
        match self {
            Self::Domination => domination::POINTS_TO_WIN,
            Self::Demolition => 0,
            _ => dm::POINTS_TO_WIN,
        }
    }

    /// gm-13: the time limit we start with. TODO(fact): BO2 reads it from playlist data.
    pub fn default_time_limit_ms(self) -> u32 {
        match self {
            Self::Domination => domination::TIME_LIMIT_MS,
            Self::Demolition => dem::TIME_LIMIT_MS,
            _ => dm::TIME_LIMIT_MS,
        }
    }

    /// gm-7, gm-12: whether the match opens on start spawns.
    pub fn uses_start_spawns(self) -> bool {
        match self {
            Self::FreeForAll => dm::USE_START_SPAWNS,
            _ => crate::OPENING_USES_START_SPAWNS,
        }
    }

    /// The start spawn classname for `team` (gm-8 to gm-11); None where the mode has none or
    /// the team is not allies or axis.
    pub fn side_spawn_classname(self, team: Team) -> Option<&'static str> {
        let is_allies = match team {
            Team::Allies => true,
            Team::Axis => false,
            _ => return None,
        };
        // Each mode names its two start spawns; which one `team` gets depends on the mode.
        let (first, second) = match self {
            Self::FreeForAll | Self::SearchAndDestroy => return None,
            Self::TeamDeathmatch | Self::Headquarters | Self::Hardpoint => {
                (tdm::OPENING_SPAWN_ALLIES, tdm::OPENING_SPAWN_AXIS)
            }
            Self::Domination => (domination::OPENING_SPAWN_ALLIES, domination::OPENING_SPAWN_AXIS),
            Self::Demolition => (dem::START_SPAWN_ATTACKER, dem::START_SPAWN_DEFENDER),
            Self::CaptureTheFlag => ("mp_ctf_spawn_allies_start", "mp_ctf_spawn_axis_start"),
        };
        let takes_first = match self {
            Self::Demolition => team == crate::ATTACKERS,
            _ => is_allies,
        };
        Some(if takes_first { first } else { second })
    }

    /// The spawn grid classnames for `team` once start spawns are over (gm-7 to gm-11).
    pub fn side_grid_classnames(self, team: Team) -> &'static [&'static str] {
        let is_allies = match team {
            Team::Allies => true,
            Team::Axis => false,
            _ => return &[],
        };
        let is_attacker = team == crate::ATTACKERS;
        match self {
            Self::FreeForAll => &[dm::SPAWN_POINT_CLASS],
            Self::TeamDeathmatch | Self::Headquarters => &[tdm::SPAWN_POINT_CLASS],
            Self::Hardpoint => &[tdm::SPAWN_POINT_CLASS, tdm::MULTI_TEAM_SPAWN_CLASSNAME],
            Self::Domination => DOM_GRID,
            Self::Demolition if is_attacker => &[dem::SPAWN_ATTACKER],
            Self::Demolition => &[dem::SPAWN_DEFENDER, dem::SPAWN_DEFENDER_A, dem::SPAWN_DEFENDER_B],
            Self::SearchAndDestroy if is_attacker => &["mp_sd_spawn_attacker"],
            Self::SearchAndDestroy => &["mp_sd_spawn_defender"],
            Self::CaptureTheFlag if is_allies => &["mp_ctf_spawn_allies"],
            Self::CaptureTheFlag => &["mp_ctf_spawn_axis"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RulesetKind;

    #[test]
    fn kill_confirmed_runs_as_team_deathmatch_rules() {
        assert_eq!(RulesetKind::for_t6_token("conf"), Some(RulesetKind::TeamDeathmatch));
        assert_eq!(RulesetKind::for_t6_token(" CONF "), Some(RulesetKind::TeamDeathmatch));
        assert_eq!(RulesetKind::for_t6_token("dom"), Some(RulesetKind::Domination));
        for party in ["gun", "oic", "shrp", "sas"] {
            assert_eq!(RulesetKind::for_t6_token(party), Some(RulesetKind::FreeForAll), "{party}");
        }
        assert_eq!(RulesetKind::for_t6_token("oneflag"), Some(RulesetKind::CaptureTheFlag));
        // The menu's list is unchanged.
        assert_eq!(RulesetKind::parse("conf"), None);
        assert_eq!(RulesetKind::for_t6_token("nonsense"), None);
    }
}
