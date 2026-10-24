use gamemode_t6::GameModeKind;

use super::choose::{spawn_candidate_indices_for, team_from_client_state};
use super::place::{FORCED_SPAWN_SOURCE, SpawnReject, crowded};
use super::{AuthoredSpawnPoint, HostGameModeSelection, MatchBootstrap};
use crate::bullet_collision::{PLAYER_MAXS, PLAYER_MINS};

fn point(classname: &str) -> AuthoredSpawnPoint {
    AuthoredSpawnPoint {
        classname: classname.to_string(),
        origin: [0.0; 3],
        angles: [0.0; 3],
        script_linkto: String::new(),
        script_destructable_area: String::new(),
    }
}

fn map_of(classnames: &[&str]) -> Vec<AuthoredSpawnPoint> {
    classnames.iter().map(|name| point(name)).collect()
}

const ALLIES: i32 = entity_t6::team::ALLIES;
const AXIS: i32 = entity_t6::team::AXIS;

#[test]
fn client_state_teams_map_to_game_teams() {
    use gamemode_t6::Team;
    assert_eq!(team_from_client_state(ALLIES), Team::Allies);
    assert_eq!(team_from_client_state(AXIS), Team::Axis);
    assert_eq!(team_from_client_state(entity_t6::team::SPECTATOR), Team::Spectator);
    assert_eq!(team_from_client_state(entity_t6::team::FREE), Team::Free);
    assert_eq!(team_from_client_state(-7), Team::Free);
}

#[test]
fn free_for_all_takes_deathmatch_points_or_else_any() {
    let mixed = map_of(&["mp_tdm_spawn", "mp_dm_spawn", "mp_dom_spawn", "mp_dm_spawn"]);
    let ffa = |map: &[AuthoredSpawnPoint]| spawn_candidate_indices_for(map, GameModeKind::FreeForAll, 0, false);
    assert_eq!(ffa(&mixed), vec![1, 3]);
    assert_eq!(ffa(&map_of(&["mp_tdm_spawn", "mp_dom_spawn"])), vec![0, 1]);
    assert!(ffa(&[]).is_empty());
}

#[test]
fn a_team_game_offers_its_own_points() {
    let map = map_of(&[
        "mp_tdm_spawn_allies_start",
        "mp_tdm_spawn",
        "mp_tdm_spawn_axis_start",
        "mp_dm_spawn",
        "mp_tdm_spawn",
    ]);
    let tdm = GameModeKind::TeamDeathmatch;
    // At the start of a match the team's start points; afterwards the shared grid.
    assert_eq!(spawn_candidate_indices_for(&map, tdm, ALLIES, true), vec![0]);
    assert_eq!(spawn_candidate_indices_for(&map, tdm, AXIS, true), vec![2]);
    assert_eq!(spawn_candidate_indices_for(&map, tdm, ALLIES, false), vec![1, 4]);
}

#[test]
fn start_points_fall_back_to_the_grid_when_the_map_has_none() {
    let map = map_of(&["mp_tdm_spawn", "mp_tdm_spawn"]);
    assert_eq!(spawn_candidate_indices_for(&map, GameModeKind::TeamDeathmatch, ALLIES, true), vec![0, 1]);
}

#[test]
fn a_team_with_no_points_gets_none_and_a_spectator_is_treated_as_free() {
    let only_deathmatch = map_of(&["mp_dm_spawn"]);
    assert!(spawn_candidate_indices_for(&only_deathmatch, GameModeKind::TeamDeathmatch, AXIS, false).is_empty());
    let spectator = entity_t6::team::SPECTATOR;
    assert_eq!(
        spawn_candidate_indices_for(&only_deathmatch, GameModeKind::TeamDeathmatch, spectator, false),
        vec![0]
    );
}

#[test]
fn a_spawn_is_crowded_only_inside_a_full_player_box() {
    let size: [f32; 3] = core::array::from_fn(|axis| PLAYER_MAXS[axis] - PLAYER_MINS[axis]);
    let at = [100.0, 200.0, 0.0];
    let beside = |gap: f32| [at[0] + gap, at[1], at[2]];
    assert!(crowded(&[at], at));
    assert!(crowded(&[beside(size[0] - 0.5)], at));
    assert!(!crowded(&[beside(size[0])], at));
    assert!(!crowded(&[], at));
    // One axis clear is enough.
    assert!(!crowded(&[[at[0], at[1], at[2] + size[2] + 1.0]], at));
}

#[test]
fn rejections_keep_their_log_names() {
    let all = [
        (SpawnReject::NoAuthoredCandidates, "no_authored_candidates"),
        (SpawnReject::NoGroundHit, "no_ground_hit"),
        (SpawnReject::StartSolid, "start_solid"),
        (SpawnReject::Occupied, "occupied"),
        (SpawnReject::UnsupportedCoverage, "unsupported_coverage"),
        (SpawnReject::AllRejected, "all_rejected"),
    ];
    for (reason, name) in all {
        assert_eq!(reason.as_str(), name);
    }
    assert_eq!(FORCED_SPAWN_SOURCE, usize::MAX);
}

#[test]
fn the_host_menu_lists_eight_game_types_each_found_by_its_token() {
    assert_eq!(HostGameModeSelection::ALL.len(), 8);
    for selection in HostGameModeSelection::ALL {
        assert_eq!(HostGameModeSelection::from_token(selection.token()), Some(selection));
        assert!(!selection.display_name().is_empty());
    }
    assert_eq!(HostGameModeSelection::from_token("TDM").map(HostGameModeSelection::kind), Some(GameModeKind::TeamDeathmatch));
    assert_eq!(HostGameModeSelection::from_token("no such mode"), None);
}

#[test]
fn a_fresh_bootstrap_is_an_empty_free_for_all() {
    let setup = MatchBootstrap::default();
    assert!(setup.spawns.is_empty() && setup.classes.is_empty() && setup.bot_classes.is_empty());
    assert_eq!(setup.kind, GameModeKind::FreeForAll);
    assert!(!setup.allow_debug_actions);
    assert!(setup.intermission_view.is_none() && setup.airstrike_height.is_none());
}
