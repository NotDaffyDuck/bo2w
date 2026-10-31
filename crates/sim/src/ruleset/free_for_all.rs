use gamemode_t6::dm;

/// What a free-for-all match is called and when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeathmatchRuleset {
    /// Points that end the match.
    pub win_score: i32,
    /// Milliseconds that end the match.
    pub round_ms: u32,
    /// The gametype's name in game files and dvars.
    pub mode_token: &'static str,
    /// The gametype's name on screen.
    pub shown_label: &'static str,
}

/// The rules every match plays by.
pub const FFA: DeathmatchRuleset = DeathmatchRuleset::from_gametype();

impl DeathmatchRuleset {
    /// The rules exactly as the gametype defines them.
    pub const fn from_gametype() -> Self {
        Self {
            shown_label: dm::MODE_TITLE,
            mode_token: dm::MODE_TOKEN,
            round_ms: dm::TIME_LIMIT_MS,
            win_score: dm::POINTS_TO_WIN,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_in_force_are_the_gametype_constants() {
        assert_eq!(FFA, DeathmatchRuleset::from_gametype());
        assert_eq!(
            (FFA.win_score, FFA.round_ms, FFA.mode_token, FFA.shown_label),
            (dm::POINTS_TO_WIN, dm::TIME_LIMIT_MS, dm::MODE_TOKEN, dm::MODE_TITLE),
        );
    }
}
