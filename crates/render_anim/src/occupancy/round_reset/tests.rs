#![cfg(test)]
//! Behaviour pins for the per-match animation reset.

use bevy::prelude::*;
use frame::{OwnLevelLoadTicket, GameSessionArmed, GameSessionStamp, GameSessionClosed, ShutdownCause, LevelGeneration};

use super::*;
use crate::motion::fpv_prepared::PreparedFpv;
use crate::motion::model_materials::PreparedModelMaterials;
use crate::motion::scene_feed::SceneBonePoseSet;
use crate::occupancy::shard_piece::{PropPhysicsShape, DebrisPhysLevel};
use crate::occupancy::gun_present::{
    HoldLifeStamp, HoldSettledLife, GunMountAnchor, FpvShownCursor, OwnSpawnPrimed,
    WaitingFpvSpawn, QueuedGunSpawn, ActiveGunMeshes,
};

fn test_app() -> App {
    let mut application = App::new();
    application.add_message::<GameSessionArmed>()
        .add_message::<GameSessionClosed>()
        .init_resource::<OwnSpawnPrimed>()
        .init_resource::<FpvShownCursor>()
        .init_resource::<WaitingFpvSpawn>()
        .init_resource::<ActiveGunMeshes>()
        .init_resource::<PreparedFpv>()
        .init_resource::<PreparedModelMaterials>()
        .init_resource::<HoldSettledLife>()
        .init_resource::<HoldLifeStamp>()
        .init_resource::<SceneBonePoseSet>();
    register_match_reset_steps(&mut application);
    application
}

fn dirty(app: &mut App) -> Entity {
    let world = app.world_mut();
    world.resource_mut::<OwnSpawnPrimed>().0 = true;
    world.resource_mut::<FpvShownCursor>().0.latest_step_seen = Some(41);
    world.resource_mut::<WaitingFpvSpawn>().0 = Some(QueuedGunSpawn {
        gun_index: assets::FpvMeshSlot::from_sequence(0),
        catalog_id: 1,
        gun_catalog_no: 2,
        parent_weapon: 0,
    });
    world.resource_mut::<HoldSettledLife>().0 = Some(6);
    world.resource_mut::<HoldLifeStamp>().0 = Some(8);
    world.spawn(GunMountAnchor).id()
}

fn installed() -> GameSessionArmed {
    GameSessionArmed {
        ask_number: 1,
        decrypt_secret: OwnLevelLoadTicket::default(),
        zone: "mp_test".into(),
        spawns_made: 0,
    }
}

fn torn_down() -> GameSessionClosed {
    GameSessionClosed {
        reason: ShutdownCause::RoundOver,
        level_generation: LevelGeneration::default(),
        game_session_key: GameSessionStamp::default(),
        game_generation: 0,
    }
}

fn assert_clean(app: &App, root: Entity) {
    let world = app.world();
    assert!(!world.resource::<OwnSpawnPrimed>().0);
    assert_eq!(world.resource::<FpvShownCursor>().0.latest_step_seen, None);
    assert!(world.resource::<WaitingFpvSpawn>().0.is_none());
    assert!(world.resource::<ActiveGunMeshes>().0.is_none());
    assert_eq!(world.resource::<HoldSettledLife>().0, None);
    assert_eq!(world.resource::<HoldLifeStamp>().0, None);
    assert!(world.get_entity(root).is_err());
}

#[test]
fn nothing_happens_without_an_install_or_teardown() {
    let mut app = test_app();
    let root = dirty(&mut app);

    app.update();

    let world = app.world();
    assert!(world.resource::<OwnSpawnPrimed>().0);
    assert_eq!(world.resource::<FpvShownCursor>().0.latest_step_seen, Some(41));
    assert!(world.resource::<WaitingFpvSpawn>().0.is_some());
    assert_eq!(world.resource::<HoldSettledLife>().0, Some(6));
    assert_eq!(world.resource::<HoldLifeStamp>().0, Some(8));
    assert!(world.get_entity(root).is_ok());
}

#[test]
fn an_install_wipes_the_first_person_state() {
    let mut app = test_app();
    let root = dirty(&mut app);
    app.world_mut().write_message(installed());

    app.update();

    assert_clean(&app, root);
}

#[test]
fn a_teardown_wipes_the_first_person_state() {
    let mut app = test_app();
    let root = dirty(&mut app);
    app.world_mut().write_message(torn_down());

    app.update();

    assert_clean(&app, root);
}

#[test]
fn only_a_teardown_resets_the_piece_physics() {
    let mut app = test_app();
    app.world_mut().write_message(installed());
    app.update();
    assert!(!app.world().contains_resource::<PropPhysicsShape>());
    assert!(!app.world().contains_resource::<DebrisPhysLevel>());

    app.world_mut().write_message(torn_down());
    app.update();
    assert!(app.world().contains_resource::<PropPhysicsShape>());
    assert!(app.world().contains_resource::<DebrisPhysLevel>());
    assert!(app.world().resource::<PropPhysicsShape>().0.is_none());
}
