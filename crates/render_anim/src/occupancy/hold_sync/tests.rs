#![cfg(test)]
//! Behaviour pins for the first-person held-weapon sync system.
//!
//! The weapon table (`PreparedFpv`) cannot be built by hand here, so these cover
//! every path that is reachable without one: clearing on life changes, on the
//! intermission, on a missing player or an empty hand, the early returns that
//! leave state alone, and the "catalog missing" wait.

use bevy::prelude::*;
use frame::{StartupReport, DeathCircumstance, SpawnLifeOver, SpawnCause, SpawnLifeBegun, EyeSubject};
use net::{HostViewClient, ShownWorldFrame};
use playerstate_t6::game::AvatarState;
use sim::{ClientId, GameRoundStage, Snapshot, Tick};

use super::*;
use crate::motion::fpv_prepared::PreparedFpv;
use crate::holes::{Cause, ShortfallLedger};
use crate::occupancy::gun_present::{
    HoldLifeStamp, HoldSettledLife, GunMountAnchor, ViewmodelStatus, GunStatusShortfall, WaitingFpvSpawn,
    QueuedGunSpawn, ActiveGunMeshes,
};

fn test_app() -> App {
    let mut app_here = App::new();
    app_here.add_message::<SpawnLifeBegun>()
        .add_message::<SpawnLifeOver>()
        .init_resource::<ShownWorldFrame>()
        .init_resource::<HostViewClient>()
        .init_resource::<EyeSubject>()
        .init_resource::<PreparedFpv>()
        .init_resource::<WaitingFpvSpawn>()
        .init_resource::<ActiveGunMeshes>()
        .init_resource::<HoldSettledLife>()
        .init_resource::<HoldLifeStamp>()
        .init_resource::<GunStatusShortfall>()
        .init_resource::<ShortfallLedger>()
        .init_resource::<StartupReport>()
        .add_systems(bevy::app::Update, match_fpv_to_held);
    app_here
}

/// Publish a snapshot in which client 0 stands holding `weapon`.
fn publish_player(app_v: &mut App, gun: u32, phase: GameRoundStage) {
    let mut snapshot = Snapshot::unpublished(Tick(1));
    snapshot.meta.phase = phase;
    let ps = AvatarState {
        weapon: gun,
        ..AvatarState::ZERO
    };
    snapshot.players.push((ClientId(0), ps));
    app_v.world_mut()
        .resource_mut::<ShownWorldFrame>()
        .announce_parsed(snapshot);
}

fn publish_empty(application: &mut App, phase: GameRoundStage) {
    let mut snapshot = Snapshot::unpublished(Tick(1));
    snapshot.meta.phase = phase;
    application.world_mut()
        .resource_mut::<ShownWorldFrame>()
        .announce_parsed(snapshot);
}

fn status(app: &App) -> Option<ViewmodelStatus> {
    app.world().resource::<GunStatusShortfall>().0.clone()
}

fn settled(app: &App) -> Option<u32> {
    app.world().resource::<HoldSettledLife>().0
}

fn held_life(app: &App) -> Option<u32> {
    app.world().resource::<HoldLifeStamp>().0
}

fn spawn_root(app: &mut App) -> Entity {
    app.world_mut().spawn(GunMountAnchor).id()
}

fn queued_request(gun_catalog_no: u32, parent_weapon: u32) -> QueuedGunSpawn {
    QueuedGunSpawn {
        gun_index: assets::FpvMeshSlot::from_sequence(0),
        catalog_id: 9,
        gun_catalog_no,
        parent_weapon,
    }
}

fn life_started(client: u32, life: u32) -> SpawnLifeBegun {
    SpawnLifeBegun {
        client,
        life,
        reason: SpawnCause::JustRespawned,
    }
}

fn life_ended(client: u32, life: u32) -> SpawnLifeOver {
    SpawnLifeOver {
        client,
        life,
        cause: DeathCircumstance::LeftAlive,
    }
}

#[test]
fn no_player_clears_everything_and_reports_not_alive() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(4);
    app.world_mut().resource_mut::<HoldLifeStamp>().0 = Some(3);
    publish_empty(&mut app, GameRoundStage::Playing);

    app.update();

    assert_eq!(status(&app), Some(ViewmodelStatus::ResetDead));
    assert_eq!(settled(&app), None);
    assert_eq!(held_life(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn no_player_with_nothing_to_clear_leaves_status_alone() {
    let mut app = test_app();
    publish_empty(&mut app, GameRoundStage::Playing);

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(held_life(&app), None);
}

#[test]
fn intermission_clears_but_keeps_the_life_marker() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(4);
    app.world_mut().resource_mut::<HoldLifeStamp>().0 = Some(3);
    publish_player(&mut app, 7, GameRoundStage::Intermission);

    app.update();

    assert_eq!(status(&app), Some(ViewmodelStatus::ResetDead));
    assert_eq!(settled(&app), None);
    assert_eq!(held_life(&app), Some(3));
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn intermission_in_a_killcam_is_not_a_clear() {
    let mut app = test_app();
    *app.world_mut().resource_mut::<EyeSubject>() = EyeSubject::ChairSlot {
        viewer: 0,
        focus: None,
    };
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(7);
    publish_player(&mut app, 7, GameRoundStage::Intermission);

    app.update();

    // Carried on to the normal path: weapon 7 was already settled, so no change.
    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(7));
}

#[test]
fn empty_hand_settles_on_zero_and_reports_no_weapon() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    publish_player(&mut app, 0, GameRoundStage::Playing);

    app.update();

    assert_eq!(status(&app), Some(ViewmodelStatus::ResetUnarmed));
    assert_eq!(settled(&app), Some(0));
    assert_eq!(
        app.world().resource::<StartupReport>().stepping_lag,
        ViewmodelStatus::ResetUnarmed.summary()
    );
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn empty_hand_already_settled_does_nothing_more() {
    let mut app = test_app();
    publish_player(&mut app, 0, GameRoundStage::Playing);
    app.update();
    app.world_mut().resource_mut::<GunStatusShortfall>().0 = None;

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(0));
}

#[test]
fn queued_request_for_the_held_weapon_is_left_alone() {
    let mut app = test_app();
    app.world_mut().resource_mut::<WaitingFpvSpawn>().0 = Some(queued_request(7, 0));
    let root = spawn_root(&mut app);
    publish_player(&mut app, 7, GameRoundStage::Playing);

    app.update();

    assert!(app.world().resource::<WaitingFpvSpawn>().0.is_some());
    assert_eq!(status(&app), None);
    assert!(app.world().get_entity(root).is_ok());
}

#[test]
fn settled_on_the_held_weapon_with_nothing_live_is_left_alone() {
    let mut app = test_app();
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(7);
    publish_player(&mut app, 7, GameRoundStage::Playing);

    app.update();

    assert_eq!(status(&app), None);
    assert_eq!(settled(&app), Some(7));
}

#[test]
fn missing_catalog_waits_and_names_the_cause() {
    let mut app = test_app();
    publish_player(&mut app, 7, GameRoundStage::Playing);

    app.update();

    assert_eq!(
        status(&app),
        Some(ViewmodelStatus::Blocked(Cause::GunTableAbsent))
    );
    assert_eq!(settled(&app), None);
    assert!(app.world().resource::<WaitingFpvSpawn>().0.is_none());
}

#[test]
fn queued_request_for_another_weapon_waits_for_the_catalog() {
    let mut app = test_app();
    app.world_mut().resource_mut::<WaitingFpvSpawn>().0 = Some(queued_request(3, 0));
    publish_player(&mut app, 7, GameRoundStage::Playing);

    app.update();

    // The stale request is not touched while there is no table to replace it.
    assert!(app.world().resource::<WaitingFpvSpawn>().0.is_some());
    assert_eq!(
        status(&app),
        Some(ViewmodelStatus::Blocked(Cause::GunTableAbsent))
    );
}

#[test]
fn local_life_start_records_the_life_and_drops_old_state() {
    let mut app = test_app();
    let root = spawn_root(&mut app);
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(4);
    publish_player(&mut app, 7, GameRoundStage::Playing);
    app.world_mut().write_message(life_started(0, 12));

    app.update();

    assert_eq!(held_life(&app), Some(12));
    assert_eq!(settled(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn another_clients_life_events_are_ignored() {
    let mut app = test_app();
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(7);
    app.world_mut().resource_mut::<HoldLifeStamp>().0 = Some(2);
    publish_player(&mut app, 7, GameRoundStage::Playing);
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
    app.world_mut().resource_mut::<HoldSettledLife>().0 = Some(4);
    app.world_mut().resource_mut::<HoldLifeStamp>().0 = Some(3);
    publish_player(&mut app, 7, GameRoundStage::Playing);
    app.world_mut().write_message(life_ended(0, 3));

    app.update();

    assert_eq!(held_life(&app), None);
    assert_eq!(settled(&app), None);
    assert!(app.world().get_entity(root).is_err());
}

#[test]
fn end_and_start_in_one_frame_keeps_the_new_life() {
    let mut app = test_app();
    app.world_mut().resource_mut::<HoldLifeStamp>().0 = Some(3);
    publish_player(&mut app, 7, GameRoundStage::Playing);
    app.world_mut().write_message(life_ended(0, 3));
    app.world_mut().write_message(life_started(0, 4));

    app.update();

    assert_eq!(held_life(&app), Some(4));
}

#[test]
fn probe_follows_a_changed_status() {
    let mut app = App::new();
    app.init_resource::<GunStatusShortfall>()
        .init_resource::<StartupReport>()
        .add_systems(bevy::app::Update, mirror_fpv_status_probe);
    app.update();
    // Nothing published yet: the report keeps its starting text.
    let start = app.world().resource::<StartupReport>().stepping_lag;

    app.world_mut().resource_mut::<GunStatusShortfall>().0 = Some(ViewmodelStatus::Queued);
    app.update();

    assert_ne!(app.world().resource::<StartupReport>().stepping_lag, start);
    assert_eq!(
        app.world().resource::<StartupReport>().stepping_lag,
        ViewmodelStatus::Queued.summary()
    );
}

#[test]
fn probe_without_a_status_writes_nothing() {
    let mut app_val = App::new();
    app_val.init_resource::<GunStatusShortfall>()
        .init_resource::<StartupReport>()
        .add_systems(bevy::app::Update, mirror_fpv_status_probe);
    let before = app_val.world().resource::<StartupReport>().stepping_lag;

    app_val.update();
    app_val.world_mut().resource_mut::<GunStatusShortfall>().0 = None;
    app_val.update();

    assert_eq!(app_val.world().resource::<StartupReport>().stepping_lag, before);
}

#[test]
fn registering_adds_the_life_messages() {
    let mut app_in = App::new();
    add_held_sync_systems(&mut app_in);
    assert!(app_in.world().contains_resource::<Messages<SpawnLifeBegun>>());
    assert!(app_in.world().contains_resource::<Messages<SpawnLifeOver>>());
}
