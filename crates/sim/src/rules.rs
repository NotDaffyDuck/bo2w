use gamemode_t6::dm;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeForAllRules {
    pub gametype_token: &'static str,
    pub display_name: &'static str,
    pub score_limit: i32,
    pub time_limit_ms: u32,
}

impl FreeForAllRules {
    pub const fn format_defaults() -> Self {
        Self {
            gametype_token: dm::GAMETYPE_TOKEN,
            display_name: dm::DISPLAY_NAME,
            score_limit: dm::SCORE_LIMIT,
            time_limit_ms: dm::TIME_LIMIT_MS,
        }
    }
}

pub const FFA: FreeForAllRules = FreeForAllRules::format_defaults();
