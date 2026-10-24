pub const GAMETYPE_TOKEN: &str = "dm";

pub const DISPLAY_NAME: &str = "FREE FOR ALL";

// gm-13: BO2 reads the limits from playlist data. TODO(fact): the BO2 defaults.
pub const SCORE_LIMIT: i32 = 1000;

pub const TIME_LIMIT_MS: u32 = 600_000;

/// gm-7: both teams spawn from the one grid; dm turns start spawns off.
pub const SPAWN_CLASSNAME: &str = "mp_dm_spawn";

pub const USE_START_SPAWNS: bool = false;
