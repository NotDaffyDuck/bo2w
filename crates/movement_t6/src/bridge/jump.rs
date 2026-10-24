//! Jumping: whether a jump may start, the launch itself, and the rules that
//! hold a jump to a fixed height.

use playerstate_t6::consts::pm_type;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, UserCmd, pm_flags};

use crate::bridge::events::add_predictable_event;
use crate::bridge::footstep::surface_type_index;
use crate::bridge::pml::Pml;
use crate::bridge::stance::StanceSurface;

/// The jump button in `UserCmd::buttons`.
const BUTTON_JUMP: u32 = 0x400;

/// The jump event; its parameter is the surface the player left, or this
/// value for a ladder.
const EV_JUMP: i32 = 136;
const LADDER_JUMP_PARM: i32 = 0x15;

/// Movement flags that stop a jump from starting: mantling, and two bits the
/// engine sets while jumping is not allowed.
const NO_JUMP_FLAGS: u32 = pm_flags::MANTLE | 0x400 | 0x8_0000;

/// A jump cannot start again until this long after the last one began.
const JUMP_COOLDOWN_MS: i32 = 500;

/// Ground flagged slick refuses a jump.
const SURF_SLICK: u32 = 0x2;
/// Ground with this flag makes no footstep or jump noise.
const SURF_NOSTEPS: u32 = 0x2000;

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
pub struct JumpCheckContext {
    /// Milliseconds since the last jump began.
    pub time_since_jump: i32,
    /// The buttons held on the previous command.
    pub old_buttons: u32,
    /// The player's `StanceSurface` as a number.
    pub stance_surface_type: u8,
}

/// What the jump check decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JumpCheckResult {
    /// No jump this frame.
    NotEligible,
    /// The button was still held from the last jump; it has been cleared so
    /// it has to be pressed again.
    HeldJumpCleared,
    /// A jump may start.
    Ready,
    /// The jump started.
    Launched,
}

/// The settings a jump launches with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpLaunchContext {
    pub jump_height: f32,
    /// The jump is the take-off of a dive.
    pub dive: bool,
    /// What the launch energy is divided by when jumping again soon after a
    /// jump (the caller fills this in from `bo2_land_factor`).
    pub crouch_jump_scale: f32,
    /// The speed of the shove away from a ladder.
    pub jump_ladder_push_vel: f32,
}

/// Forget the jump: the flag, and where it started.
pub fn clear_state(ps: &mut PlayerState) {
    ps.pm_flags &= !pm_flags::JUMPING;
    ps.jump_origin_z = 0.0;
}

/// How far a ground step may lift the player at `origin` without taking them
/// past the top of the jump; `None` when they are already at or above it.
pub fn get_step_height(ps: &PlayerState, origin: [f32; 3]) -> Option<f32> {
    let ceiling = ps.jump_origin_z + JUMP_HEIGHT_LIMIT;
    if ceiling <= origin[2] {
        return None;
    }
    if ceiling < origin[2] + JUMP_STEP_LIMIT {
        return Some(ceiling - origin[2]);
    }
    Some(JUMP_STEP_LIMIT)
}

/// The player is at or past the top of the jump.
pub(crate) fn above_max(ps: &PlayerState) -> bool {
    ps.jump_origin_z + JUMP_HEIGHT_LIMIT <= ps.origin[2]
}

/// After a move that rose from `before`, hold the upward speed to what gets
/// the player no higher than the top of the jump.
pub(crate) fn clamp_velocity(ps: &mut PlayerState, before: [f32; 3]) {
    if ps.origin[2] - before[2] <= 0.0 {
        return;
    }
    let room = (ps.jump_origin_z + JUMP_HEIGHT_LIMIT) - ps.origin[2];
    if room < 0.1 {
        ps.velocity[2] = 0.0;
        return;
    }
    let speed_to_top = libm::sqrtf(ps.gravity as f32 * room * 2.0);
    if speed_to_top < ps.velocity[2] {
        ps.velocity[2] = speed_to_top;
    }
}

/// What a jump's launch energy is divided by: 2.5 just after landing one,
/// easing back to 1 as the landing wears off; always 1 with the jump slowdown
/// off.
#[must_use]
pub fn bo2_land_factor(ps: &PlayerState, jump_slowdown: bool) -> f32 {
    if !jump_slowdown {
        return 1.0;
    }
    if ps.pm_time > LAND_FACTOR_WINDOW_MS {
        return LAND_FACTOR_MAX;
    }
    (ps.pm_time as f32 * 1.5 * LAND_FACTOR_PER_MS) + 1.0
}

/// Launch the player upward. A jump taken again soon after one is weaker.
fn start(ps: &mut PlayerState, pml: &mut Pml, cmd: &UserCmd, context: JumpLaunchContext) {
    let mut energy = (ps.gravity as f32) * (context.jump_height + context.jump_height);
    if ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time <= REJUMP_WINDOW_MS && !context.dive {
        energy /= context.crouch_jump_scale;
    }

    pml.walking = 0;
    pml.ground_plane = 0;
    pml.almost_ground_plane = 0;

    ps.ground_entity_num = ENTITYNUM_NONE;
    ps.jump_origin_z = ps.origin[2];
    ps.jump_time = cmd.server_time;
    ps.velocity[2] = libm::sqrtf(energy);

    let flags = ps.pm_flags;
    ps.pm_time = 0;
    ps.sprint_state.sprint_button_up_required = 0;

    let spread = ps.aim_spread_scale + 64.0;
    ps.aim_spread_scale = if spread > 255.0 { 255.0 } else { spread };

    // Leaving the ground ends a hard landing; a dive sets its own flag later.
    ps.pm_flags = (flags & !(pm_flags::TIME_HARDLANDING | 0x100)) | pm_flags::JUMPING;
}

/// The surface type a jump or step off this ground sounds like; 0 for ground
/// that makes no noise.
pub fn ground_surface_type(surface_flags: u32) -> i32 {
    if surface_flags & SURF_NOSTEPS != 0 {
        return 0;
    }
    surface_type_index(surface_flags) as i32
}

/// Tell the clients about the jump: a ladder push, or a leap off a surface
/// that makes a noise.
fn event(ps: &mut PlayerState, surface_flags: u32) {
    if ps.pm_flags & pm_flags::LADDER != 0 {
        add_predictable_event(ps, EV_JUMP, LADDER_JUMP_PARM);
        return;
    }
    let surface = ground_surface_type(surface_flags);
    if surface != 0 {
        add_predictable_event(ps, EV_JUMP, surface);
    }
}

fn normalized(v: [f32; 3]) -> [f32; 3] {
    let length = libm::sqrtf(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        v
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Shove the player off the ladder along the way they are facing; facing the
/// ladder, the heading is reflected off it so they go away from the rungs.
fn push_off_ladder(ps: &mut PlayerState, pml: &Pml, push_speed: f32) {
    ps.velocity[2] *= LADDER_RISE_KEPT;

    let flat_forward = normalized([pml.forward[0], pml.forward[1], 0.0]);
    let ladder = ps.v_ladder_vec;
    let heading = if dot(ladder, pml.forward) >= 0.0 {
        flat_forward
    } else {
        let scale = dot(flat_forward, ladder) * LADDER_REFLECT;
        normalized([
            flat_forward[0] + scale * ladder[0],
            flat_forward[1] + scale * ladder[1],
            flat_forward[2] + scale * ladder[2],
        ])
    };

    ps.velocity[0] = push_speed * heading[0];
    ps.velocity[1] = push_speed * heading[1];
    ps.pm_flags &= !pm_flags::LADDER;
}

/// Decide whether a jump may start this frame. A button still held from the
/// last jump is cleared from `cmd`.
pub fn check_gate(
    ps: &PlayerState,
    cmd: &mut UserCmd,
    context: JumpCheckContext,
) -> JumpCheckResult {
    let blocked = ps.pm_flags & NO_JUMP_FLAGS != 0
        || context.time_since_jump < JUMP_COOLDOWN_MS
        || pm_type::is_dead(ps.pm_type);
    if blocked
        || context.stance_surface_type != StanceSurface::Stand as u8
        || cmd.buttons & BUTTON_JUMP == 0
    {
        return JumpCheckResult::NotEligible;
    }

    if context.old_buttons & BUTTON_JUMP != 0 {
        cmd.buttons &= !BUTTON_JUMP;
        return JumpCheckResult::HeldJumpCleared;
    }
    JumpCheckResult::Ready
}

/// Check for a jump and, if one is allowed, start it: launch, tell the
/// clients, and push off a ladder when on one. Slick ground refuses a jump.
pub fn check(
    ps: &mut PlayerState,
    pml: &mut Pml,
    cmd: &mut UserCmd,
    gate: JumpCheckContext,
    launch: JumpLaunchContext,
) -> JumpCheckResult {
    let verdict = check_gate(ps, cmd, gate);
    if verdict != JumpCheckResult::Ready {
        return verdict;
    }
    let ground_flags = pml.ground_trace[4];
    if ground_flags & SURF_SLICK != 0 {
        return JumpCheckResult::NotEligible;
    }

    start(ps, pml, cmd, launch);
    event(ps, ground_flags);
    if ps.pm_flags & pm_flags::LADDER != 0 {
        push_off_ladder(ps, pml, launch.jump_ladder_push_vel);
    }
    JumpCheckResult::Launched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pml;

    const STONE: u32 = 5 << 20;

    fn gate() -> JumpCheckContext {
        JumpCheckContext {
            time_since_jump: 1000,
            old_buttons: 0,
            stance_surface_type: 0,
        }
    }

    fn launch() -> JumpLaunchContext {
        JumpLaunchContext {
            jump_height: 39.0,
            dive: false,
            crouch_jump_scale: 2.0,
            jump_ladder_push_vel: 200.0,
        }
    }

    fn pressing_jump() -> UserCmd {
        UserCmd {
            buttons: BUTTON_JUMP,
            server_time: 4000,
            ..UserCmd::default()
        }
    }

    fn standing() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.gravity = 800;
        ps.ground_entity_num = 3;
        ps.origin = [0.0, 0.0, 100.0];
        ps
    }

    #[test]
    fn clearing_the_state_drops_the_flag_and_the_start_height() {
        let mut ps = standing();
        ps.pm_flags = pm_flags::JUMPING | pm_flags::CROUCH;
        ps.jump_origin_z = 55.0;
        clear_state(&mut ps);
        assert_eq!(ps.pm_flags, pm_flags::CROUCH);
        assert_eq!(ps.jump_origin_z, 0.0);
    }

    #[test]
    fn a_ground_step_is_cut_short_by_the_top_of_the_jump() {
        let mut ps = standing();
        ps.jump_origin_z = 100.0;
        assert_eq!(get_step_height(&ps, [0.0, 0.0, 100.0]), Some(18.0));
        assert_eq!(get_step_height(&ps, [0.0, 0.0, 130.0]), Some(9.0));
        assert_eq!(get_step_height(&ps, [0.0, 0.0, 139.0]), None);
    }

    #[test]
    fn the_top_of_the_jump_is_thirty_nine_above_where_it_began() {
        let mut ps = standing();
        ps.jump_origin_z = 100.0;
        ps.origin[2] = 138.9;
        assert!(!above_max(&ps));
        ps.origin[2] = 139.0;
        assert!(above_max(&ps));
    }

    #[test]
    fn a_rising_jump_is_slowed_to_just_reach_the_top() {
        let mut ps = standing();
        ps.jump_origin_z = 100.0;
        ps.origin[2] = 130.0;
        ps.velocity[2] = 500.0;
        clamp_velocity(&mut ps, [0.0, 0.0, 120.0]);
        // 9 units of room under gravity 800: sqrt(800 * 9 * 2).
        assert!((ps.velocity[2] - 120.0).abs() < 1e-3, "{}", ps.velocity[2]);

        ps.origin[2] = 138.95;
        ps.velocity[2] = 80.0;
        clamp_velocity(&mut ps, [0.0, 0.0, 138.0]);
        assert_eq!(ps.velocity[2], 0.0);

        ps.velocity[2] = 80.0;
        clamp_velocity(&mut ps, [0.0, 0.0, 139.0]);
        assert_eq!(ps.velocity[2], 80.0, "falling or level is left alone");
    }

    #[test]
    fn landing_hard_makes_the_next_jump_weaker_for_a_moment() {
        let mut ps = standing();
        assert_eq!(bo2_land_factor(&ps, false), 1.0);
        ps.pm_time = 5000;
        assert_eq!(bo2_land_factor(&ps, true), 2.5);
        ps.pm_time = 1000;
        assert!((bo2_land_factor(&ps, true) - 1.882_353).abs() < 1e-4);
        ps.pm_time = 0;
        assert_eq!(bo2_land_factor(&ps, true), 1.0);
    }

    #[test]
    fn a_jump_leaves_the_ground_going_up_at_the_launch_speed() {
        let mut ps = standing();
        let mut frame = pml(true);
        let mut cmd = pressing_jump();
        let result = check(&mut ps, &mut frame, &mut cmd, gate(), launch());
        assert_eq!(result, JumpCheckResult::Launched);
        assert!((ps.velocity[2] - libm::sqrtf(800.0 * 78.0)).abs() < 1e-3);
        assert_eq!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert_eq!(
            (frame.walking, frame.ground_plane, frame.almost_ground_plane),
            (0, 0, 0)
        );
        assert_eq!(ps.jump_origin_z, 100.0);
        assert_eq!(ps.jump_time, 4000);
        assert_ne!(ps.pm_flags & pm_flags::JUMPING, 0);
        assert_eq!(ps.aim_spread_scale, 64.0);
    }

    #[test]
    fn a_hard_landing_flag_ends_when_the_jump_starts() {
        let mut ps = standing();
        ps.pm_flags = pm_flags::TIME_HARDLANDING | pm_flags::CROUCH;
        let mut frame = pml(true);
        check(&mut ps, &mut frame, &mut pressing_jump(), gate(), launch());
        assert_eq!(ps.pm_flags & pm_flags::TIME_HARDLANDING, 0);
        assert_ne!(ps.pm_flags & pm_flags::CROUCH, 0);
    }

    #[test]
    fn jumping_again_inside_the_window_divides_the_energy() {
        let mut ps = standing();
        ps.pm_flags = pm_flags::JUMPING;
        ps.pm_time = 100;
        let mut frame = pml(true);
        check(&mut ps, &mut frame, &mut pressing_jump(), gate(), launch());
        assert!((ps.velocity[2] - libm::sqrtf(800.0 * 78.0 / 2.0)).abs() < 1e-3);

        let mut dive = standing();
        dive.pm_flags = pm_flags::JUMPING;
        dive.pm_time = 100;
        let diving = JumpLaunchContext {
            dive: true,
            ..launch()
        };
        check(
            &mut dive,
            &mut pml(true),
            &mut pressing_jump(),
            gate(),
            diving,
        );
        assert!(
            (dive.velocity[2] - libm::sqrtf(800.0 * 78.0)).abs() < 1e-3,
            "a dive is not weakened"
        );
    }

    #[test]
    fn the_gate_refuses_what_stops_a_jump() {
        let ps = standing();
        let mut cmd = pressing_jump();
        assert_eq!(check_gate(&ps, &mut cmd, gate()), JumpCheckResult::Ready);

        let soon = JumpCheckContext {
            time_since_jump: 499,
            ..gate()
        };
        assert_eq!(
            check_gate(&ps, &mut cmd, soon),
            JumpCheckResult::NotEligible
        );
        let at_the_edge = JumpCheckContext {
            time_since_jump: 500,
            ..gate()
        };
        assert_eq!(
            check_gate(&ps, &mut cmd, at_the_edge),
            JumpCheckResult::Ready
        );

        let crouched = JumpCheckContext {
            stance_surface_type: StanceSurface::Crouch as u8,
            ..gate()
        };
        assert_eq!(
            check_gate(&ps, &mut cmd, crouched),
            JumpCheckResult::NotEligible
        );

        let mut blocked = standing();
        for flag in [pm_flags::MANTLE, 0x400, 0x8_0000] {
            blocked.pm_flags = flag;
            assert_eq!(
                check_gate(&blocked, &mut cmd, gate()),
                JumpCheckResult::NotEligible,
                "flag {flag:#x}"
            );
        }
        blocked.pm_flags = 0;
        blocked.pm_type = pm_type::DEAD;
        assert_eq!(
            check_gate(&blocked, &mut cmd, gate()),
            JumpCheckResult::NotEligible
        );

        let mut idle = UserCmd::default();
        assert_eq!(
            check_gate(&ps, &mut idle, gate()),
            JumpCheckResult::NotEligible
        );
    }

    #[test]
    fn a_jump_button_held_from_before_is_cleared_and_needs_a_fresh_press() {
        let ps = standing();
        let mut cmd = pressing_jump();
        let held = JumpCheckContext {
            old_buttons: BUTTON_JUMP,
            ..gate()
        };
        assert_eq!(
            check_gate(&ps, &mut cmd, held),
            JumpCheckResult::HeldJumpCleared
        );
        assert_eq!(cmd.buttons & BUTTON_JUMP, 0);
    }

    #[test]
    fn slick_ground_refuses_the_jump() {
        let mut ps = standing();
        let mut frame = pml(true);
        frame.ground_trace[4] = SURF_SLICK;
        let result = check(&mut ps, &mut frame, &mut pressing_jump(), gate(), launch());
        assert_eq!(result, JumpCheckResult::NotEligible);
        assert_eq!(ps.ground_entity_num, 3);
    }

    #[test]
    fn the_jump_event_names_the_ground_and_stays_quiet_on_silent_ground() {
        assert_eq!(ground_surface_type(STONE), 5);
        assert_eq!(ground_surface_type(STONE | SURF_NOSTEPS), 0);

        let mut ps = standing();
        event(&mut ps, STONE);
        assert_eq!(
            (ps.predictable_events[0], ps.predictable_event_parms[0]),
            (EV_JUMP, 5)
        );
        let events_before = ps.predictable_event_sequence;
        event(&mut ps, 0);
        assert_eq!(
            ps.predictable_event_sequence, events_before,
            "the default surface adds nothing"
        );
    }

    #[test]
    fn jumping_off_a_ladder_pushes_away_and_lets_go() {
        let mut ps = standing();
        ps.pm_flags = pm_flags::LADDER;
        ps.velocity = [0.0, 0.0, 100.0];
        ps.v_ladder_vec = [1.0, 0.0, 0.0];
        let mut frame = pml(true);
        // Facing along +x with the ladder normal also +x: straight off.
        check(&mut ps, &mut frame, &mut pressing_jump(), gate(), launch());
        assert_eq!(ps.pm_flags & pm_flags::LADDER, 0);
        assert_eq!(ps.predictable_event_parms[0], LADDER_JUMP_PARM as u32);
        assert!((ps.velocity[0] - 200.0).abs() < 1e-3 && ps.velocity[1].abs() < 1e-3);
    }

    #[test]
    fn facing_the_ladder_the_push_is_reflected_away_from_it() {
        let mut ps = standing();
        ps.pm_flags = pm_flags::LADDER;
        ps.v_ladder_vec = [-1.0, 0.0, 0.0];
        push_off_ladder(&mut ps, &pml(true), 200.0);
        assert!((ps.velocity[0] + 200.0).abs() < 1e-3, "{:?}", ps.velocity);
    }
}
