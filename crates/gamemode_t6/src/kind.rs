use crate::{Team, dem, dm, dom, tdm};

/// The BO2 game types we run (gm-6). Wire tags 0-6 are unchanged from the old crate; 7 was
/// Sabotage, which BO2 does not have, and is Hardpoint now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GameModeKind {
    #[default]
    FreeForAll = 0,

    Demolition = 1,

    Domination = 2,

    TeamDeathmatch = 3,

    SearchAndDestroy = 4,

    CaptureTheFlag = 5,

    Headquarters = 6,

    Hardpoint = 7,
}

impl GameModeKind {
    /// BO2 tokens first (gm-6); the MW2 names war, dd and ffa stay accepted for old command lines.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().as_bytes() {
            b"dm" | b"ffa" | b"freeforall" | b"free_for_all" | b"deathmatch" => {
                Some(Self::FreeForAll)
            }

            b"dem" | b"dd" | b"demolition" => Some(Self::Demolition),
            b"dom" | b"domination" => Some(Self::Domination),
            b"tdm" | b"war" | b"teamdeathmatch" | b"team_deathmatch" => Some(Self::TeamDeathmatch),
            b"sd" | b"searchanddestroy" | b"search_and_destroy" => Some(Self::SearchAndDestroy),
            b"ctf" | b"capturetheflag" | b"capture_the_flag" => Some(Self::CaptureTheFlag),
            b"hq" | b"headquarters" => Some(Self::Headquarters),
            b"koth" | b"hardpoint" => Some(Self::Hardpoint),
            _ => None,
        }
    }

    pub fn parse_ascii_ignore_case(s: &str) -> Option<Self> {
        let mut buf = [0u8; 32];
        let bytes = s.trim().as_bytes();
        if bytes.len() > buf.len() {
            return None;
        }
        for (i, &b) in bytes.iter().enumerate() {
            buf[i] = b.to_ascii_lowercase();
        }
        Self::parse(core::str::from_utf8(&buf[..bytes.len()]).ok()?)
    }

    /// gm-6: the BO2 gametype token.
    pub fn token(self) -> &'static str {
        match self {
            Self::FreeForAll => dm::GAMETYPE_TOKEN,
            Self::Demolition => dem::GAMETYPE_TOKEN,
            Self::Domination => dom::GAMETYPE_TOKEN,
            Self::TeamDeathmatch => tdm::GAMETYPE_TOKEN,
            Self::SearchAndDestroy => "sd",
            Self::CaptureTheFlag => "ctf",
            Self::Headquarters => "hq",
            Self::Hardpoint => "koth",
        }
    }

    /// The gametype script names to look for, BO2's first. The later names are the MW2 scripts
    /// that still stand in where a BO2 one is missing.
    pub fn script_tokens(self) -> &'static [&'static str] {
        match self {
            Self::FreeForAll => &[dm::GAMETYPE_TOKEN],
            Self::Demolition => &[dem::GAMETYPE_TOKEN, "dd"],
            Self::Domination => &[dom::GAMETYPE_TOKEN],
            Self::TeamDeathmatch => &[tdm::GAMETYPE_TOKEN, "war"],
            Self::SearchAndDestroy => &["sd"],
            Self::CaptureTheFlag => &["ctf"],
            Self::Headquarters => &["hq", "koth"],
            Self::Hardpoint => &["koth"],
        }
    }

    pub fn wire_tag(self) -> u8 {
        self as u8
    }

    pub fn from_wire_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::FreeForAll),
            1 => Some(Self::Demolition),
            2 => Some(Self::Domination),
            3 => Some(Self::TeamDeathmatch),
            4 => Some(Self::SearchAndDestroy),
            5 => Some(Self::CaptureTheFlag),
            6 => Some(Self::Headquarters),
            7 => Some(Self::Hardpoint),
            _ => None,
        }
    }

    // TODO(fact): the BO2 localized names (MPUI_*) for these.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::FreeForAll => dm::DISPLAY_NAME,
            Self::Demolition => dem::DISPLAY_NAME,
            Self::Domination => dom::DISPLAY_NAME,
            Self::TeamDeathmatch => tdm::DISPLAY_NAME,
            Self::SearchAndDestroy => "SEARCH AND DESTROY",
            Self::CaptureTheFlag => "CAPTURE THE FLAG",
            Self::Headquarters => "HEADQUARTERS",
            Self::Hardpoint => "HARDPOINT",
        }
    }

    pub fn is_team(self) -> bool {
        !matches!(self, Self::FreeForAll)
    }

    /// gm-13: the score limit we start with. TODO(fact): BO2 reads it from playlist data.
    pub fn default_score_limit(self) -> i32 {
        match self {
            Self::Domination => dom::SCORE_LIMIT,
            Self::Demolition => 0,
            _ => dm::SCORE_LIMIT,
        }
    }

    /// gm-13: the time limit we start with. TODO(fact): BO2 reads it from playlist data.
    pub fn default_time_limit_ms(self) -> u32 {
        match self {
            Self::Domination => dom::TIME_LIMIT_MS,
            Self::Demolition => dem::TIME_LIMIT_MS,
            _ => dm::TIME_LIMIT_MS,
        }
    }

    /// gm-7, gm-12: whether the match opens on start spawns.
    pub fn uses_start_spawns(self) -> bool {
        match self {
            Self::FreeForAll => dm::USE_START_SPAWNS,
            _ => crate::USE_START_SPAWNS_AT_START,
        }
    }

    /// The start spawn classname for `team` (gm-8 to gm-11); None where the mode has none or
    /// the team is not allies or axis.
    pub fn team_start_classname(self, team: Team) -> Option<&'static str> {
        let allies = match team {
            Team::Allies => true,
            Team::Axis => false,
            _ => return None,
        };
        let attackers = team == crate::ATTACKERS;
        match self {
            Self::FreeForAll | Self::SearchAndDestroy => None,
            Self::TeamDeathmatch | Self::Headquarters | Self::Hardpoint => Some(if allies {
                tdm::START_SPAWN_ALLIES
            } else {
                tdm::START_SPAWN_AXIS
            }),
            Self::Domination => Some(if allies {
                dom::START_SPAWN_ALLIES
            } else {
                dom::START_SPAWN_AXIS
            }),
            Self::Demolition => Some(if attackers {
                dem::START_SPAWN_ATTACKER
            } else {
                dem::START_SPAWN_DEFENDER
            }),
            Self::CaptureTheFlag => Some(if allies {
                "mp_ctf_spawn_allies_start"
            } else {
                "mp_ctf_spawn_axis_start"
            }),
        }
    }

    /// The spawn grid classnames for `team` once start spawns are over (gm-7 to gm-11).
    pub fn team_grid_classnames(self, team: Team) -> &'static [&'static str] {
        let allies = match team {
            Team::Allies => true,
            Team::Axis => false,
            _ => return &[],
        };
        let attackers = team == crate::ATTACKERS;
        match self {
            Self::FreeForAll => &[dm::SPAWN_CLASSNAME],
            Self::TeamDeathmatch | Self::Headquarters => &[tdm::SPAWN_CLASSNAME],
            Self::Hardpoint => &[tdm::SPAWN_CLASSNAME, tdm::MULTI_TEAM_SPAWN_CLASSNAME],
            Self::Domination => &[dom::SPAWN_CLASSNAME],
            Self::Demolition => {
                if attackers {
                    &[dem::SPAWN_ATTACKER]
                } else {
                    &[dem::SPAWN_DEFENDER, dem::SPAWN_DEFENDER_A, dem::SPAWN_DEFENDER_B]
                }
            }
            Self::SearchAndDestroy => {
                if attackers {
                    &["mp_sd_spawn_attacker"]
                } else {
                    &["mp_sd_spawn_defender"]
                }
            }
            Self::CaptureTheFlag => {
                if allies {
                    &["mp_ctf_spawn_allies"]
                } else {
                    &["mp_ctf_spawn_axis"]
                }
            }
        }
    }
}
