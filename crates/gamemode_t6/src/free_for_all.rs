//! Free-for-all: everyone against everyone, all spawning from the one shared grid. Facts gm-7 and gm-13.

/// The mode's short name in the game type setting, and the name the menus show.
pub const GAMETYPE_TOKEN: &str = "dm";
pub const DISPLAY_NAME: &str = "FREE FOR ALL";

/// Points and time to win. BO2 takes both from playlist data. TODO(fact): its defaults.
pub const SCORE_LIMIT: i32 = 1_000;
pub const TIME_LIMIT_MS: u32 = 10 * 60 * 1_000;

/// Spawn point class. This mode has no start spawns, not even for the first life.
pub const SPAWN_CLASSNAME: &str = "mp_dm_spawn";
pub const USE_START_SPAWNS: bool = false;
