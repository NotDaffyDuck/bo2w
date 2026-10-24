pub const GAMETYPE_TOKEN: &str = "dom";

pub const DISPLAY_NAME: &str = "DOMINATION";

// gm-13: BO2 reads the limits from playlist data. TODO(fact): the BO2 defaults.
pub const SCORE_LIMIT: i32 = 300;

pub const TIME_LIMIT_MS: u32 = 1_800_000;

/// gm-9.
pub const SPAWN_CLASSNAME: &str = "mp_dom_spawn";

pub const START_SPAWN_ALLIES: &str = "mp_dom_spawn_allies_start";

pub const START_SPAWN_AXIS: &str = "mp_dom_spawn_axis_start";

/// gm-9: flag spawns are this prefix plus the flag label.
pub const FLAG_SPAWN_PREFIX: &str = "mp_dom_spawn_flag";
