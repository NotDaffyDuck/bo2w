//! The use button on the ground: which gun or thrown weapon the player is aiming at, how long the button must be
//! held to take it, and the prompt the player is shown meanwhile.

use game_math::euler_basis;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, buttons};
use weapon_t6::player::T6WeaponPs;
use weapon_t6::state::{is_dropping_or_swim_in, is_raising_or_swim_out};

use super::inventory::{current_primary, main_light_total};
use super::reclaim::{reclaim, rounds_after_reclaim};
use super::{LootOnGround, walker_can_touch};
use crate::InteractTap;
use crate::step_view::StepWorldView;
use crate::sim_parts::{SeatNo, StepNo, script_gun_wants_two_hands};

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
    no: i32,
    gun: u32,
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
fn alignment(forward: [f32; 3], shift: [f32; 3], distance: f32) -> f32 {
    if distance > 0.0 { (0..3).map(|axis| forward[axis] * shift[axis] / distance).sum() } else { 0.0 }
}

/// The distance cost of a target: nearer and more squarely in view is lower.
fn aim_score(forward: [f32; 3], shift: [f32; 3], distance: f32) -> f32 {
    distance + (1.0 - (alignment(forward, shift, distance) + 1.0) * 0.5) * AIM_PENALTY
}

fn line_of_sight_open(world: &StepWorldView, eye: [f32; 3], till: [f32; 3]) -> bool {
    world.trace_world(eye, till, [0.0; 3], [0.0; 3], SIGHT_MASK).fraction >= 1.0
}

/// The player's BO2 weapon state, when there is one to read.
fn gun_state(world: &StepWorldView, walker: SeatNo) -> Option<T6WeaponPs> {
    if world.gun_facts().is_empty() { None } else { crate::gun_brain::synced_copy(world, walker) }
}

/// Whether the use button offers nothing at all right now: the player is not in play, or the gun state forbids it
/// (climbing, sprinting, last stand, changing weapons, ...).
fn offers_nothing(world: &StepWorldView, walker: SeatNo, ps: &AvatarState, gun: Option<&T6WeaponPs>) -> bool {
    let blocked = match gun {
        Some(gun) => crate::floor_gun_facts::hint_blocked(gun, ps),
        // Without a copy of the gun state, the movement flags and weapon states that stop the hint.
        None => ps.movement_state_bits & (4 | 0x8000) != 0 || matches!(ps.weapon_state_t6, 22..=24 | 26..=28),
    };
    blocked || !walker_can_touch(world, walker, ps)
}

/// Whether `item` is a gun the use button may offer. With the gun state, that is what taking it would allow (within
/// the vertical window); without it, any other primary the player could take up.
fn offerable_gun(
    world: &StepWorldView,
    taker: SeatNo,
    ps: &AvatarState,
    gun: Option<&T6WeaponPs>,
    item: &LootOnGround,
    eye_z: f32,
    now: i32,
) -> bool {
    match gun {
        Some(weapon_ps) => {
            let below = item.origin[2] + GUN_BOX_MAXS[2] < eye_z - VERTICAL_WINDOW;
            let above = item.origin[2] + GUN_BOX_MINS[2] > eye_z + VERTICAL_WINDOW;
            !below && !above && crate::floor_gun_facts::use_candidate(weapon_ps, world.gun_facts(), item, taker, now)
        }
        None => {
            let weapon = item.state.weapon.0;
            let usable = !item.scavenge && weapon != 0 && !ps.weapons.contains(&(weapon as i32));
            let room = main_light_total(world, ps) < 2 || current_primary(world, ps) != 0;
            usable && room
        }
    }
}

/// What the use button would take right now, if anything.
fn aim_pick(world: &StepWorldView, walker: SeatNo, pstate: &AvatarState) -> Option<Target> {
    let gun = gun_state(world, walker);
    if offers_nothing(world, walker, pstate, gun.as_ref()) {
        return None;
    }
    let eye = [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now];
    let forward = euler_basis(pstate.look_angles).0;
    let clock_millis = world.entity_kernel().level_time_ms();
    let mut pick = Pick::default();

    for slot in world.ordered_loot_slots() {
        let Some(item) = world.loot_at_slot(slot) else {
            continue;
        };
        if !offerable_gun(world, walker, pstate, gun.as_ref(), &item, eye[2], clock_millis) {
            continue;
        }
        let middle: [f32; 3] =
            core::array::from_fn(|axis| item.origin[axis] + (GUN_BOX_MINS[axis] + GUN_BOX_MAXS[axis]) * 0.5);
        let gap: [f32; 3] = core::array::from_fn(|axis| middle[axis] - eye[axis]);
        let distance = length_squared(gap).sqrt();
        if distance > GUN_REACH || !line_of_sight_open(world, eye, middle) {
            continue;
        }
        let target = Target { no: item.state.slot_index, gun: item.state.index as u32, thrown: false };
        pick.offer(aim_score(forward, gap, distance), target);
    }

    world.visit_projectiles(|projectile| {
        if projectile.at.tr_type != entity_t6::flight::path_kind::AT_REST
            || rounds_after_reclaim(world, walker, pstate, projectile).is_none()
        {
            return;
        }
        let from_body: [f32; 3] = core::array::from_fn(|axis| projectile.origin[axis] - pstate.origin[axis]);
        if length_squared(from_body) > THROWN_BODY_REACH * THROWN_BODY_REACH {
            return;
        }
        let gap: [f32; 3] = core::array::from_fn(|axis| projectile.origin[axis] - eye[axis]);
        let distance = length_squared(gap).sqrt();
        if distance > THROWN_REACH || !line_of_sight_open(world, eye, projectile.origin) {
            return;
        }
        let target = Target { no: projectile.ent_no, gun: projectile.gun, thrown: true };
        pick.offer(aim_score(forward, gap, distance) - THROWN_BONUS, target);
    });
    pick.0.map(|(_, target)| target)
}

/// Whether the hold has lasted long enough to take what is aimed at. The use-and-reload button needs
/// `HOLD_TO_TAKE_MS` (and, with the gun state, a gun that is not coming up or going away); the plain use button
/// takes at once.
fn hold_long_enough(hold: Option<(i32, bool)>, gun: Option<&T6WeaponPs>, ps: &AvatarState, now: i32) -> bool {
    let waited = |since: i32| now - since >= HOLD_TO_TAKE_MS;
    match gun {
        Some(weapon_ps) => {
            crate::floor_gun_facts::hold_takes(weapon_ps, ps) && hold.is_none_or(|(since, by_reload)| !by_reload || waited(since))
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
pub(crate) fn phase_use_items(world: &mut StepWorldView, tick: StepNo, presses: &[InteractTap], orders: &[(u32, u32)]) {
    let clock = crate::bodies::level_time_ms(tick);
    for who in world.ordered_player_ids() {
        let Some(state) = world.avatar(who).copied() else {
            continue;
        };
        let aimed = aim_pick(world, who, &state);
        let gun = gun_state(world, who);
        let pressed_bits = orders.iter().filter(|&&(client, _)| client == who.0).fold(0, |all, &(_, bits)| all | bits);
        let held = pressed_bits & (buttons::INTERACT_KEY | buttons::INTERACT_RELOAD_KEY) != 0;
        let edge = presses.iter().any(|press| press.client == who.0 && press.rim);
        let aimed_entity = aimed.map(|target| world.entity_kernel().ref_for(target.no).expect("occupied item"));

        let usage = world.client_meta_mut(who);
        // Letting go of the use-and-reload button before it took anything reloads instead.
        let let_go_of_reload = !held && usage.item_use_hold.take().is_some_and(|(_, by_reload)| by_reload);
        if let_go_of_reload {
            usage.item_use_reload |= usage.use_target.is_some();
        }
        if !held || aimed.is_none() {
            usage.use_target = None;
        }
        if edge && gun.as_ref().is_none_or(crate::floor_gun_facts::press_picks) {
            usage.use_target = aimed_entity;
            usage.item_use_hold = Some((clock, pressed_bits & buttons::INTERACT_KEY == 0));
        }
        let settled = clock - usage.item_ready_ms >= SPAWN_GRACE_MS;
        let ready = held
            && settled
            && usage.use_target.is_some()
            && aimed_entity == usage.use_target
            && hold_long_enough(usage.item_use_hold, gun.as_ref(), &state, clock);
        if ready {
            let target = aimed.expect("aimed target");
            if target.thrown {
                reclaim(world, who, target.no, tick);
            } else {
                crate::floor_gun_facts::grab(world, who, target.no);
            }
            let usage = world.client_meta_mut(who);
            usage.use_target = None;
            usage.item_use_hold = None;
        }
        show_prompt(world, who);
    }
}

/// Fills in the on-screen prompt for what the use button is now pointing at (nothing, once it has been taken).
fn show_prompt(world: &mut StepWorldView, id: SeatNo) {
    let Some(fresh) = world.avatar(id).copied() else {
        return;
    };
    let aimed = aim_pick(world, id, &fresh);
    let two_hand = aimed.is_some_and(|target| script_gun_wants_two_hands(world.weapon_script_label(target.gun)));
    if let Some(hud) = world.occupant_mut(id) {
        hud.hint_kind = aimed.map_or(0, |target| target.gun as i32 + 4);
        hud.hint_entity = aimed.map_or(NO_ENTITY_SLOT, |target| target.no);
        hud.hint_text = -1;
        hud.cursor_hint_dual_wield = i32::from(two_hand);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AHEAD: [f32; 3] = [1.0, 0.0, 0.0];

    fn target(number: i32) -> Target {
        Target { no: number, gun: 1, thrown: false }
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
        assert_eq!(pick.0.as_ref().map(|(_, t)| t.no), Some(1));
        pick.offer(49.0, target(3));
        pick.offer(60.0, target(4));
        assert_eq!(pick.0.as_ref().map(|(_, t)| t.no), Some(3));
    }

    #[test]
    fn the_plain_use_button_takes_at_once_and_the_reload_button_after_the_hold() {
        let ps = AvatarState::ZERO;
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
        let mut ps = AvatarState::ZERO;
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
