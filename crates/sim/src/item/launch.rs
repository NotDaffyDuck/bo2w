//! Putting a gun on the ground: the throw a dropped gun gets, the cap on how many may lie about, and the entity
//! that carries one.

use anim_t6::bg_rand;
use entity_t6::Trajectory;
use entity_t6::trajectory::tr_type::{GRAVITY, STATIONARY};
use game_math::angle_vectors;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};
use weapon_t6::ammo_table::player_weapons_find_slot;

use super::DroppedItem;
use super::inventory::{INVENTORY_PRIMARY, Rounds, held_rounds, take_weapon};
use crate::bullet_collision::PLAYER_MINS;
use crate::entity_local::truncated_tr_delta;
use crate::frame::FrameWorld;
use crate::gentity::{EntityRunKind, init_item_state};
use crate::world::{ClientId, Tick};

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
fn throw_velocity(yaw: f32, seed: &mut u32) -> [f32; 3] {
    let (ahead, _, _) = angle_vectors([0.0, yaw, 0.0]);
    let mut velocity = ahead.map(|component| component * THROW_FORWARD);
    velocity[2] += random_centered(seed) * THROW_UP_SPREAD + THROW_UP_BASE;
    velocity[0] += random_centered(seed) * THROW_SIDE_SPREAD;
    velocity[1] += random_centered(seed) * THROW_SIDE_SPREAD;
    velocity
}

/// Where on the player a dropped gun appears: half way up their collision box, which follows their stance (35
/// standing, 25 crouched or in last stand, 15 prone).
fn release_height(ps: &PlayerState) -> f32 {
    (super::stance_maxs_z(ps) - PLAYER_MINS[2]) * 0.5
}

/// A dropped copy of `weapon` is only worth making when the player carries it as a primary and still has something
/// in a magazine (a riot shield has no rounds and always may go).
fn may_drop(world: &FrameWorld, ps: &PlayerState, weapon: u32) -> bool {
    if weapon == 0 || player_weapons_find_slot(&ps.weapons, weapon as i32) < 0 {
        return false;
    }
    let name = world.weapon_script_name(weapon);
    if name.contains("ac130") {
        return false;
    }
    let is_primary =
        world.combat_facts_for(weapon).is_some_and(|facts| facts.inventory_type == INVENTORY_PRIMARY);
    if !is_primary {
        return false;
    }
    if name.contains("riotshield") {
        return true;
    }
    let held = held_rounds(world, ps, weapon);
    held.right != 0 || held.left != 0
}

/// Lays a gun in the world as its own entity and returns its number, or `ENTITYNUM_NONE` when no entity is free. If
/// the cap on dropped guns is reached the oldest one is removed first.
pub(crate) fn push_dropped_item(
    world: &mut FrameWorld,
    weapon: u32,
    origin: [f32; 3],
    pos: Trajectory,
    apos: Trajectory,
    owner: i32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
    falling: bool,
    scavenger: bool,
) -> i32 {
    if world.dropped_item_count() >= MAX_DROPPED {
        let oldest = world.dropped_item_numbers_sorted()[0];
        let evicted = world.remove_dropped_item_by_number(oldest).expect("oldest dropped item vanished");
        world.free_dynamic_entity_number(evicted.state.number);
    }
    let number = match world.allocate_dynamic_entity(EntityRunKind::Item) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "dropped item not spawned: {error:?}");
            return ENTITYNUM_NONE;
        }
    };
    world.push_dropped_item(DroppedItem {
        state: init_item_state(number, weapon, pos, apos, owner),
        origin,
        falling,
        clip_r,
        clip_l,
        stock,
        stock2: 0,
        scavenger,
        owner_clear_ms: 0,
    });
    number
}

/// Throws a gun (or a scavenger bag) out of `ps`, falling under gravity. Returns the new item's number.
pub(crate) fn launch_dropped_from_ps(
    world: &mut FrameWorld,
    tick: Tick,
    ps: &PlayerState,
    owner: i32,
    weapon: u32,
    clip_r: i32,
    clip_l: i32,
    stock: i32,
    scavenger: bool,
) -> i32 {
    let mut seed = world.anim_event_seed();
    // Only the whole units of the throw's speed survive.
    let velocity = truncated_tr_delta(throw_velocity(ps.viewangles[1], &mut seed));
    world.set_anim_event_seed(seed);

    let origin = [ps.origin[0], ps.origin[1], ps.origin[2] + release_height(ps)];
    let pos = Trajectory {
        tr_type: GRAVITY,
        tr_time: crate::corpse::level_time_ms(tick),
        tr_duration: 0,
        tr_delta: velocity,
        tr_base: origin,
    };
    let apos = Trajectory { tr_type: STATIONARY, tr_base: [0.0, ps.viewangles[1], 0.0], ..Trajectory::default() };
    push_dropped_item(world, weapon, origin, pos, apos, owner, clip_r, clip_l, stock, true, scavenger)
}

/// The player lets go of `weapon`: it leaves their hands with the rounds it holds. `None` when they may not drop it
/// or no entity is free.
pub(crate) fn drop_weapon(world: &mut FrameWorld, tick: Tick, player: ClientId, weapon: u32) -> Option<i32> {
    let ps = world.player(player).copied()?;
    if !may_drop(world, &ps, weapon) {
        return None;
    }
    let Rounds { right, left, reserve } = held_rounds(world, &ps, weapon);
    let number =
        launch_dropped_from_ps(world, tick, &ps, player.0 as i32, weapon, right, left, reserve, false);
    if number == ENTITYNUM_NONE {
        return None;
    }
    if let Some(ps) = world.player_mut(player) {
        take_weapon(ps, weapon);
    }
    Some(number)
}

/// A scavenger bag falls out of the player; nothing is taken from them.
pub(crate) fn drop_scavenger_item(world: &mut FrameWorld, tick: Tick, player: ClientId, weapon: u32) -> Option<i32> {
    let ps = world.player(player).copied()?;
    let number = launch_dropped_from_ps(world, tick, &ps, player.0 as i32, weapon, 0, 0, 0, true);
    (number != ENTITYNUM_NONE).then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use movement_t6::bridge::view_height;

    #[test]
    fn a_dropped_gun_appears_half_way_up_the_players_box_in_each_stance() {
        let mut ps = PlayerState::ZERO;
        let mut at = |target| {
            ps.view_height_target = target;
            release_height(&ps)
        };
        assert_eq!(at(view_height::STAND), 35.0);
        assert_eq!(at(view_height::CROUCH), 25.0);
        assert_eq!(at(view_height::LAST_STAND), 25.0);
        assert_eq!(at(view_height::PRONE), 15.0);
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
            let (ahead, _, _) = angle_vectors([0.0, yaw, 0.0]);
            let up = random_centered(&mut replay) * THROW_UP_SPREAD + THROW_UP_BASE;
            let east = random_centered(&mut replay) * THROW_SIDE_SPREAD;
            let north = random_centered(&mut replay) * THROW_SIDE_SPREAD;
            let expected = [ahead[0] * THROW_FORWARD + east, ahead[1] * THROW_FORWARD + north, ahead[2] * THROW_FORWARD + up];
            for axis in 0..3 {
                assert!((velocity[axis] - expected[axis]).abs() < 1e-3, "yaw {yaw} axis {axis}");
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
