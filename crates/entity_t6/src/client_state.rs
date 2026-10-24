//! BO2's `clientState_s`: the per-player record every client gets in each
//! snapshot (team, model, name, clan tag, perks, score), with BO2's team
//! numbers and its name rules from `ClientCleanName`, `BG_GetTeamName`,
//! `Scr_GetTeam` and `CS_DisplayName`.

use core::fmt;

/// `team_t`. BO2 numbers allies before axis, unlike MW2.
pub mod team {
    pub const FREE: i32 = 0;
    /// `TEAM_BAD` shares 0 with `TEAM_FREE`.
    pub const BAD: i32 = 0;
    pub const ALLIES: i32 = 1;
    pub const AXIS: i32 = 2;
    pub const THREE: i32 = 3;
    pub const FOUR: i32 = 4;
    pub const FIVE: i32 = 5;
    pub const SIX: i32 = 6;
    pub const SEVEN: i32 = 7;
    pub const EIGHT: i32 = 8;
    pub const NUM_PLAYING_TEAMS: i32 = 9;
    pub const SPECTATOR: i32 = 9;
    pub const NUM_TEAMS: i32 = 10;
    pub const LOCALPLAYERS: i32 = 11;
}

/// `ffa_team_t`, the team a free-for-all player is drawn as.
pub mod ffa_team {
    pub const NONE: i32 = 0;
    pub const AXIS: i32 = 1;
    pub const ALLIES: i32 = 2;
    pub const THREE: i32 = 3;
}

/// `VehicleAnimState`.
pub mod veh_anim_state {
    pub const IDLE: i32 = 0;
    pub const ENTRY: i32 = 1;
    pub const CHANGEPOS: i32 = 2;
    pub const EXIT: i32 = 3;
}

/// `s_teamNames`, the table `BG_GetTeamName` reads.
pub const TEAM_NAMES: [&str; team::NUM_TEAMS as usize] = [
    "TEAM_FREE",
    "TEAM_ALLIES",
    "TEAM_AXIS",
    "TEAM_THREE",
    "TEAM_FOUR",
    "TEAM_FIVE",
    "TEAM_SIX",
    "TEAM_SEVEN",
    "TEAM_EIGHT",
    "TEAM_SPECTATOR",
];

/// `BG_GetTeamName`. BO2 asserts `team < TEAM_NUM_TEAMS`; out of range
/// gives `None` here.
#[must_use]
pub fn bg_team_name(team: i32) -> Option<&'static str> {
    usize::try_from(team).ok().and_then(|t| TEAM_NAMES.get(t)).copied()
}

/// `CG_GetFFATeamName`. A free-for-all player with no team draws as the
/// spectator team.
#[must_use]
pub fn ffa_team_name(ffa: i32) -> Option<&'static str> {
    match ffa {
        ffa_team::NONE => Some("TEAM_SPECTATOR"),
        ffa_team::AXIS => Some("TEAM_AXIS"),
        ffa_team::ALLIES => Some("TEAM_ALLIES"),
        ffa_team::THREE => Some("TEAM_THREE"),
        _ => None,
    }
}

/// The script error `Scr_GetTeam` raises for a string it does not know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IllegalTeamString;

/// `Scr_GetTeam`: a script team string to `team_t`. `neutral`, `none` and
/// `free` give `TEAM_FREE`; anything else unknown is a script error.
pub fn team_from_script_string(s: &str) -> Result<i32, IllegalTeamString> {
    Ok(match s {
        "axis" => team::AXIS,
        "allies" => team::ALLIES,
        "team3" => team::THREE,
        "team4" => team::FOUR,
        "team5" => team::FIVE,
        "team6" => team::SIX,
        "team7" => team::SEVEN,
        "team8" => team::EIGHT,
        "spectator" => team::SPECTATOR,
        "neutral" | "none" | "free" => team::FREE,
        _ => return Err(IllegalTeamString),
    })
}

/// `AddScrTeamName`: the string script reads back for a team. Only these
/// four have one; BO2 warns and pushes undefined for the rest.
#[must_use]
pub fn script_team_name(team: i32) -> Option<&'static str> {
    match team {
        team::FREE => Some("free"),
        team::ALLIES => Some("allies"),
        team::AXIS => Some("axis"),
        team::SPECTATOR => Some("spectator"),
        _ => None,
    }
}

/// `score_s`, the scoreboard row carried in the client state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub ping: i32,
    pub status_icon: i32,
    pub place: i32,
    pub score: i32,
    pub kills: i32,
    pub assists: i32,
    pub deaths: i32,
    pub wager_winnings: i32,
    pub scoreboard_columns: [i32; 5],
    pub downs: i32,
    pub revives: i32,
    pub headshots: i32,
    pub score_multiplier: i32,
    pub current_streak: i32,
}

/// `clientState_s.name`: 32 bytes, NUL-terminated.
pub const NAME_LEN: usize = 32;
/// `clientState_s.clanAbbrev`: 8 bytes, NUL-terminated.
pub const CLAN_ABBREV_LEN: usize = 8;
/// What `ClientCleanName` falls back to for an empty name.
pub const UNNAMED_PLAYER: &str = "UnnamedPlayer";

/// `clientState_s` in BO2's field order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientState {
    pub client_index: i32,
    pub team: i32,
    pub ffa_team: i32,
    pub modelindex: i32,
    pub riot_shield_next: i32,
    pub attach_model_index: [i32; 6],
    pub attach_tag_index: [i32; 6],
    pub name: [u8; NAME_LEN],
    pub max_sprint_time_multiplier: f32,
    pub rank: i32,
    /// A union in BO2: `prestige` in MP, `lastDaysPlayed` elsewhere.
    pub prestige: i32,
    pub last_damage_time: i32,
    pub last_stand_start_time: i32,
    pub turned_human_time: i32,
    pub being_revived: i32,
    pub xuid: u64,
    pub league_team_id: u64,
    pub league_division_id: i32,
    pub league_subdivision_id: u64,
    pub league_subdivision_rank: i32,
    pub perks: [u32; 2],
    pub voice_connectivity_bits: i32,
    pub clan_abbrev: [u8; CLAN_ABBREV_LEN],
    pub attached_veh_ent_num: i32,
    pub attached_veh_seat: i32,
    pub needs_revive: i32,
    pub clan_abbrev_ev: i32,
    pub veh_anim_state: i32,
    pub score: Score,
    pub client_ui_visibility_flags: i32,
    pub offhand_weapon_visible: i32,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            client_index: 0,
            team: team::FREE,
            ffa_team: ffa_team::NONE,
            modelindex: 0,
            riot_shield_next: 0,
            attach_model_index: [0; 6],
            attach_tag_index: [0; 6],
            name: [0; NAME_LEN],
            max_sprint_time_multiplier: 0.0,
            rank: 0,
            prestige: 0,
            last_damage_time: 0,
            last_stand_start_time: 0,
            turned_human_time: 0,
            being_revived: 0,
            xuid: 0,
            league_team_id: 0,
            league_division_id: 0,
            league_subdivision_id: 0,
            league_subdivision_rank: 0,
            perks: [0; 2],
            voice_connectivity_bits: 0,
            clan_abbrev: [0; CLAN_ABBREV_LEN],
            attached_veh_ent_num: 0,
            attached_veh_seat: 0,
            needs_revive: 0,
            clan_abbrev_ev: 0,
            veh_anim_state: veh_anim_state::IDLE,
            score: Score::default(),
            client_ui_visibility_flags: 0,
            offhand_weapon_visible: 0,
        }
    }
}

/// The text of a NUL-terminated byte field, up to the first NUL. Bytes that
/// are not UTF-8 end the text there.
#[must_use]
pub fn c_str(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    match core::str::from_utf8(&bytes[..end]) {
        Ok(s) => s,
        Err(e) => core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
    }
}

/// `I_strncpyz`: copy at most `N - 1` bytes and NUL-terminate.
#[must_use]
pub fn strncpyz<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    let n = s.len().min(N.saturating_sub(1));
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    out
}

/// `ClientCleanName` into the 32-byte name: drops leading spaces, `^`
/// colour codes (the `^` and the byte after it) and any space past three
/// in a row, keeps at most 31 bytes, and gives `UnnamedPlayer` when nothing
/// is left.
#[must_use]
pub fn clean_name(input: &str) -> [u8; NAME_LEN] {
    let src = input.as_bytes();
    let mut out = [0u8; NAME_LEN];
    let mut len = 0usize;
    let mut spaces = 0;
    let mut i = 0usize;
    while i < src.len() && src[i] != 0 {
        let c = src[i];
        i += 1;
        if len == 0 && c == b' ' {
            continue;
        }
        if c == b'^' {
            i += 1;
            continue;
        }
        if c == b' ' {
            spaces += 1;
            if spaces > 3 {
                continue;
            }
        } else {
            spaces = 0;
        }
        if len > NAME_LEN - 2 {
            break;
        }
        out[len] = c;
        len += 1;
    }
    if len == 0 {
        out = strncpyz(UNNAMED_PLAYER);
    }
    out
}

impl ClientState {
    #[must_use]
    pub fn name_str(&self) -> &str {
        c_str(&self.name)
    }

    #[must_use]
    pub fn clan_abbrev_str(&self) -> &str {
        c_str(&self.clan_abbrev)
    }

    /// The clan tag as `ClientUserinfoChanged` stores it: at most 7 bytes.
    pub fn set_clan_abbrev(&mut self, s: &str) {
        self.clan_abbrev = strncpyz(s);
    }

    /// `CS_DisplayName`. Bit 1 of `parts` shows the name, bit 2 the clan
    /// tag as `[TAG]`; the tag bit is dropped when there is no tag.
    #[must_use]
    pub fn display_name(&self, parts: i32) -> DisplayName<'_> {
        let parts = if self.clan_abbrev[0] == 0 { parts & !2 } else { parts };
        let (clan, name) = match parts {
            3 => (Some(self.clan_abbrev_str()), Some(self.name_str())),
            1 => (None, Some(self.name_str())),
            2 => (Some(self.clan_abbrev_str()), None),
            _ => (None, None),
        };
        DisplayName { clan, name }
    }
}

/// What `CS_DisplayName` prints: `[clan]name`, `name`, `[clan]` or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayName<'a> {
    pub clan: Option<&'a str>,
    pub name: Option<&'a str>,
}

impl fmt::Display for DisplayName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(clan) = self.clan {
            write!(f, "[{clan}]")?;
        }
        if let Some(name) = self.name {
            f.write_str(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_numbers_and_names() {
        assert_eq!(bg_team_name(team::ALLIES), Some("TEAM_ALLIES"));
        assert_eq!(bg_team_name(team::AXIS), Some("TEAM_AXIS"));
        assert_eq!(bg_team_name(team::SPECTATOR), Some("TEAM_SPECTATOR"));
        assert_eq!(bg_team_name(team::NUM_TEAMS), None);
        assert_eq!(bg_team_name(-1), None);
        assert_eq!(ffa_team_name(ffa_team::NONE), Some("TEAM_SPECTATOR"));
        assert_eq!(ffa_team_name(4), None);
    }

    #[test]
    fn script_team_strings() {
        assert_eq!(team_from_script_string("axis"), Ok(2));
        assert_eq!(team_from_script_string("allies"), Ok(1));
        assert_eq!(team_from_script_string("team8"), Ok(8));
        assert_eq!(team_from_script_string("spectator"), Ok(9));
        assert_eq!(team_from_script_string("neutral"), Ok(0));
        assert_eq!(team_from_script_string("free"), Ok(0));
        assert_eq!(team_from_script_string("red"), Err(IllegalTeamString));
        assert_eq!(script_team_name(team::AXIS), Some("axis"));
        assert_eq!(script_team_name(team::THREE), None);
    }

    #[test]
    fn clean_names() {
        assert_eq!(c_str(&clean_name("  ^1Red^7Dog")), "RedDog");
        assert_eq!(c_str(&clean_name("a      b")), "a   b");
        assert_eq!(c_str(&clean_name("")), "UnnamedPlayer");
        assert_eq!(c_str(&clean_name("   ")), "UnnamedPlayer");
        assert_eq!(c_str(&clean_name("^1")), "UnnamedPlayer");
        assert_eq!(c_str(&clean_name("x^")), "x");
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(c_str(&clean_name(long)), &long[..31]);
    }

    #[test]
    fn display_names() {
        let mut cs = ClientState { name: clean_name("Bot"), ..ClientState::default() };
        assert_eq!(cs.display_name(3).name, Some("Bot"));
        assert_eq!(cs.display_name(3).clan, None);
        assert_eq!(cs.display_name(2), DisplayName { clan: None, name: None });
        cs.set_clan_abbrev("LONGCLANTAG");
        assert_eq!(cs.clan_abbrev_str(), "LONGCLA");
        let d = cs.display_name(3);
        assert_eq!((d.clan, d.name), (Some("LONGCLA"), Some("Bot")));
        assert_eq!(cs.display_name(2).name, None);
        assert_eq!(cs.display_name(0), DisplayName { clan: None, name: None });
    }
}
