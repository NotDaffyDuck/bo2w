//! The free-for-all rule set, taken straight from the gametype's own constants.

use gamemode_t6::dm;

/// What a free-for-all match is called and when it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeForAllRules {
    pub display_name: &'static str,
    pub gametype_token: &'static str,
    pub time_limit_ms: u32,
    pub score_limit: i32,
}

impl FreeForAllRules {
    /// The rules the gametype ships with.
    pub const fn format_defaults() -> Self {
        Self {
            display_name: dm::DISPLAY_NAME,
            gametype_token: dm::GAMETYPE_TOKEN,
            time_limit_ms: dm::TIME_LIMIT_MS,
            score_limit: dm::SCORE_LIMIT,
        }
    }
}

/// The free-for-all rules in force.
pub const FFA: FreeForAllRules = FreeForAllRules::format_defaults();
