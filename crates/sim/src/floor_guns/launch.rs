//! Putting a gun on the ground: the throw a dropped gun gets, the cap on how many may lie about, and the entity
//! that carries one.

use anim_t6::bg_rand;
use entity_t6::Trajectory;
use entity_t6::flight::path_kind::{FALLING, AT_REST};
use game_math::euler_basis;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState};
use weapon_t6::ammo_table::player_weapons_find_slot;

use super::LootOnGround;
use super::inventory::{INVENTORY_PRIMARY, Rounds, held_rounds, take_weapon};
use crate::shot_trace::BODY_BOX_LOWER;
use crate::ent_links::clipped_shift;
use crate::step_view::StepWorldView;
use crate::ent_slots::{EntityBehaviorClass, loot_state_at};
use crate::sim_parts::{SeatNo, StepNo};

/// At most this many dropped guns lie in the world; one more evicts the oldest.
const MAX_DROPPED: usize = 16;

/// A dropped gun leaves the player at this speed along their yaw, plus the random kicks below.
const THROW_FORWARD: f32 = 10.0;
const THROW_UP_BASE: f32 = 10.0;
const THROW_UP_SPREAD: f32 = 5.0;
const THROW_SIDE_SPREAD: f32 = 100.0;

/// The next random number in [0, 1), from the shared generator.
pub fn random_unit(seed: &mut u32) -> f32 {
    bg_rand(seed) as f32 * (1.0 / 32768.0)
}

/// The next random number in [-1, 1).
fn random_centered(seed: &mut u32) -> f32 {
    random_unit(seed) * 2.0 - 1.0
}

/// A gun dropped by a player facing `yaw` leaves at this velocity.
fn throw_velocity(yaw: f32, rng_seed: &mut u32) -> [f32; 3] {
    let (ahead, _, _) = euler_basis([0.0, yaw, 0.0]);
    let mut velocity = ahead.map(|component| component * THROW_FORWARD);
    velocity[2] += random_centered(rng_seed) * THROW_UP_SPREAD + THROW_UP_BASE;
    velocity[0] += random_centered(rng_seed) * THROW_SIDE_SPREAD;
    velocity[1] += random_centered(rng_seed) * THROW_SIDE_SPREAD;
    velocity
}

/// Where on the player a dropped gun appears: half way up their collision box, which follows their stance (35
/// standing, 25 crouched or in last stand, 15 prone).
fn release_height(ps: &AvatarState) -> f32 {
    (super::stance_maxs_z(ps) - BODY_BOX_LOWER[2]) * 0.5
}

/// A dropped copy of `weapon` is only worth making when the player carries it as a primary and still has something
/// in a magazine (a riot shield has no rounds and always may go).
fn may_drop(world: &StepWorldView, ps: &AvatarState, gun: u32) -> bool {
    if gun == 0 || player_weapons_find_slot(&ps.weapons, gun as i32) < 0 {
        return false;
    }
    let script_name = world.weapon_script_label(gun);
    if script_name.contains("ac130") {
        return false;
    }
    let is_primary =
        world.damage_profile_of(gun).is_some_and(|facts| facts.carry_slot_kind == INVENTORY_PRIMARY);
    if !is_primary {
        return false;
    }
    if script_name.contains("riotshield") {
        return true;
    }
    let held = held_rounds(world, ps, gun);
    held.right != 0 || held.left != 0
}

/// Lays a gun in the world as its own entity and returns its number, or `NO_ENTITY_SLOT` when no entity is free. If
/// the cap on dropped guns is reached the oldest one is removed first.
pub(crate) fn push_dropped_item(
    world: &mut StepWorldView,
    gun: u32,
    spot: [f32; 3],
    start: Trajectory,
    facing: Trajectory,
    owner: i32,
    right_clip: i32,
    left_clip: i32,
    reserve: i32,
    in_flight: bool,
    bag: bool,
) -> i32 {
    if world.dropped_item_total() >= MAX_DROPPED {
        let oldest = world.ordered_loot_slots()[0];
        let evicted = world.take_loot_at_slot(oldest).expect("oldest dropped item vanished");
        let freed = evicted.state.slot_index;
        world.free_dynamic_entity_number(freed);
    }
    let allocated = world
        .allocate_dynamic_entity(EntityBehaviorClass::Item)
        .inspect_err(|error| diag::warn!(Sim, "dropped item not spawned: {error:?}"))
        .ok();
    let Some(entity) = allocated else { return NO_ENTITY_SLOT };
    let slot = entity.slot();
    let dropped = LootOnGround {
        state: loot_state_at(slot, gun, start, facing, owner),
        origin: spot,
        dropping: in_flight,
        right_mag: right_clip,
        left_mag: left_clip,
        spare: reserve,
        stock2: 0,
        scavenge: bag,
        owner_clear_ms: 0,
    };
    world.add_loot_row(dropped);
    slot
}

/// Throws a gun (or a scavenger bag) out of `ps`, falling under gravity. Returns the new item's number.
pub(crate) fn launch_dropped_from_ps(
    world: &mut StepWorldView,
    tick: StepNo,
    pstate: &AvatarState,
    owner: i32,
    gun: u32,
    right_clip: i32,
    left_clip: i32,
    reserve: i32,
    bag: bool,
) -> i32 {
    let mut rng = world.anim_event_seed();
    // Only the whole units of the throw's speed survive.
    let thrown = clipped_shift(throw_velocity(pstate.look_angles[1], &mut rng));
    world.set_anim_event_seed(rng);

    let spawn_at = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + release_height(pstate)];
    let flight = Trajectory {
        tr_type: FALLING,
        began_at: crate::bodies::level_time_ms(tick),
        span_ms: 0,
        rate: thrown,
        base_point: spawn_at,
    };
    let spin_path = Trajectory { tr_type: AT_REST, base_point: [0.0, pstate.look_angles[1], 0.0], ..Trajectory::default() };
    push_dropped_item(world, gun, spawn_at, flight, spin_path, owner, right_clip, left_clip, reserve, true, bag)
}

/// The player lets go of `weapon`: it leaves their hands with the rounds it holds. `None` when they may not drop it
/// or no entity is free.
pub(crate) fn drop_weapon(world: &mut StepWorldView, tick: StepNo, player: SeatNo, weapon: u32) -> Option<i32> {
    let ps = world.avatar(player).copied()?;
    if !may_drop(world, &ps, weapon) {
        return None;
    }
    let Rounds { right, left, reserve } = held_rounds(world, &ps, weapon);
    let slot_no =
        launch_dropped_from_ps(world, tick, &ps, player.0 as i32, weapon, right, left, reserve, false);
    if slot_no == NO_ENTITY_SLOT {
        return None;
    }
    if let Some(ps) = world.occupant_mut(player) {
        take_weapon(ps, weapon);
    }
    Some(slot_no)
}

/// A scavenger bag falls out of the player; nothing is taken from them.
pub(crate) fn drop_scavenger_item(world: &mut StepWorldView, tick: StepNo, player: SeatNo, weapon: u32) -> Option<i32> {
    let ps = world.avatar(player).copied()?;
    let number = launch_dropped_from_ps(world, tick, &ps, player.0 as i32, weapon, 0, 0, 0, true);
    (number != NO_ENTITY_SLOT).then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use movement_t6::bridge::eye_level;

    #[test]
    fn a_dropped_gun_appears_half_way_up_the_players_box_in_each_stance() {
        let mut ps = AvatarState::ZERO;
        let mut at = |target| {
            ps.eye_height_goal = target;
            release_height(&ps)
        };
        assert_eq!(at(eye_level::UPRIGHT), 35.0);
        assert_eq!(at(eye_level::DUCK_BUTTON_BIT), 25.0);
        assert_eq!(at(eye_level::DOWNED_GRACE), 25.0);
        assert_eq!(at(eye_level::LYING_FLAT), 15.0);
    }

    #[test]
    fn random_numbers_stay_in_their_ranges_and_follow_the_seed() {
        let mut seed = 12345;
        let mut twin = 12345;
        for _ in 0..2000 {
            let unit = random_unit(&mut seed);
            assert!((0.0..1.0).contains(&unit), "{unit}");
            assert_eq!(unit, random_unit(&mut twin));
            let centered = random_centered(&mut seed);
            assert!((-1.0..1.0).contains(&centered), "{centered}");
            assert_eq!(centered, random_centered(&mut twin));
        }
        assert_eq!(seed, twin);
    }

    #[test]
    fn the_throw_draws_up_then_east_then_north_and_leaves_along_the_yaw() {
        for yaw in [0.0, 90.0, 215.0] {
            let start = 777;
            let mut seed = start;
            let velocity = throw_velocity(yaw, &mut seed);

            let mut replay = start;
            let (ahead, _, _) = euler_basis([0.0, yaw, 0.0]);
            let up = random_centered(&mut replay) * THROW_UP_SPREAD + THROW_UP_BASE;
            let east = random_centered(&mut replay) * THROW_SIDE_SPREAD;
            let north = random_centered(&mut replay) * THROW_SIDE_SPREAD;
            let expected = [ahead[0] * THROW_FORWARD + east, ahead[1] * THROW_FORWARD + north, ahead[2] * THROW_FORWARD + up];
            for (axis, (got, want)) in velocity.iter().zip(expected).enumerate() {
                assert!((got - want).abs() < 1e-3, "yaw {yaw} axis {axis}");
            }
            assert_eq!(seed, replay, "exactly three draws per throw");
        }
    }

    #[test]
    fn the_throw_never_leaves_faster_than_its_spreads_allow() {
        let mut seed = 99;
        for _ in 0..500 {
            let velocity = throw_velocity(40.0, &mut seed);
            assert!(velocity[2] >= THROW_UP_BASE - THROW_UP_SPREAD && velocity[2] < THROW_UP_BASE + THROW_UP_SPREAD);
            for horizontal in &velocity[..2] {
                assert!(horizontal.abs() <= THROW_SIDE_SPREAD + THROW_FORWARD);
            }
        }
    }
}
