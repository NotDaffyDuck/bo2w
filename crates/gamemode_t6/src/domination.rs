pub const MODE_TOKEN: &str = "dom";

pub const MODE_TITLE: &str = "DOMINATION";

// gm-13: BO2 reads the limits from playlist data. TODO(fact): the BO2 defaults.
pub const POINTS_TO_WIN: i32 = 300;

pub const TIME_LIMIT_MS: u32 = 30 * 60 * 1_000;

/// gm-9.
pub const SPAWN_POINT_CLASS: &str = "mp_dom_spawn";

pub const OPENING_SPAWN_ALLIES: &str = "mp_dom_spawn_allies_start";

pub const OPENING_SPAWN_AXIS: &str = "mp_dom_spawn_axis_start";

/// gm-9: flag spawns are this prefix plus the flag label.
pub const FLAG_SPAWN_PREFIX: &str = "mp_dom_spawn_flag";
