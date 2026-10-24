//! Where players start: the spawn points a map authors, the choice among them, and the match setup the host hands the
//! simulation.

mod choose;
mod place;
#[cfg(test)]
mod tests;

use gamemode_t6::GameModeKind;

use crate::match_state::ClassDef;

pub use choose::{spawn_candidate_indices_for, team_from_client_state};
pub use place::{FORCED_SPAWN_SOURCE, SpawnAttemptReport, SpawnReject};
pub(crate) use place::decide_forced_spawn;

/// A spawn entity as the map authors it.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredSpawnPoint {
    pub classname: String,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// The entity's `script_linkto` key.
    pub script_linkto: String,
    /// The entity's `script_destructable_area` key.
    pub script_destructable_area: String,
}

/// A spawn point settled on, with where it was before the floor was found under it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnDecision {
    pub classname: String,
    /// Index into the authored list, or `FORCED_SPAWN_SOURCE` for a spot given by position.
    pub source_index: usize,
    pub raw_origin: [f32; 3],
    pub raw_angles: [f32; 3],
    pub traced_origin: [f32; 3],
}

/// Everything the host decides before a match starts.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchBootstrap {
    pub spawns: Vec<AuthoredSpawnPoint>,
    pub classes: Vec<ClassDef>,
    pub bot_classes: Vec<ClassDef>,
    /// Seed of the match's random stream.
    pub seed: u64,
    pub kind: GameModeKind,
    /// Whether console commands that cheat are honoured.
    pub allow_debug_actions: bool,
    pub score_limit: i32,
    pub time_limit_ms: u32,
    /// Where the camera sits between matches, if the map has such a spot.
    pub intermission_view: Option<AuthoredSpawnPoint>,
    /// Altitude of the map's airstrike plane, if it sets one.
    pub airstrike_height: Option<f32>,
}

impl Default for MatchBootstrap {
    fn default() -> Self {
        let (score_limit, time_limit_ms) = crate::score::bootstrap_score_defaults();
        Self {
            spawns: Vec::new(),
            classes: Vec::new(),
            bot_classes: Vec::new(),
            seed: 0,
            kind: GameModeKind::FreeForAll,
            allow_debug_actions: false,
            score_limit,
            time_limit_ms,
            intermission_view: None,
            airstrike_height: None,
        }
    }
}

/// The game type named by `BO2MP_GAMETYPE`, free for all when it is unset or unknown.
pub fn host_game_mode_kind() -> GameModeKind {
    let named = std::env::var("BO2MP_GAMETYPE").ok();
    named.as_deref().and_then(GameModeKind::parse_ascii_ignore_case).unwrap_or(GameModeKind::FreeForAll)
}

/// The game type the host runs the next match as.
#[derive(bevy_ecs::prelude::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostGameModeSelection(GameModeKind);

impl HostGameModeSelection {
    /// Every game type the host can run, in menu order.
    pub const ALL: [Self; 8] = [
        Self(GameModeKind::FreeForAll),
        Self(GameModeKind::Domination),
        Self(GameModeKind::Demolition),
        Self(GameModeKind::TeamDeathmatch),
        Self(GameModeKind::SearchAndDestroy),
        Self(GameModeKind::CaptureTheFlag),
        Self(GameModeKind::Headquarters),
        Self(GameModeKind::Hardpoint),
    ];

    pub fn from_token(token: &str) -> Option<Self> {
        GameModeKind::parse_ascii_ignore_case(token).map(Self)
    }

    pub fn from_env() -> Self {
        Self(host_game_mode_kind())
    }

    pub const fn kind(self) -> GameModeKind {
        self.0
    }

    pub fn token(self) -> &'static str {
        self.0.token()
    }

    pub fn display_name(self) -> &'static str {
        self.0.display_name()
    }
}
