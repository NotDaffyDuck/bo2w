//! Jumping: whether a jump may start, the launch itself, and the rules that
//! hold a jump to a fixed height.

use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, InputCommand, movement_state_bits};

use crate::bridge::event_feed::queue_forecast_happening;
use crate::bridge::stride_sfx::material_kind_number;
use crate::bridge::pml::TickLocals;
use crate::bridge::posture::PostureFooting;

/// The jump button in `InputCommand::buttons`.
const INTENT_LEAP_BIT: u32 = 0x400;

/// The jump event; its parameter is the surface the player left, or this
/// value for a ladder.
const EV_JUMP: i32 = 136;
const LADDER_JUMP_PARM: i32 = 0x15;

/// Movement flags that stop a jump from starting: mantling, and two bits the
/// engine sets while jumping is not allowed.
const NO_JUMP_FLAGS: u32 = movement_state_bits::MANTLE | 0x400 | 0x8_0000;

/// A jump cannot start again until this long after the last one began.
const JUMP_COOLDOWN_MS: i32 = 500;

/// Ground flagged slick refuses a jump.
const SURF_SLICK: u32 = 0x2;
/// Ground with this flag makes no footstep or jump noise.
const FLOOR_SILENT_FEET: u32 = 0x2000;

/// How high above where a jump started the player may rise, and the most one
/// ground step may lift them while it does.
const JUMP_HEIGHT_LIMIT: f32 = 39.0;
const JUMP_STEP_LIMIT: f32 = 18.0;

/// A jump off a ladder keeps this much of the climbing speed, and a push-off
/// away from the ladder reflects the heading off it by this multiple.
const LADDER_RISE_KEPT: f32 = 0.75;
const LADDER_REFLECT: f32 = -2.0;

/// Taking another jump this soon after landing one has its energy divided by
/// the land factor (see `bo2_land_factor`).
const LAND_FACTOR_WINDOW_MS: i32 = 0x6a3;
const LAND_FACTOR_MAX: f32 = 2.5;
const LAND_FACTOR_PER_MS: f32 = 0.000_588_235_27;
const REJUMP_WINDOW_MS: i32 = 0x708;

/// What the caller knows about the player's input and stance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeapGateArgs {
    /// Milliseconds since the last jump began.
    pub since_leap_ms: i32,
    /// The buttons held on the previous command.
    pub last_tick_buttons: u32,
    /// The player's `PostureFooting` as a number.
    pub posture_footing_kind: u8,
}

/// What the jump check decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeapGateVerdict {
    /// No jump this frame.
    BarredFromIt,
    /// The button was still held from the last jump; it has been cleared so
    /// it has to be pressed again.
    LeapHoldReleased,
    /// A jump may start.
    Ready,
    /// The jump started.
    Launched,
}

/// The settings a jump launches with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeapKickoffArgs {
    pub leap_height: f32,
    /// The jump is the take-off of a dive.
    pub dive: bool,
    /// What the launch energy is divided by when jumping again soon after a
    /// jump (the caller fills this in from `bo2_land_factor`).
    pub squat_leap_ratio: f32,
    /// The speed of the shove away from a ladder.
    pub rung_kickoff_speed: f32,
}

/// The highest the current jump may take the player.
fn top_of_jump(avatar: &AvatarState) -> f32 {
    avatar.takeoff_height + JUMP_HEIGHT_LIMIT
}

/// Forget the jump: the flag, and where it started.
pub fn clear_state(avatar: &mut AvatarState) {
    avatar.takeoff_height = 0.0;
    avatar.movement_state_bits &= !movement_state_bits::JUMPING;
}

/// How far a ground step may lift a player standing at height `z` without
/// taking them past the top of the jump; `None` once they are at or above it.
pub fn step_room(avatar: &AvatarState, z: f32) -> Option<f32> {
    let top = top_of_jump(avatar);
    match top <= z {
        true => None,
        false if z + JUMP_STEP_LIMIT > top => Some(top - z),
        false => Some(JUMP_STEP_LIMIT),
    }
}

/// The player is at or past the top of the jump.
pub(crate) fn above_max(avatar: &AvatarState) -> bool {
    top_of_jump(avatar) <= avatar.origin[2]
}

/// After a move that rose from `before`, hold the upward speed to what gets
/// the player no higher than the top of the jump.
pub(crate) fn clamp_velocity(avatar: &mut AvatarState, before: [f32; 3]) {
    let rose = !(avatar.origin[2] - before[2] <= 0.0);
    if rose {
        avatar.velocity[2] = rise_capped(avatar);
    }
}

/// The upward speed, cut to what just reaches the top (none within a tenth
/// of a unit of it).
fn rise_capped(avatar: &AvatarState) -> f32 {
    let headroom = top_of_jump(avatar) - avatar.origin[2];
    if headroom < 0.1 {
        return 0.0;
    }
    let reach_top = libm::sqrtf(avatar.gravity as f32 * headroom * 2.0);
    match reach_top < avatar.velocity[2] {
        true => reach_top,
        false => avatar.velocity[2],
    }
}

/// What a jump's launch energy is divided by: 2.5 just after landing one,
/// easing back to 1 as the landing wears off; always 1 with the jump slowdown
/// off.
#[must_use]
pub fn bo2_land_factor(avatar: &AvatarState, jump_slowdown: bool) -> f32 {
    match (jump_slowdown, avatar.move_timer > LAND_FACTOR_WINDOW_MS) {
        (false, _) => 1.0,
        (true, true) => LAND_FACTOR_MAX,
        (true, false) => 1.0 + avatar.move_timer as f32 * 1.5 * LAND_FACTOR_PER_MS,
    }
}

/// The take-off speed: from the jump height under gravity, its energy
/// divided down for a jump taken again soon after one (a dive never is).
fn launch_speed(avatar: &AvatarState, frame_ctx: &LeapKickoffArgs) -> f32 {
    let energy = avatar.gravity as f32 * (frame_ctx.leap_height + frame_ctx.leap_height);
    let again = avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer <= REJUMP_WINDOW_MS;
    libm::sqrtf(match again && !frame_ctx.dive {
        true => energy / frame_ctx.squat_leap_ratio,
        false => energy,
    })
}

/// Leave the ground going up, noting where and when the jump began.
fn take_off(avatar: &mut AvatarState, tick_vars: &mut TickLocals, authority_clock_ms: i32, frame_ctx: &LeapKickoffArgs) {
    let speed = launch_speed(avatar, frame_ctx);
    tick_vars.leave_ground();
    avatar.standing_on = NO_ENTITY_SLOT;
    avatar.velocity[2] = speed;
    avatar.takeoff_height = avatar.origin[2];
    avatar.hop_start_ms = authority_clock_ms;
    avatar.move_timer = 0;
    avatar.sprint_state.sprint_needs_release = 0;
    // The jump widens the aim spread, to its 255 ceiling.
    avatar.sight_spread_factor = match avatar.sight_spread_factor + 64.0 {
        wide if wide > 255.0 => 255.0,
        wide => wide,
    };
    // Leaving the ground ends a hard landing; a dive sets its own flag later.
    let cleared = avatar.movement_state_bits & !(movement_state_bits::TIME_HARDLANDING | 0x100);
    avatar.movement_state_bits = cleared | movement_state_bits::JUMPING;
}

/// The surface type a jump or step off this ground sounds like; 0 for ground
/// that makes no noise.
pub fn ground_surface_type(surface_flags: u32) -> i32 {
    match surface_flags & FLOOR_SILENT_FEET {
        0 => material_kind_number(surface_flags) as i32,
        _ => 0,
    }
}

/// Tell the clients about the jump: a ladder push, or a leap off a surface
/// that makes a noise.
fn announce(avatar: &mut AvatarState, surf: u32) {
    let parm = match avatar.movement_state_bits & movement_state_bits::LADDER {
        0 => ground_surface_type(surf),
        _ => LADDER_JUMP_PARM,
    };
    if parm != 0 {
        queue_forecast_happening(avatar, EV_JUMP, parm);
    }
}

/// `v` scaled to unit length; a zero vector stays as it is.
fn unit(v: [f32; 3]) -> [f32; 3] {
    let mag = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    match mag > 0.0 {
        true => v.map(|c| c / mag),
        false => v,
    }
}

fn dot3(lhs: [f32; 3], rhs: [f32; 3]) -> f32 {
    lhs[0] * rhs[0] + lhs[1] * rhs[1] + lhs[2] * rhs[2]
}

/// Shove the player off the ladder along the way they are facing; facing the
/// ladder, the heading is reflected off it so they go away from the rungs.
fn push_off_ladder(avatar: &mut AvatarState, tick_vars: &TickLocals, push_speed: f32) {
    let rungs = avatar.ladder_normal_dir;
    let level = unit([tick_vars.fwd_axis[0], tick_vars.fwd_axis[1], 0.0]);
    let facing_away = dot3(rungs, tick_vars.fwd_axis) >= 0.0;
    let heading = match facing_away {
        true => level,
        false => {
            let k = dot3(level, rungs) * LADDER_REFLECT;
            unit(core::array::from_fn(|i| level[i] + k * rungs[i]))
        }
    };
    avatar.velocity[2] *= LADDER_RISE_KEPT;
    avatar.velocity[0] = push_speed * heading[0];
    avatar.velocity[1] = push_speed * heading[1];
    avatar.movement_state_bits &= !movement_state_bits::LADDER;
}

/// Decide whether a jump may start this frame. A button still held from the
/// last jump is cleared from `cmd`.
pub fn check_gate(
    avatar: &AvatarState,
    ucmd: &mut InputCommand,
    frame_ctx: LeapGateArgs,
) -> LeapGateVerdict {
    let free = avatar.movement_state_bits & NO_JUMP_FLAGS == 0
        && frame_ctx.since_leap_ms >= JUMP_COOLDOWN_MS
        && !movement_mode::is_dead(avatar.movement_mode);
    let standing = frame_ctx.posture_footing_kind == PostureFooting::Stand as u8;
    let pressed = ucmd.buttons & INTENT_LEAP_BIT != 0;
    match (free && standing && pressed, frame_ctx.last_tick_buttons & INTENT_LEAP_BIT != 0) {
        (false, _) => LeapGateVerdict::BarredFromIt,
        (true, false) => LeapGateVerdict::Ready,
        (true, true) => {
            // Held over from the last jump: it has to be let go first.
            ucmd.buttons &= !INTENT_LEAP_BIT;
            LeapGateVerdict::LeapHoldReleased
        }
    }
}

/// Check for a jump and, if one is allowed, start it: launch, tell the
/// clients, and push off a ladder when on one. Slick ground refuses a jump.
pub fn check(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    ucmd: &mut InputCommand,
    gate: LeapGateArgs,
    launch: LeapKickoffArgs,
) -> LeapGateVerdict {
    match check_gate(avatar, ucmd, gate) {
        LeapGateVerdict::Ready => {}
        other => return other,
    }
    let underfoot = tick_vars.floor_probe[4];
    if underfoot & SURF_SLICK != 0 {
        return LeapGateVerdict::BarredFromIt;
    }
    take_off(avatar, tick_vars, ucmd.authority_clock_ms, &launch);
    announce(avatar, underfoot);
    if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        push_off_ladder(avatar, tick_vars, launch.rung_kickoff_speed);
    }
    LeapGateVerdict::Launched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;
    use LeapGateVerdict::{LeapHoldReleased, Launched, BarredFromIt, Ready};

    const STONE: u32 = 5 << 20;

    /// Standing, a second after the last jump, nothing held before.
    const GATE: LeapGateArgs = LeapGateArgs { since_leap_ms: 1000, last_tick_buttons: 0, posture_footing_kind: 0 };

    /// A 39-unit jump, halved when taken again, 200 off a ladder.
    const LAUNCH: LeapKickoffArgs =
        LeapKickoffArgs { squat_leap_ratio: 2.0, rung_kickoff_speed: 200.0, leap_height: 39.0_f32, dive: !true };

    fn press() -> InputCommand {
        InputCommand { buttons: INTENT_LEAP_BIT, authority_clock_ms: 4000, ..InputCommand::default() }
    }

    /// On entity 3 at height 100, gravity 800.
    fn grounded_now() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        (avatar.gravity, avatar.standing_on, avatar.origin) = (800, 3, [0.0, 0.0, 100.0]);
        avatar
    }

    /// Runs the full check with a pressed jump on flat ground.
    fn jump(avatar: &mut AvatarState, launch: LeapKickoffArgs) -> (LeapGateVerdict, TickLocals) {
        let mut frame = tick_vars(true);
        let result = check(avatar, &mut frame, &mut press(), GATE, launch);
        (result, frame)
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn clearing_the_state_drops_the_flag_and_the_start_height() {
        let mut avatar = grounded_now();
        avatar.movement_state_bits = movement_state_bits::JUMPING | movement_state_bits::DUCK_BUTTON_BIT;
        avatar.takeoff_height = 55.0;
        clear_state(&mut avatar);
        assert_eq!((avatar.movement_state_bits, avatar.takeoff_height), (movement_state_bits::DUCK_BUTTON_BIT, 0.0));
    }

    #[test]
    fn a_ground_step_is_cut_short_by_the_top_of_the_jump() {
        let mut avatar = grounded_now();
        avatar.takeoff_height = 100.0;
        for (z, step) in [(100.0, Some(18.0)), (130.0, Some(9.0)), (139.0, None)] {
            assert_eq!(step_room(&avatar, z), step, "{z}");
        }
    }

    #[test]
    fn the_top_of_the_jump_is_thirty_nine_above_where_it_began() {
        let mut avatar = grounded_now();
        avatar.takeoff_height = 100.0;
        avatar.origin[2] = 138.9;
        assert!(!above_max(&avatar));
        avatar.origin[2] = 139.0;
        assert!(above_max(&avatar));
    }

    #[test]
    fn a_rising_jump_is_slowed_to_just_reach_the_top() {
        let mut avatar = grounded_now();
        avatar.takeoff_height = 100.0;
        let mut after = |z: f32, vz: f32, from: f32| {
            avatar.origin[2] = z;
            avatar.velocity[2] = vz;
            clamp_velocity(&mut avatar, [0.0, 0.0, from]);
            avatar.velocity[2]
        };
        // 9 units of room under gravity 800: sqrt(800 * 9 * 2).
        let v = after(130.0, 500.0, 120.0);
        assert!(near(v, 120.0, 1e-3), "{v}");
        assert_eq!(after(138.95, 80.0, 138.0), 0.0);
        assert_eq!(after(138.95, 80.0, 139.0), 80.0, "falling or level is left alone");
    }

    #[test]
    fn landing_hard_makes_the_next_jump_weaker_for_a_moment() {
        let mut avatar = grounded_now();
        assert_eq!(bo2_land_factor(&avatar, false), 1.0);
        let at = |avatar: &mut AvatarState, t: i32| {
            avatar.move_timer = t;
            bo2_land_factor(avatar, true)
        };
        assert_eq!(at(&mut avatar, 5000), 2.5);
        assert!(near(at(&mut avatar, 1000), 1.882_353, 1e-4));
        assert_eq!(at(&mut avatar, 0), 1.0);
    }

    #[test]
    fn a_jump_leaves_the_ground_going_up_at_the_launch_speed() {
        let mut avatar = grounded_now();
        let (result, frame) = jump(&mut avatar, LAUNCH);
        assert_eq!(result, Launched);
        assert!(near(avatar.velocity[2], libm::sqrtf(800.0 * 78.0), 1e-3));
        assert_eq!(avatar.standing_on, NO_ENTITY_SLOT);
        assert_eq!([frame.walking, frame.standing_on_floor, frame.near_floor_contact], [0; 3]);
        assert_eq!((avatar.takeoff_height, avatar.hop_start_ms), (100.0, 4000));
        assert_ne!(avatar.movement_state_bits & movement_state_bits::JUMPING, 0);
        assert_eq!(avatar.sight_spread_factor, 64.0);
    }

    #[test]
    fn a_hard_landing_flag_ends_when_the_jump_starts() {
        let mut avatar = grounded_now();
        avatar.movement_state_bits = movement_state_bits::TIME_HARDLANDING | movement_state_bits::DUCK_BUTTON_BIT;
        jump(&mut avatar, LAUNCH);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::TIME_HARDLANDING, 0);
        assert_ne!(avatar.movement_state_bits & movement_state_bits::DUCK_BUTTON_BIT, 0);
    }

    #[test]
    fn jumping_again_inside_the_window_divides_the_energy() {
        let again = || {
            let mut avatar = grounded_now();
            avatar.movement_state_bits = movement_state_bits::JUMPING;
            avatar.move_timer = 100;
            avatar
        };
        let mut plain = again();
        jump(&mut plain, LAUNCH);
        assert!(near(plain.velocity[2], libm::sqrtf(800.0 * 78.0 / 2.0), 1e-3));
        let mut dive = again();
        jump(&mut dive, LeapKickoffArgs { dive: true, ..LAUNCH });
        assert!(near(dive.velocity[2], libm::sqrtf(800.0 * 78.0), 1e-3), "a dive is not weakened");
    }

    #[test]
    fn the_gate_refuses_what_stops_a_jump() {
        let avatar = grounded_now();
        let mut ucmd = press();
        let mut gate_with = |avatar: &AvatarState, gate: LeapGateArgs| check_gate(avatar, &mut ucmd, gate);
        assert_eq!(gate_with(&avatar, GATE), Ready);
        assert_eq!(gate_with(&avatar, LeapGateArgs { since_leap_ms: 499, ..GATE }), BarredFromIt);
        assert_eq!(gate_with(&avatar, LeapGateArgs { since_leap_ms: 500, ..GATE }), Ready);
        let crouched = LeapGateArgs { posture_footing_kind: PostureFooting::Crouch as u8, ..GATE };
        assert_eq!(gate_with(&avatar, crouched), BarredFromIt);
        for flag in [movement_state_bits::MANTLE, 0x400, 0x8_0000] {
            let mut blocked = grounded_now();
            blocked.movement_state_bits = flag;
            assert_eq!(gate_with(&blocked, GATE), BarredFromIt, "flag {flag:#x}");
        }
        let mut dead = grounded_now();
        dead.movement_mode = movement_mode::DEAD;
        assert_eq!(gate_with(&dead, GATE), BarredFromIt);
        assert_eq!(check_gate(&avatar, &mut InputCommand::default(), GATE), BarredFromIt);
    }

    #[test]
    fn a_jump_button_held_from_before_is_cleared_and_needs_a_fresh_press() {
        let mut ucmd = press();
        let held = LeapGateArgs { last_tick_buttons: INTENT_LEAP_BIT, ..GATE };
        assert_eq!(check_gate(&grounded_now(), &mut ucmd, held), LeapHoldReleased);
        assert_eq!(ucmd.buttons & INTENT_LEAP_BIT, 0);
    }

    #[test]
    fn slick_ground_refuses_the_jump() {
        let mut avatar = grounded_now();
        let mut frame = tick_vars(true);
        frame.floor_probe[4] = SURF_SLICK;
        assert_eq!(check(&mut avatar, &mut frame, &mut press(), GATE, LAUNCH), BarredFromIt);
        assert_eq!(avatar.standing_on, 3);
    }

    #[test]
    fn the_jump_event_names_the_ground_and_stays_quiet_on_silent_ground() {
        assert_eq!(ground_surface_type(STONE), 5);
        assert_eq!(ground_surface_type(STONE | FLOOR_SILENT_FEET), 0);
        let mut avatar = grounded_now();
        announce(&mut avatar, STONE);
        assert_eq!((avatar.predictable_events[0], avatar.predictable_event_parms[0]), (EV_JUMP, 5));
        let seq = avatar.predictable_event_sequence;
        announce(&mut avatar, 0);
        assert_eq!(avatar.predictable_event_sequence, seq, "the default surface adds nothing");
    }

    #[test]
    fn jumping_off_a_ladder_pushes_away_and_lets_go() {
        let mut avatar = grounded_now();
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.velocity = [0.0_f32, 0.0, 100.0];
        avatar.ladder_normal_dir = [1.0, 0.0, 0.0];
        // Facing along +x with the ladder normal also +x: straight off.
        jump(&mut avatar, LAUNCH);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::LADDER, 0);
        assert_eq!(avatar.predictable_event_parms[0], LADDER_JUMP_PARM as u32);
        assert!(near(avatar.velocity[0], 200.0, 1e-3) && avatar.velocity[1].abs() < 1e-3);
    }

    #[test]
    fn facing_the_ladder_the_push_is_reflected_away_from_it() {
        let mut avatar = grounded_now();
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.ladder_normal_dir = [-1.0, 0.0, 0.0];
        push_off_ladder(&mut avatar, &tick_vars(true), 200.0);
        assert!(near(avatar.velocity[0], -200.0, 1e-3), "{:?}", avatar.velocity);
    }
}
