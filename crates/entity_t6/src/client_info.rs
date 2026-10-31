//! Per-player facts the client snapshot carries: BO2's team numbers and team
//! strings (`BG_GetTeamName`, `Scr_GetTeam`), the scoreboard row, and the
//! fixed-size NUL-terminated name fields.

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
pub mod team_code {
    pub const NONE: i32 = 0;
    pub const AXIS: i32 = 1;
    pub const ALLIES: i32 = 2;
    pub const THREE: i32 = 3;
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
        team_code::NONE => Some("TEAM_SPECTATOR"),
        team_code::AXIS => Some("TEAM_AXIS"),
        team_code::ALLIES => Some("TEAM_ALLIES"),
        team_code::THREE => Some("TEAM_THREE"),
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
pub struct ScoreRow {
    pub round_trip: i32,
    pub status_icon: i32,
    pub place: i32,
    pub tally: i32,
    pub eliminations: i32,
    pub helps: i32,
    pub times_died: i32,
    pub wager_winnings: i32,
    pub scoreboard_columns: [i32; 5],
    pub downs: i32,
    pub revives: i32,
    pub headshots: i32,
    pub score_multiplier: i32,
    pub current_streak: i32,
}

/// The text of a NUL-terminated byte field, up to the first NUL. Bytes that
/// are not UTF-8 end the text there.
#[must_use]
pub fn c_str(bytes: &[u8]) -> &str {
    let end = bytes.iter().take_while(|&&b| b != 0).count();
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
        assert_eq!(ffa_team_name(team_code::NONE), Some("TEAM_SPECTATOR"));
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
}
