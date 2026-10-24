//! Behaviour pins for the per-match animation reset.

use bevy::prelude::*;
use frame::{LocalLoadKey, MatchInstalled, MatchKey, MatchTornDown, TeardownReason, WorldGeneration};

use super::*;
use crate::anim::fpv_prepared::PreparedFpv;
use crate::anim::model_materials::PreparedModelMaterials;
use crate::anim::scene_submission::AnimDObjSceneSkels;
use crate::occupancy::dyn_ent::{DynEntPhysClip, DynEntPhysWorld};
use crate::occupancy::fpv_present::{
    FpvHeldLife, FpvHeldSettled, FpvPlacementRoot, FpvPresentCursor, LocalSpawnArmed,
    PendingFpvSpawn, PendingFpvSpawnRequest, SessionViewmodel,
};

fn test_app() -> App {
    let mut app = App::new();
    app.add_message::<MatchInstalled>()
        .add_message::<MatchTornDown>()
        .init_resource::<LocalSpawnArmed>()
        .init_resource::<FpvPresentCursor>()
        .init_resource::<PendingFpvSpawn>()
        .init_resource::<SessionViewmodel>()
        .init_resource::<PreparedFpv>()
        .init_resource::<PreparedModelMaterials>()
        .init_resource::<FpvHeldSettled>()
        .init_resource::<FpvHeldLife>()
        .init_resource::<AnimDObjSceneSkels>();
    register_match_reset_systems(&mut app);
    app
}

fn dirty(app: &mut App) -> Entity {
    let world = app.world_mut();
    world.resource_mut::<LocalSpawnArmed>().0 = true;
    world.resource_mut::<FpvPresentCursor>().0.last_tick = Some(41);
    world.resource_mut::<PendingFpvSpawn>().0 = Some(PendingFpvSpawnRequest {
        gun_index: assets::FpvMeshIndex::from_order(0),
        catalog_id: 1,
        weapon_id: 2,
        parent_weapon: 0,
    });
    world.resource_mut::<FpvHeldSettled>().0 = Some(6);
    world.resource_mut::<FpvHeldLife>().0 = Some(8);
    world.spawn(FpvPlacementRoot).id()
}

fn installed() -> MatchInstalled {
    MatchInstalled {
        request_id: 1,
        load_key: LocalLoadKey::default(),
        zone: "mp_test".into(),
        spawn_count: 0,
    }
}

fn torn_down() -> MatchTornDown {
    MatchTornDown {
        reason: TeardownReason::MatchEnded,
        world_generation: WorldGeneration::default(),
        match_key: MatchKey::default(),
        match_epoch: 0,
    }
}

fn assert_clean(app: &App, root: Entity) {
    let world = app.world();
    assert!(!world.resource::<LocalSpawnArmed>().0);
    assert_eq!(world.resource::<FpvPresentCursor>().0.last_tick, None);
    assert!(world.resource::<PendingFpvSpawn>().0.is_none());
    assert!(world.resource::<SessionViewmodel>().0.is_none());
    assert_eq!(world.resource::<FpvHeldSettled>().0, None);
    assert_eq!(world.resource::<FpvHeldLife>().0, None);
    assert!(world.get_entity(root).is_err());
}

#[test]
fn nothing_happens_without_an_install_or_teardown() {
    let mut app = test_app();
    let root = dirty(&mut app);

    app.update();

    let world = app.world();
    assert!(world.resource::<LocalSpawnArmed>().0);
    assert_eq!(world.resource::<FpvPresentCursor>().0.last_tick, Some(41));
    assert!(world.resource::<PendingFpvSpawn>().0.is_some());
    assert_eq!(world.resource::<FpvHeldSettled>().0, Some(6));
    assert_eq!(world.resource::<FpvHeldLife>().0, Some(8));
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
    assert!(!app.world().contains_resource::<DynEntPhysClip>());
    assert!(!app.world().contains_resource::<DynEntPhysWorld>());

    app.world_mut().write_message(torn_down());
    app.update();
    assert!(app.world().contains_resource::<DynEntPhysClip>());
    assert!(app.world().contains_resource::<DynEntPhysWorld>());
    assert!(app.world().resource::<DynEntPhysClip>().0.is_none());
}
