//! Black Ops II's side of the script runtime: the start-up stubs the match
//! installs, the commands they call, and what they do to players, props and
//! missiles. The language itself lives in `lang` and knows none of this.

pub(crate) mod controls;
pub(crate) mod damage;
pub(crate) mod dof;
pub(crate) mod level;
pub(crate) mod match_start;
pub(crate) mod natives;
pub(crate) mod players;
pub(crate) mod presence;
pub(crate) mod runtime;
pub(crate) mod startup;
pub(crate) mod volume;
pub(crate) mod weapons;

/// Level notifies the step loop answers itself.
pub const EXIT_LEVEL: &str = "engine:exitlevel";
pub const MAP_RESTART: &str = "engine:map_restart";
