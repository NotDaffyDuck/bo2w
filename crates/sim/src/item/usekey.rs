//! The use button on the ground: which gun or thrown weapon the player is aiming at, how long the button must be
//! held to take it, and the prompt the player is shown meanwhile.

use game_math::angle_vectors;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, buttons};
use weapon_t6::player::T6WeaponPs;
use weapon_t6::state::{is_dropping_or_swim_in, is_raising_or_swim_out};

use super::inventory::{current_primary, primary_count};
use super::reclaim::{reclaim, rounds_after_reclaim};
use super::{DroppedItem, walker_can_touch};
use crate::UsePress;
use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick, gsc_give_weapon_is_akimbo};

/// How long the use-and-reload button is held to take a gun; a shorter tap reloads instead. The plain use button
/// takes it at once.
const HOLD_TO_TAKE_MS: i32 = 250;
/// A fresh spawn cannot take anything for this long.
const SPAWN_GRACE_MS: i32 = 500;

/// A gun is offered to the use button within this distance of the eye, and only if nothing solid is in between.
const GUN_REACH: f32 = 128.0;
/// A thrown weapon: within this distance of the eye, and within `THROWN_BODY_REACH` of the player's origin.
const THROWN_REACH: f32 = 160.0;
const THROWN_BODY_REACH: f32 = 90.0;
/// Guns higher or lower than this from the eye are not offered.
const VERTICAL_WINDOW: f32 = 96.0;
/// Looking straight away from a target costs this much more than looking straight at it.
const AIM_PENALTY: f32 = 256.0;
/// A thrown weapon is preferred over a gun by this much.
const THROWN_BONUS: f32 = 512.0;
/// The line of sight to a target is blocked by solid world and clips of this kind.
const SIGHT_MASK: u32 = 0x11;
/// The box a dropped gun is aimed at by: one unit on a side, from its origin.
const GUN_BOX_MINS: [f32; 3] = [0.0; 3];
const GUN_BOX_MAXS: [f32; 3] = [1.0; 3];

/// What the use button is pointing at.
#[derive(Clone, Copy)]
struct Target {
    number: i32,
    weapon: u32,
    /// A thrown weapon lying still, as opposed to a dropped gun.
    thrown: bool,
}

/// The best target seen so far; the lowest score wins and the first seen keeps a tie.
#[derive(Default)]
struct Pick(Option<(f32, Target)>);

impl Pick {
    fn offer(&mut self, score: f32, target: Target) {
        if self.0.as_ref().is_none_or(|(best, _)| score < *best) {
            self.0 = Some((score, target));
        }
    }
}

fn length_squared(v: [f32; 3]) -> f32 {
    v.iter().map(|part| part * part).sum()
}

/// How closely the player's view points at something `distance` away along `delta`: 1 dead on, -1 straight away.
fn alignment(forward: [f32; 3], delta: [f32; 3], distance: f32) -> f32 {
    if distance > 0.0 { (0..3).map(|axis| forward[axis] * delta[axis] / distance).sum() } else { 0.0 }
}

/// The distance cost of a target: nearer and more squarely in view is lower.
fn aim_score(forward: [f32; 3], delta: [f32; 3], distance: f32) -> f32 {
    distance + (1.0 - (alignment(forward, delta, distance) + 1.0) * 0.5) * AIM_PENALTY
}

fn sight_clear(world: &FrameWorld, eye: [f32; 3], to: [f32; 3]) -> bool {
    world.trace_world(eye, to, [0.0; 3], [0.0; 3], SIGHT_MASK).fraction >= 1.0
}

/// The player's BO2 weapon state, when there is one to read.
fn gun_state(world: &FrameWorld, walker: ClientId) -> Option<T6WeaponPs> {
    if world.gun_facts().is_empty() { None } else { crate::t6_gun::synced_copy(world, walker) }
}

/// Whether the use button offers nothing at all right now: the player is not in play, or the gun state forbids it
/// (climbing, sprinting, last stand, changing weapons, ...).
fn offers_nothing(world: &FrameWorld, walker: ClientId, ps: &PlayerState, gun: Option<&T6WeaponPs>) -> bool {
    let blocked = match gun {
        Some(gun) => crate::item_t6::hint_blocked(gun, ps),
        // Without a copy of the gun state, the movement flags and weapon states that stop the hint.
        None => ps.pm_flags & (4 | 0x8000) != 0 || matches!(ps.weapon_state_t6, 22..=24 | 26..=28),
    };
    blocked || !walker_can_touch(world, walker, ps)
}

/// Whether `item` is a gun the use button may offer. With the gun state, that is what taking it would allow (within
/// the vertical window); without it, any other primary the player could take up.
fn offerable_gun(
    world: &FrameWorld,
    walker: ClientId,
    ps: &PlayerState,
    gun: Option<&T6WeaponPs>,
    item: &DroppedItem,
    eye_z: f32,
    now: i32,
) -> bool {
    match gun {
        Some(gun) => {
            let below = item.origin[2] + GUN_BOX_MAXS[2] < eye_z - VERTICAL_WINDOW;
            let above = item.origin[2] + GUN_BOX_MINS[2] > eye_z + VERTICAL_WINDOW;
            !below && !above && crate::item_t6::use_candidate(gun, world.gun_facts(), item, walker, now)
        }
        None => {
            let weapon = item.state.weapon.0;
            let usable = !item.scavenger && weapon != 0 && !ps.weapons.contains(&(weapon as i32));
            let room = primary_count(world, ps) < 2 || current_primary(world, ps) != 0;
            usable && room
        }
    }
}

/// What the use button would take right now, if anything.
fn aim_pick(world: &FrameWorld, walker: ClientId, ps: &PlayerState) -> Option<Target> {
    let gun = gun_state(world, walker);
    if offers_nothing(world, walker, ps, gun.as_ref()) {
        return None;
    }
    let eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
    let (forward, _, _) = angle_vectors(ps.viewangles);
    let now = world.entity_kernel().level_time_ms();
    let mut pick = Pick::default();

    for number in world.dropped_item_numbers_sorted() {
        let Some(item) = world.dropped_item_by_number(number) else {
            continue;
        };
        if !offerable_gun(world, walker, ps, gun.as_ref(), &item, eye[2], now) {
            continue;
        }
        let middle: [f32; 3] =
            core::array::from_fn(|axis| item.origin[axis] + (GUN_BOX_MINS[axis] + GUN_BOX_MAXS[axis]) * 0.5);
        let delta: [f32; 3] = core::array::from_fn(|axis| middle[axis] - eye[axis]);
        let distance = length_squared(delta).sqrt();
        if distance > GUN_REACH || !sight_clear(world, eye, middle) {
            continue;
        }
        let target = Target { number: item.state.number, weapon: item.state.index as u32, thrown: false };
        pick.offer(aim_score(forward, delta, distance), target);
    }

    world.visit_projectiles(|projectile| {
        if projectile.pos.tr_type != entity_t6::trajectory::tr_type::STATIONARY
            || rounds_after_reclaim(world, walker, ps, projectile).is_none()
        {
            return;
        }
        let from_body: [f32; 3] = core::array::from_fn(|axis| projectile.origin[axis] - ps.origin[axis]);
        if length_squared(from_body) > THROWN_BODY_REACH * THROWN_BODY_REACH {
            return;
        }
        let delta: [f32; 3] = core::array::from_fn(|axis| projectile.origin[axis] - eye[axis]);
        let distance = length_squared(delta).sqrt();
        if distance > THROWN_REACH || !sight_clear(world, eye, projectile.origin) {
            return;
        }
        let target = Target { number: projectile.entnum, weapon: projectile.weapon, thrown: true };
        pick.offer(aim_score(forward, delta, distance) - THROWN_BONUS, target);
    });
    pick.0.map(|(_, target)| target)
}

/// Whether the hold has lasted long enough to take what is aimed at. The use-and-reload button needs
/// `HOLD_TO_TAKE_MS` (and, with the gun state, a gun that is not coming up or going away); the plain use button
/// takes at once.
fn hold_long_enough(hold: Option<(i32, bool)>, gun: Option<&T6WeaponPs>, ps: &PlayerState, now: i32) -> bool {
    let waited = |since: i32| now - since >= HOLD_TO_TAKE_MS;
    match gun {
        Some(gun) => {
            crate::item_t6::hold_takes(gun, ps) && hold.is_none_or(|(since, by_reload)| !by_reload || waited(since))
        }
        None => hold.is_none_or(|(since, by_reload)| {
            !by_reload
                // The swim in and out hold it too, as the old mirror of the weapon states read them.
                || (waited(since)
                    && !is_raising_or_swim_out(ps.weapon_state_t6)
                    && !is_dropping_or_swim_in(ps.weapon_state_t6))
        }),
    }
}

/// Runs the use button for every player: the hold timing, taking what is aimed at, and the prompt on the screen.
pub(crate) fn phase_use_items(world: &mut FrameWorld, tick: Tick, presses: &[UsePress], cmds: &[(u32, u32)]) {
    let now = crate::corpse::level_time_ms(tick);
    for id in world.client_ids_sorted() {
        let Some(ps) = world.player(id).copied() else {
            continue;
        };
        let aimed = aim_pick(world, id, &ps);
        let gun = gun_state(world, id);
        let pressed_bits = cmds.iter().filter(|&&(client, _)| client == id.0).fold(0, |all, &(_, bits)| all | bits);
        let held = pressed_bits & (buttons::USE | buttons::USE_RELOAD) != 0;
        let edge = presses.iter().any(|press| press.client == id.0 && press.edge);
        let aimed_entity = aimed.map(|target| world.entity_kernel().current_ref(target.number).expect("occupied item"));

        let meta = world.client_meta_mut(id);
        // Letting go of the use-and-reload button before it took anything reloads instead.
        let let_go_of_reload = !held && meta.item_use_hold.take().is_some_and(|(_, by_reload)| by_reload);
        if let_go_of_reload {
            meta.item_use_reload |= meta.item_use_entity.is_some();
        }
        if !held || aimed.is_none() {
            meta.item_use_entity = None;
        }
        if edge && gun.as_ref().is_none_or(crate::item_t6::press_picks) {
            meta.item_use_entity = aimed_entity;
            meta.item_use_hold = Some((now, pressed_bits & buttons::USE == 0));
        }
        let settled = now - meta.item_use_spawn_ms >= SPAWN_GRACE_MS;
        let ready = held
            && settled
            && meta.item_use_entity.is_some()
            && aimed_entity == meta.item_use_entity
            && hold_long_enough(meta.item_use_hold, gun.as_ref(), &ps, now);
        if ready {
            let target = aimed.expect("aimed target");
            if target.thrown {
                reclaim(world, id, target.number, tick);
            } else {
                crate::item_t6::grab(world, id, target.number);
            }
            let meta = world.client_meta_mut(id);
            meta.item_use_entity = None;
            meta.item_use_hold = None;
        }
        show_prompt(world, id);
    }
}

/// Fills in the on-screen prompt for what the use button is now pointing at (nothing, once it has been taken).
fn show_prompt(world: &mut FrameWorld, id: ClientId) {
    let Some(fresh) = world.player(id).copied() else {
        return;
    };
    let aimed = aim_pick(world, id, &fresh);
    let dual = aimed.is_some_and(|target| gsc_give_weapon_is_akimbo(world.weapon_script_name(target.weapon)));
    if let Some(ps) = world.player_mut(id) {
        ps.cursor_hint = aimed.map_or(0, |target| target.weapon as i32 + 4);
        ps.cursor_hint_ent_index = aimed.map_or(ENTITYNUM_NONE, |target| target.number);
        ps.cursor_hint_string = -1;
        ps.cursor_hint_dual_wield = i32::from(dual);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AHEAD: [f32; 3] = [1.0, 0.0, 0.0];

    fn target(number: i32) -> Target {
        Target { number, weapon: 1, thrown: false }
    }

    #[test]
    fn alignment_is_one_dead_ahead_minus_one_behind_and_zero_on_top() {
        assert_eq!(alignment(AHEAD, [5.0, 0.0, 0.0], 5.0), 1.0);
        assert_eq!(alignment(AHEAD, [-5.0, 0.0, 0.0], 5.0), -1.0);
        assert_eq!(alignment(AHEAD, [0.0, 3.0, 0.0], 3.0), 0.0);
        assert_eq!(alignment(AHEAD, [0.0; 3], 0.0), 0.0);
    }

    #[test]
    fn looking_away_costs_the_full_penalty_and_looking_sideways_half() {
        assert_eq!(aim_score(AHEAD, [10.0, 0.0, 0.0], 10.0), 10.0);
        assert_eq!(aim_score(AHEAD, [0.0, 10.0, 0.0], 10.0), 10.0 + AIM_PENALTY * 0.5);
        assert_eq!(aim_score(AHEAD, [-10.0, 0.0, 0.0], 10.0), 10.0 + AIM_PENALTY);
    }

    #[test]
    fn a_thrown_weapon_in_reach_always_beats_any_gun_in_reach() {
        let worst_thrown = aim_score(AHEAD, [-THROWN_REACH, 0.0, 0.0], THROWN_REACH) - THROWN_BONUS;
        let best_gun = aim_score(AHEAD, [0.5, 0.0, 0.0], 0.5);
        assert!(worst_thrown < best_gun);
    }

    #[test]
    fn the_lowest_score_wins_and_the_first_of_a_tie_keeps_it() {
        let mut pick = Pick::default();
        assert!(pick.0.is_none());
        pick.offer(50.0, target(1));
        pick.offer(50.0, target(2));
        assert_eq!(pick.0.as_ref().map(|(_, t)| t.number), Some(1));
        pick.offer(49.0, target(3));
        pick.offer(60.0, target(4));
        assert_eq!(pick.0.as_ref().map(|(_, t)| t.number), Some(3));
    }

    #[test]
    fn the_plain_use_button_takes_at_once_and_the_reload_button_after_the_hold() {
        let ps = PlayerState::ZERO;
        assert!(hold_long_enough(None, None, &ps, 1000));
        assert!(hold_long_enough(Some((1000, false)), None, &ps, 1000));
        assert!(!hold_long_enough(Some((1000, true)), None, &ps, 1000));
        assert!(!hold_long_enough(Some((1000, true)), None, &ps, 1000 + HOLD_TO_TAKE_MS - 1));
        assert!(hold_long_enough(Some((1000, true)), None, &ps, 1000 + HOLD_TO_TAKE_MS));
    }

    #[test]
    fn the_reload_button_does_not_take_while_the_gun_is_coming_up_or_going_away() {
        let held_long = Some((0, true));
        let now = 10 * HOLD_TO_TAKE_MS;
        let mut ps = PlayerState::ZERO;
        let busy: Vec<i32> =
            (0..64).filter(|&state| is_raising_or_swim_out(state) || is_dropping_or_swim_in(state)).collect();
        assert!(!busy.is_empty(), "no weapon state counts as busy");
        for state in busy {
            ps.weapon_state_t6 = state;
            assert!(!hold_long_enough(held_long, None, &ps, now), "state {state}");
            // The plain use button never waits on the gun in this fallback.
            assert!(hold_long_enough(Some((0, false)), None, &ps, now), "state {state}");
        }
    }
}
