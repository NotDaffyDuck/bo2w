//! Which authored spawn points a player may be put on.

use entity_t6::team;
use gamemode_t6::dm::SPAWN_CLASSNAME;
use gamemode_t6::{GameModeKind, Team};

use super::AuthoredSpawnPoint;

/// The game mode team for a client state's team number.
pub fn team_from_client_state(client_state_team: i32) -> Team {
    match client_state_team {
        team::ALLIES => Team::Allies,
        team::AXIS => Team::Axis,
        team::SPECTATOR => Team::Spectator,
        _ => Team::Free,
    }
}

/// Indices of the points in `spawns` whose classname satisfies `wanted`, in list order.
fn indices_where(spawns: &[AuthoredSpawnPoint], wanted: impl Fn(&str) -> bool) -> Vec<usize> {
    let mut found = Vec::new();
    for (index, point) in spawns.iter().enumerate() {
        if wanted(&point.classname) {
            found.push(index);
        }
    }
    found
}

/// Free-for-all uses the deathmatch points, or any point at all when the map has none.
fn free_for_all(spawns: &[AuthoredSpawnPoint]) -> Vec<usize> {
    let deathmatch = indices_where(spawns, |classname| classname == SPAWN_CLASSNAME);
    if deathmatch.is_empty() { (0..spawns.len()).collect() } else { deathmatch }
}

/// The points `client_state_team` may spawn on in game type `kind`. A team game offers its team's start points while
/// `use_start_spawns` asks for them (and the map has any), else the team's ordinary points; a team with neither gets
/// nothing. Everything else spawns as free for all.
pub fn spawn_candidate_indices_for(
    spawns: &[AuthoredSpawnPoint],
    kind: GameModeKind,
    client_state_team: i32,
    use_start_spawns: bool,
) -> Vec<usize> {
    let team = team_from_client_state(client_state_team);
    if !(kind.is_team() && matches!(team, Team::Allies | Team::Axis)) {
        return free_for_all(spawns);
    }
    if use_start_spawns
        && let Some(start) = kind.team_start_classname(team)
    {
        let starts = indices_where(spawns, |classname| classname == start);
        if !starts.is_empty() {
            return starts;
        }
    }
    let grid = kind.team_grid_classnames(team);
    if grid.is_empty() {
        return Vec::new();
    }
    indices_where(spawns, |classname| grid.iter().any(|want| *want == classname))
}
