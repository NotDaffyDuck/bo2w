//! Which authored spawn points a player may be put on.

use entity_t6::team;
use gamemode_t6::dm::SPAWN_POINT_CLASS;
use gamemode_t6::{RulesetKind, Team};

use super::MapSpawnSpot;

/// The game mode team for a client state's team number.
pub fn team_from_client_state(team_number: i32) -> Team {
    match team_number {
        team::ALLIES => Team::Allies,
        team::AXIS => Team::Axis,
        team::SPECTATOR => Team::Spectator,
        _unlisted => Team::Free,
    }
}

/// Indices of the points in `spawns` whose classname satisfies `wanted`, in list order.
fn indices_where(spawns: &[MapSpawnSpot], wanted: impl Fn(&str) -> bool) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, point) in spawns.iter().enumerate() {
        if wanted(&point.kind_name) {
            found.push(index);
        }
    }
    found
}

/// Free-for-all uses the deathmatch points, or any point at all when the map has none.
fn free_for_all(spawns: &[MapSpawnSpot]) -> Vec<usize> {
    let deathmatch = indices_where(spawns, |classname| classname == SPAWN_POINT_CLASS);
    if deathmatch.is_empty() { (0..spawns.len()).collect() } else { deathmatch }
}

/// The points the team numbered `team_number` may spawn on in game type `game_type`. A team game offers its team's start
/// points while `prefer_start_points` asks for them (and the map has any), else the team's ordinary points; a team with
/// neither gets nothing. Every other case spawns as free for all.
pub fn eligible_spawn_rows(
    points: &[MapSpawnSpot],
    game_type: RulesetKind,
    team_number: i32,
    prefer_start_points: bool,
) -> Vec<usize> {
    let team = team_from_client_state(team_number);
    let plays_for_a_side = matches!(team, Team::Allies | Team::Axis);
    if !game_type.has_teams() || !plays_for_a_side {
        return free_for_all(points);
    }
    if prefer_start_points
        && let Some(start) = game_type.side_spawn_classname(team)
    {
        let starts = indices_where(points, |classname| classname == start);
        if !starts.is_empty() {
            return starts;
        }
    }
    let grid = game_type.side_grid_classnames(team);
    if grid.is_empty() {
        return Vec::new();
    }
    indices_where(points, |classname| grid.iter().any(|want| *want == classname))
}
