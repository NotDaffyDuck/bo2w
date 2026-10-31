//! Where players start: the spawn points a map authors, the choice among them, and the match setup the host hands the
//! simulation.

mod choose;
mod place;
#[cfg(test)]
mod tests;

use gamemode_t6::RulesetKind;

use crate::player_ledger::LoadoutClassSpec;

pub use choose::{eligible_spawn_rows, team_from_client_state};
pub use place::{FORCED_SPAWN_SOURCE, SpawnTryOutcome, SpawnRefusalCause};
pub(crate) use place::decide_forced_spawn;

/// A spawn entity as the map authors it.
#[derive(Clone, Debug, PartialEq)]
pub struct MapSpawnSpot {
    pub kind_name: String,
    pub origin: [f32; 3],
    pub turn: [f32; 3],
    /// The entity's `script_linkto` key.
    pub script_tied: String,
    /// The entity's `script_destructable_area` key.
    pub script_destructable_area: String,
}

/// A spawn point settled on, with where it was before the floor was found under it.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnVerdict {
    pub kind_name: String,
    /// Index into the authored list, or `FORCED_SPAWN_SOURCE` for a spot given by position.
    pub origin_row: usize,
    pub authored_origin: [f32; 3],
    pub authored_angles: [f32; 3],
    pub dropped_origin: [f32; 3],
}

/// Everything the host decides before a match starts.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchStartSetup {
    pub spawns: Vec<MapSpawnSpot>,
    pub classes: Vec<LoadoutClassSpec>,
    pub bot_classes: Vec<LoadoutClassSpec>,
    /// The perks the stats tables offer as (stats table number, slot 0..=2); a class may only
    /// carry a perk in the slot listed here.
    pub class_perk_slots: Vec<(u32, u8)>,
    /// Seed of the match's random stream.
    pub root: u64,
    pub kind: RulesetKind,
    /// Whether console commands that cheat are honoured.
    pub permit_cheat_requests: bool,
    pub win_score: i32,
    pub round_ms: u32,
    /// Where the camera sits between matches, if the map has such a spot.
    pub scoreboard_eye: Option<MapSpawnSpot>,
    /// Altitude of the map's airstrike plane, if it sets one.
    pub airstrike_height: Option<f32>,
}

impl Default for MatchStartSetup {
    fn default() -> Self {
        let (default_score, default_time) = crate::round_clock::opening_limits();
        Self {
            kind: RulesetKind::FreeForAll,
            root: 0,
            win_score: default_score,
            round_ms: default_time,
            spawns: Default::default(),
            classes: Default::default(),
            bot_classes: Default::default(),
            class_perk_slots: Default::default(),
            permit_cheat_requests: Default::default(),
            scoreboard_eye: Default::default(),
            airstrike_height: Default::default(),
        }
    }
}

/// The game type named by `BO2MP_GAMETYPE`, free for all when it is unset or unknown.
pub fn hosted_ruleset() -> RulesetKind {
    ruleset_named(std::env::var("BO2MP_GAMETYPE").ok().as_deref())
}

/// The ruleset a game type token plays on (Kill Confirmed on team deathmatch's), free for
/// all when there is none or it is unknown.
pub(crate) fn ruleset_named(named: Option<&str>) -> RulesetKind {
    named.and_then(RulesetKind::for_t6_token).unwrap_or(RulesetKind::FreeForAll)
}

/// The game type the host runs the next match as.
#[derive(bevy_ecs::prelude::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostChosenMode(RulesetKind);

impl HostChosenMode {
    /// Every game type the host can run, in menu order.
    pub const ALL: [Self; 8] = {
        [
            Self(RulesetKind::FreeForAll),
            Self(RulesetKind::Domination),
            Self(RulesetKind::Demolition),
            Self(RulesetKind::TeamDeathmatch),
            Self(RulesetKind::SearchAndDestroy),
            Self(RulesetKind::CaptureTheFlag),
            Self(RulesetKind::Headquarters),
            Self(RulesetKind::Hardpoint),
        ]
    };

    pub fn from_short_code(token: &str) -> Option<Self> {
        let parsed = RulesetKind::parse_ascii_ignore_case(token)?;
        Some(Self(parsed))
    }

    pub fn read_env() -> Self {
        let from_environment = hosted_ruleset();
        Self(from_environment)
    }

    pub const fn kind(self) -> RulesetKind {
        self.0
    }

    pub fn token(self) -> &'static str {
        let Self(inner) = self;
        inner.token()
    }

    pub fn shown_label(self) -> &'static str {
        let Self(inner) = self;
        inner.shown_label()
    }
}
