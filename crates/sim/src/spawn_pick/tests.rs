use gamemode_t6::RulesetKind;

use super::choose::{eligible_spawn_rows, team_from_client_state};
use super::place::{FORCED_SPAWN_SOURCE, SpawnRefusalCause, crowded};
use super::{MapSpawnSpot, HostChosenMode, MatchStartSetup, ruleset_named};
use crate::shot_trace::{BODY_BOX_UPPER, BODY_BOX_LOWER};

fn point(classname: &str) -> MapSpawnSpot {
    MapSpawnSpot {
        kind_name: classname.to_string(),
        origin: [0.0; 3],
        turn: Default::default(),
        script_tied: Default::default(),
        script_destructable_area: Default::default(),
    }
}

fn map_of(classnames: &[&str]) -> Vec<MapSpawnSpot> {
    classnames.iter().map(|name| point(name)).collect()
}

const ALLIES: i32 = entity_t6::team::ALLIES;
const AXIS: i32 = entity_t6::team::AXIS;
const DEATHMATCH_POINT: &str = "mp_dm_spawn";

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
    let ffa = |map: &[MapSpawnSpot]| eligible_spawn_rows(map, RulesetKind::FreeForAll, 0, false);
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
        DEATHMATCH_POINT,
        "mp_tdm_spawn",
    ]);
    let tdm = RulesetKind::TeamDeathmatch;
    // At the start of a match the team's start points; afterwards the shared grid.
    assert_eq!(eligible_spawn_rows(&map, tdm, ALLIES, true), vec![0]);
    assert_eq!(eligible_spawn_rows(&map, tdm, AXIS, true), vec![2]);
    assert_eq!(eligible_spawn_rows(&map, tdm, ALLIES, false), vec![1, 4]);
}

#[test]
fn start_points_fall_back_to_the_grid_when_the_map_has_none() {
    let map = map_of(&["mp_tdm_spawn", "mp_tdm_spawn"]);
    assert_eq!(eligible_spawn_rows(&map, RulesetKind::TeamDeathmatch, ALLIES, true), vec![0, 1]);
}

#[test]
fn a_team_with_no_points_gets_none_and_a_spectator_is_treated_as_free() {
    let only_deathmatch = map_of(&["mp_dm_spawn"]);
    assert!(eligible_spawn_rows(&only_deathmatch, RulesetKind::TeamDeathmatch, AXIS, false).is_empty());
    let spectator = entity_t6::team::SPECTATOR;
    assert_eq!(
        eligible_spawn_rows(&only_deathmatch, RulesetKind::TeamDeathmatch, spectator, false),
        vec![0]
    );
}

#[test]
fn a_spawn_is_crowded_only_inside_a_full_player_box() {
    let size: [f32; 3] = core::array::from_fn(|axis| BODY_BOX_UPPER[axis] - BODY_BOX_LOWER[axis]);
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
        (SpawnRefusalCause::NoSpawnPoints, "no_authored_candidates"),
        (SpawnRefusalCause::NoFloorBelow, "no_ground_hit"),
        (SpawnRefusalCause::StartSolid, "start_solid"),
        (SpawnRefusalCause::Occupied, "occupied"),
        (SpawnRefusalCause::NoWorldClip, "unsupported_coverage"),
        (SpawnRefusalCause::EveryPointRefused, "all_rejected"),
    ];
    for (reason, name) in all {
        assert_eq!(reason.as_str(), name);
    }
    assert_eq!(FORCED_SPAWN_SOURCE, usize::MAX);
}

#[test]
fn the_host_menu_lists_eight_game_types_each_found_by_its_token() {
    assert_eq!(HostChosenMode::ALL.len(), 8);
    for selection in HostChosenMode::ALL {
        assert_eq!(HostChosenMode::from_short_code(selection.token()), Some(selection));
        assert!(!selection.shown_label().is_empty());
    }
    assert_eq!(HostChosenMode::from_short_code("TDM").map(HostChosenMode::kind), Some(RulesetKind::TeamDeathmatch));
    assert_eq!(HostChosenMode::from_short_code("no such mode"), None);
}

#[test]
fn a_fresh_bootstrap_is_an_empty_free_for_all() {
    let setup = MatchStartSetup::default();
    assert!(setup.spawns.is_empty() && setup.classes.is_empty() && setup.bot_classes.is_empty());
    assert_eq!(setup.kind, RulesetKind::FreeForAll);
    assert!(!setup.permit_cheat_requests);
    assert!(setup.scoreboard_eye.is_none() && setup.airstrike_height.is_none());
}

#[test]
fn kill_confirmed_spawns_on_team_deathmatch_rules() {
    assert_eq!(ruleset_named(Some("conf")), RulesetKind::TeamDeathmatch);
    assert_eq!(ruleset_named(Some("nonsense")), RulesetKind::FreeForAll);
    assert_eq!(ruleset_named(None), RulesetKind::FreeForAll);
}
