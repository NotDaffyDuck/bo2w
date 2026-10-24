//! Behaviour pins for the first-person held-weapon sync system.
//!
//! The weapon table (`PreparedFpv`) cannot be built by hand here, so these cover
//! every path that is reachable without one: clearing on life changes, on the
//! intermission, on a missing player or an empty hand, the early returns that
//! leave state alone, and the "catalog missing" wait.

use bevy::prelude::*;
use frame::{LaunchReport, LifeEndCause, LifeEnded, LifeStartReason, LifeStarted, ViewSubject};
use net::{LocalPresentClient, PresentedSnapshot};
use playerstate_t6::game::PlayerState;
use sim::{ClientId, MatchPhase, Snapshot, Tick};

use super::*;
use crate::anim::fpv_prepared::PreparedFpv;
use crate::gaps::{RenderGapCause, RenderPresentationGaps};
use crate::occupancy::fpv_present::{
    FpvHeldLife, FpvHeldSettled, FpvPlacementRoot, FpvState, FpvStatusGap, PendingFpvSpawn,
    PendingFpvSpawnRequest, SessionViewmodel,
};

fn test_app() -> App {
    let mut app = App::new();
    app.add_message::<LifeStarted>()
        .add_message::<LifeEnded>()
        .init_resource::<PresentedSnapshot>()
        .init_resource::<LocalPresentClient>()
        .init_resource::<ViewSubject>()
        .init_resource::<PreparedFpv>()
        .init_resource::<PendingFpvSpawn>()
        .init_resource::<SessionViewmodel>()
        .init_resource::<FpvHeldSettled>()
        .init_resource::<FpvHeldLife>()
        .init_resource::<FpvStatusGap>()
        .init_resource::<RenderPresentationGaps>()
        .init_resource::<LaunchReport>()
        .add_systems(Update, sync_fpv_to_held_weapon);
    app
}

/// Publish a snapshot in which client 0 stands holding `weapon`.
fn publish_player(app: &mut App, weapon: u32, phase: MatchPhase) {
    let mut snapshot = Snapshot::unpublished(Tick(1));
    snapshot.meta.phase = phase;
    let ps = PlayerState {
        weapon,
        ..PlayerState::ZERO
    };
    snapshot.players.push((ClientId(0), ps));
    app.world_mut()
        .resource_mut::<PresentedSnapshot>()
        .publish_decoded(snapshot);
}

fn publish_empty(app: &mut App, phase: MatchPhase) {
    let mut snapshot = Snapshot::unpublished(Tick(1));
    snapshot.meta.phase = phase;
    app.world_mut()
        .resource_mut::<PresentedSnapshot>()
        .publish_decoded(snapshot);
}

fn status(app: &App) -> Option<FpvState> {
    app.world().resource::<FpvStatusGap>().0.clone()
}

fn settled(app: &App) -> Option<u32> {
    app.world().resource::<FpvHeldSettled>().0
}

fn held_life(app: &App) -> Option<u32> {
    app.world().resource::<FpvHeldLife>().0
}

fn spawn_root(app: &mut App) -> Entity {
    app.world_mut().spawn(FpvPlacementRoot).id()
}

fn queued_request(weapon_id: u32, parent_weapon: u32) -> PendingFpvSpawnRequest {
    PendingFpvSpawnRequest {
        gun_index: assets::FpvMeshIndex::from_order(0),
        catalog_id: 9,
        weapon_id,
        parent_weapon,
    }
}

fn life_started(client: u32, life: u32) -> LifeStarted {
    LifeStarted {
        client,
        life,
        reason: LifeStartReason::BecameAlive,
    }
}

fn life_ended(client: u32, life: u32) -> LifeEnded {
    LifeEnded {
        client,
        life,
        cause: LifeEndCause::LeftAlive,
    }
}

#[test]
fn no_player_clears_everything_and_reports_not_alive() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(4);
    app.world_mut().resource_mut::<FpvHeldLife>().0 = Some(3);
    publish_empty(&mut app, MatchPhase::Playing);

    app.update();

    assert_eq!(status(&app), Some(FpvState::ClearedNotAlive));
    assert_eq!(settled(&app), None);
    assert_eq!(held_life(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn no_player_with_nothing_to_clear_leaves_status_alone() {
    let mut app = test_app();
    publish_empty(&mut app, MatchPhase::Playing);

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(held_life(&app), None);
}

#[test]
fn intermission_clears_but_keeps_the_life_marker() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(4);
    app.world_mut().resource_mut::<FpvHeldLife>().0 = Some(3);
    publish_player(&mut app, 7, MatchPhase::Intermission);

    app.update();

    assert_eq!(status(&app), Some(FpvState::ClearedNotAlive));
    assert_eq!(settled(&app), None);
    assert_eq!(held_life(&app), Some(3));
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn intermission_in_a_killcam_is_not_a_clear() {
    let mut app = test_app();
    *app.world_mut().resource_mut::<ViewSubject>() = ViewSubject::Seat {
        viewer: 0,
        focus: None,
    };
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(7);
    publish_player(&mut app, 7, MatchPhase::Intermission);

    app.update();

    // Carried on to the normal path: weapon 7 was already settled, so no change.
    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(7));
}

#[test]
fn empty_hand_settles_on_zero_and_reports_no_weapon() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    publish_player(&mut app, 0, MatchPhase::Playing);

    app.update();

    assert_eq!(status(&app), Some(FpvState::ClearedNoWeapon));
    assert_eq!(settled(&app), Some(0));
    assert_eq!(
        app.world().resource::<LaunchReport>().sim_gap,
        FpvState::ClearedNoWeapon.label()
    );
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn empty_hand_already_settled_does_nothing_more() {
    let mut app = test_app();
    publish_player(&mut app, 0, MatchPhase::Playing);
    app.update();
    app.world_mut().resource_mut::<FpvStatusGap>().0 = None;

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(0));
}

#[test]
fn queued_request_for_the_held_weapon_is_left_alone() {
    let mut app = test_app();
    app.world_mut().resource_mut::<PendingFpvSpawn>().0 = Some(queued_request(7, 0));
    let root = spawn_root(&mut app);
    publish_player(&mut app, 7, MatchPhase::Playing);

    app.update();

    assert!(app.world().resource::<PendingFpvSpawn>().0.is_some());
    assert_eq!(status(&app), None);
    assert!(app.world().get_entity(root).is_ok());
}

#[test]
fn settled_on_the_held_weapon_with_nothing_live_is_left_alone() {
    let mut app = test_app();
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(7);
    publish_player(&mut app, 7, MatchPhase::Playing);

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(7));
}

#[test]
fn missing_catalog_waits_and_names_the_cause() {
    let mut app = test_app();
    publish_player(&mut app, 7, MatchPhase::Playing);

    app.update();

    assert_eq!(
        status(&app),
        Some(FpvState::Blocked(RenderGapCause::FpvCatalogMissing))
    );
    assert_eq!(settled(&app), None);
    assert!(app.world().resource::<PendingFpvSpawn>().0.is_none());
}

#[test]
fn queued_request_for_another_weapon_waits_for_the_catalog() {
    let mut app = test_app();
    app.world_mut().resource_mut::<PendingFpvSpawn>().0 = Some(queued_request(3, 0));
    publish_player(&mut app, 7, MatchPhase::Playing);

    app.update();

    // The stale request is not touched while there is no table to replace it.
    assert!(app.world().resource::<PendingFpvSpawn>().0.is_some());
    assert_eq!(
        status(&app),
        Some(FpvState::Blocked(RenderGapCause::FpvCatalogMissing))
    );
}

#[test]
fn local_life_start_records_the_life_and_drops_old_state() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(4);
    publish_player(&mut app, 7, MatchPhase::Playing);
    app.world_mut().write_message(life_started(0, 12));

    app.update();

    assert_eq!(held_life(&app), Some(12));
    assert_eq!(settled(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn another_clients_life_events_are_ignored() {
    let mut app = test_app();
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(7);
    app.world_mut().resource_mut::<FpvHeldLife>().0 = Some(2);
    publish_player(&mut app, 7, MatchPhase::Playing);
    app.world_mut().write_message(life_started(5, 99));
    app.world_mut().write_message(life_ended(6, 1));

    app.update();

    assert_eq!(held_life(&app), Some(2));
    assert_eq!(settled(&app), Some(7));
    assert_eq!(status(&app), None);
}

#[test]
fn local_life_end_clears_and_forgets_the_life() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<FpvHeldSettled>().0 = Some(4);
    app.world_mut().resource_mut::<FpvHeldLife>().0 = Some(3);
    publish_player(&mut app, 7, MatchPhase::Playing);
    app.world_mut().write_message(life_ended(0, 3));

    app.update();

    assert_eq!(held_life(&app), None);
    assert_eq!(settled(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn end_and_start_in_one_frame_keeps_the_new_life() {
    let mut app = test_app();
    app.world_mut().resource_mut::<FpvHeldLife>().0 = Some(3);
    publish_player(&mut app, 7, MatchPhase::Playing);
    app.world_mut().write_message(life_ended(0, 3));
    app.world_mut().write_message(life_started(0, 4));

    app.update();

    assert_eq!(held_life(&app), Some(4));
}

#[test]
fn probe_follows_a_changed_status() {
    let mut app = App::new();
    app.init_resource::<FpvStatusGap>()
        .init_resource::<LaunchReport>()
        .add_systems(Update, sync_fpv_status_to_probe);
    app.update();
    // Nothing published yet: the report keeps its starting text.
    let start = app.world().resource::<LaunchReport>().sim_gap;

    app.world_mut().resource_mut::<FpvStatusGap>().0 = Some(FpvState::Queued);
    app.update();

    assert_ne!(app.world().resource::<LaunchReport>().sim_gap, start);
    assert_eq!(
        app.world().resource::<LaunchReport>().sim_gap,
        FpvState::Queued.label()
    );
}

#[test]
fn probe_without_a_status_writes_nothing() {
    let mut app = App::new();
    app.init_resource::<FpvStatusGap>()
        .init_resource::<LaunchReport>()
        .add_systems(Update, sync_fpv_status_to_probe);
    let before = app.world().resource::<LaunchReport>().sim_gap;

    app.update();
    app.world_mut().resource_mut::<FpvStatusGap>().0 = None;
    app.update();

    assert_eq!(app.world().resource::<LaunchReport>().sim_gap, before);
}

#[test]
fn registering_adds_the_life_messages() {
    let mut app = App::new();
    register_held_sync_systems(&mut app);
    assert!(app.world().contains_resource::<Messages<LifeStarted>>());
    assert!(app.world().contains_resource::<Messages<LifeEnded>>());
}
