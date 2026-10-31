pub const MODE_TOKEN: &str = "dem";

pub const MODE_TITLE: &str = "DEMOLITION";

// gm-13: BO2 reads the limits from playlist data. TODO(fact): the BO2 defaults.
pub const TIME_LIMIT_MS: u32 = 3 * 60 * 1_000;

/// gm-10: attackers spawn from one grid; defenders from the main grid plus the _a and _b ones.
pub const SPAWN_ATTACKER: &str = "mp_dem_spawn_attacker";

pub const SPAWN_DEFENDER: &str = "mp_dem_spawn_defender";

pub const SPAWN_DEFENDER_A: &str = "mp_dem_spawn_defender_a";

pub const SPAWN_DEFENDER_B: &str = "mp_dem_spawn_defender_b";

pub const START_SPAWN_ATTACKER: &str = "mp_dem_spawn_attacker_start";

pub const START_SPAWN_DEFENDER: &str = "mp_dem_spawn_defender_start";
