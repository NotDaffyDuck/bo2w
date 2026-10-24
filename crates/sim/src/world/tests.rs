//! Tests of the world's parts that need no map: the roster, the clock, the blockers, snapshots both ways, and the
//! lag-compensation fallbacks.

use entity_t6::EntityState;

use super::*;
use crate::bullet_collision::{CurrentAuthorityReason, HistorySampleVerdict, ShotSampleProvenance};
use crate::identities::{MatchPhase, ProjectileId};
use crate::snapshot::Snapshot;

fn two_client_world() -> SimState {
    let mut world = SimState::new();
    world.client_meta_mut(ClientId(2));
    world.client_meta_mut(ClientId(7));
    world
}

fn snapshot_of(world: &SimState, tick: u32) -> Snapshot {
    world.snapshot_with_dynamic_rows(Tick(tick), Vec::new(), Vec::new(), Vec::new(), Vec::new())
}

// ---- player-state helpers -------------------------------------------------------------------------------------

#[test]
fn weapon_names_with_akimbo_are_dual_wield() {
    assert!(gsc_give_weapon_is_akimbo("kard_akimbo_mp"));
    assert!(!gsc_give_weapon_is_akimbo("kard_mp"));
}

#[test]
fn inventory_takes_a_weapon_once_and_never_the_empty_one() {
    let mut ps = blank_player_state();
    inventory_add_weapon(&mut ps, 0, false);
    assert!(ps.weapons.iter().all(|slot| *slot == 0));

    inventory_add_weapon(&mut ps, 12, false);
    inventory_add_weapon(&mut ps, 12, false);
    inventory_add_weapon(&mut ps, 30, false);
    assert_eq!(ps.weapons.iter().filter(|slot| **slot == 12).count(), 1);
    assert_eq!(ps.weapons.iter().position(|slot| *slot == 12), Some(0));
    assert_eq!(ps.weapons.iter().position(|slot| *slot == 30), Some(1));
}

#[test]
fn giving_the_empty_weapon_empties_the_hands() {
    let mut ps = blank_player_state();
    give_weapon_to_ps_akimbo(&mut ps, 12, false);
    assert_eq!((ps.weapon, ps.weapon_primary), (12, 12));

    give_weapon_to_ps_akimbo(&mut ps, 0, false);
    assert_eq!((ps.weapon, ps.weapon_primary, ps.last_weapon_hand), (0, 0, 0));
}

#[test]
fn a_spawned_player_stands_where_asked_with_full_health() {
    let ps = spawn_player_state([10.0, 20.0, 30.0], [0.0, 90.0, 0.0]);
    assert_eq!(ps.origin, [10.0, 20.0, 30.0]);
    assert_eq!(ps.viewangles, [0.0, 90.0, 0.0]);
    assert_eq!((ps.health, ps.max_health), (100, 100));
    assert_eq!(ps.corpse_index, -1);
}

// ---- roster and clock ------------------------------------------------------------------------------------------

#[test]
fn a_new_client_row_is_blank_and_listed_in_order() {
    let mut world = SimState::new();
    for id in [9, 3, 5] {
        world.client_meta_mut(ClientId(id));
    }
    assert_eq!(world.client_count(), 3);
    assert_eq!(world.client_ids_sorted(), [ClientId(3), ClientId(5), ClientId(9)]);
    assert!(world.client_meta(ClientId(4)).is_none());
    assert_eq!(world.packed_client_name(ClientId(4)), [0; 16]);
}

#[test]
fn the_last_command_a_client_sent_is_remembered_per_client() {
    let mut world = SimState::new();
    world.set_old_cmd(ClientId(1), 0b101, [1, 2, 3]);
    world.set_old_cmd_move(ClientId(1), [127, -127]);
    world.set_old_buttons(ClientId(2), 8);

    assert_eq!(world.old_buttons(ClientId(1)), Some(0b101));
    assert_eq!(world.old_cmd_angles(ClientId(1)), Some([1, 2, 3]));
    assert_eq!(world.old_cmd_move(ClientId(1)), Some([127, -127]));
    assert_eq!(world.old_buttons(ClientId(2)), Some(8));
    assert_eq!(world.old_cmd_angles(ClientId(2)), None);
}

#[test]
fn ids_count_up_and_a_projectile_id_never_comes_out_as_zero() {
    let mut world = SimState::new();
    let first = world.alloc_shot_id();
    assert_eq!(world.alloc_shot_id(), first.next());

    world.serials.projectile = ProjectileId(u32::MAX);
    assert_eq!(world.allocate_projectile_id(), ProjectileId(u32::MAX));
    assert_eq!(world.allocate_projectile_id(), ProjectileId(1));
}

#[test]
fn a_new_world_waits_in_warmup_and_is_not_running() {
    let world = SimState::new();
    assert!(!world.is_running());
    assert_eq!(world.phase(), MatchPhase::Warmup);
    assert_eq!(world.match_elapsed_ms(), 0);
    assert!(world.publishes_snapshot());
}

#[test]
fn a_prediction_world_stops_publishing() {
    let mut world = SimState::new();
    world.suppress_snapshot_publish();
    assert!(!world.publishes_snapshot());
}

// ---- blockers --------------------------------------------------------------------------------------------------

#[test]
fn a_mantle_blocker_ends_where_its_owner_said_and_goes_after_its_time() {
    let mut world = SimState::new();
    let blocker = |owner, ends, until| MantleBlocker { owner, origin: [0.0; 3], ends, until };
    world.add_mantle_blocker(blocker(4, 1000, 1500));
    world.add_mantle_blocker(blocker(5, 1200, 1700));
    world.add_mantle_blocker(blocker(4, 2000, 2500));

    assert_eq!(world.mantle_end_time(4), Some(2000), "the newest climb of the owner answers");
    assert_eq!(world.mantle_end_time(9), None);

    world.prune_mantle_blockers(1500);
    assert_eq!(world.mantle_blockers().len(), 2, "a blocker whose time is exactly up is gone");
    world.prune_mantle_blockers(10_000);
    assert!(world.mantle_blockers().is_empty());
}

#[test]
fn tactical_insertion_boxes_are_replaced_whole() {
    let mut world = SimState::new();
    world.set_tac_insert_blockers(vec![([0.0; 3], [1.0; 3])]);
    world.set_tac_insert_blockers(vec![([2.0; 3], [3.0; 3]), ([4.0; 3], [5.0; 3])]);
    assert_eq!(world.tac_insert_blockers().len(), 2);
    assert_eq!(world.tac_insert_blockers()[0].0, [2.0; 3]);
}

// ---- snapshots -------------------------------------------------------------------------------------------------

#[test]
fn a_snapshot_lists_every_client_and_the_clock() {
    let mut world = two_client_world();
    world.clock.elapsed_ms = 4321;
    let snapshot = snapshot_of(&world, 12);

    assert_eq!(snapshot.tick, Tick(12));
    let listed: Vec<ClientId> = snapshot.meta.clients.iter().map(|(id, _)| *id).collect();
    assert_eq!(listed, [ClientId(2), ClientId(7)]);
    assert_eq!(snapshot.meta.match_elapsed_ms, 4321);
    assert_eq!(snapshot.meta.rng, world.rng_debug_meta());
}

#[test]
fn an_empty_world_snapshots_without_entities() {
    let snapshot = snapshot_of(&SimState::new(), 1);
    assert!(snapshot.meta.entities.is_empty());
    assert!(snapshot.meta.item_ammo.is_empty());
    assert!(snapshot.meta.script_movers.is_empty());
}

#[test]
fn adopting_a_snapshot_brings_the_clients_and_the_clock_over() {
    let mut authority = two_client_world();
    authority.clock.elapsed_ms = 777;
    let snapshot = snapshot_of(&authority, 3);

    let mut copy = SimState::new();
    copy.client_meta_mut(ClientId(40));
    let (report, projectiles, movers, items) = copy.adopt_snapshot_state(&snapshot);

    assert_eq!(report.clients, 2);
    assert!(!report.content_mismatch);
    assert_eq!(report.dropped, 0, "the one client that was not in the snapshot was replaced, not lost");
    assert!(projectiles.is_empty() && movers.is_empty() && items.is_empty());
    assert_eq!(copy.client_ids_sorted(), [ClientId(2), ClientId(7)]);
    assert_eq!(copy.match_elapsed_ms(), 777);
    assert!(copy.is_running());
    assert_eq!(copy.rng_debug_meta(), authority.rng_debug_meta());
}

#[test]
fn a_snapshot_from_another_seed_is_reported_as_a_content_mismatch() {
    let authority = two_client_world();
    let mut copy = SimState::new();
    copy.dice.seed = 99;
    let (report, ..) = copy.adopt_snapshot_state(&snapshot_of(&authority, 1));
    assert!(report.content_mismatch);
}

#[test]
fn a_prediction_world_keeps_only_its_own_client() {
    let authority = two_client_world();
    let snapshot = snapshot_of(&authority, 8);

    let mut prediction = SimState::new();
    let (report, ..) = prediction.adopt_prediction_snapshot_state(&snapshot, ClientId(7));
    assert_eq!(report.clients, 1);
    assert_eq!(prediction.client_ids_sorted(), [ClientId(7)]);
    assert!(prediction.prediction_remote_bodies().is_empty());
}

#[test]
fn adopting_forgets_the_last_commands() {
    let authority = two_client_world();
    let mut copy = SimState::new();
    copy.set_old_cmd(ClientId(2), 1, [0; 3]);
    copy.set_old_cmd_move(ClientId(2), [1, 1]);
    copy.adopt_snapshot_state(&snapshot_of(&authority, 1));
    assert_eq!(copy.old_buttons(ClientId(2)), None);
    assert_eq!(copy.old_cmd_angles(ClientId(2)), None);
    assert_eq!(copy.old_cmd_move(ClientId(2)), None);
}

#[test]
#[should_panic(expected = "ET_ITEM does not occupy an Item kernel slot")]
fn a_snapshot_with_an_item_the_kernel_does_not_know_is_refused() {
    let mut snapshot = snapshot_of(&SimState::new(), 1);
    snapshot.meta.entities.push(EntityState { number: 40, e_type: entity_t6::et::ITEM, ..EntityState::default() });
    SimState::new().adopt_snapshot_state(&snapshot);
}

#[test]
#[should_panic(expected = "ET_MISSILE does not occupy a Missile kernel slot")]
fn a_snapshot_with_a_missile_the_kernel_does_not_know_is_refused() {
    let mut snapshot = snapshot_of(&SimState::new(), 1);
    snapshot.meta.entities.push(EntityState { number: 41, e_type: entity_t6::et::MISSILE, ..EntityState::default() });
    SimState::new().adopt_snapshot_state(&snapshot);
}

#[test]
fn a_prediction_world_takes_on_the_authoritys_map_and_seed() {
    let mut authority = SimState::new();
    authority.dice.seed = 0xBEEF;
    authority.refresh_digest();

    let mut prediction = SimState::new();
    prediction.initialize_prediction_from(&authority);
    assert_eq!(prediction.root_seed(), 0xBEEF);
    assert_eq!(prediction.content_digest(), authority.content_digest());
}

// ---- lag compensation ------------------------------------------------------------------------------------------

#[test]
fn a_shot_without_a_claim_is_judged_against_the_present() {
    let world = two_client_world();
    let query = world.lagcomp_query_for(ClientId(2), Tick(10), |_| None);

    let expected = HistorySampleVerdict::CurrentAuthority { tick: Tick(10), reason: CurrentAuthorityReason::NoSampleClaim };
    assert_eq!(query.players.verdict, expected);
    assert_eq!(query.entities.verdict, expected);
    assert!(query.players.poses.is_empty());
}

#[test]
fn a_command_without_a_claim_picks_the_no_claim_sample() {
    let mut world = SimState::new();
    world.set_lagcomp_commands([((ClientId(1), 100), ShotSampleProvenance::NO_CLAIM)]);
    world.select_lagcomp_command(ClientId(1), 100);
    assert_eq!(world.lagcomp_sample(ClientId(1)), Some(ShotSampleProvenance::NO_CLAIM));

    world.select_lagcomp_command(ClientId(3), 555);
    assert_eq!(world.lagcomp_sample(ClientId(3)), Some(ShotSampleProvenance::NO_CLAIM));
}

#[test]
fn the_lunge_target_is_what_the_client_last_saw() {
    let mut world = SimState::new();
    world.set_lunge_seen([(ClientId(1), [1.0, 2.0, 3.0], [0.0, 0.0, 1.0])]);
    assert_eq!(world.lunge_seen(ClientId(1)), Some(([1.0, 2.0, 3.0], [0.0, 0.0, 1.0])));
    world.set_lunge_seen([]);
    assert_eq!(world.lunge_seen(ClientId(1)), None);
}

// ---- census ----------------------------------------------------------------------------------------------------

#[test]
fn the_hitvol_dump_has_one_blank_row_before_any_body_is_recorded() {
    let rows = SimState::new().hitvol_dump(|_| None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].client, None);
    assert_eq!(rows[0].geom, "none");
    assert_eq!(rows[0].bone_count, 0);
}
