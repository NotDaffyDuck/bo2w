//! Sprint: how long it lasts, when it ends, and how much is left.

use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};

/// The sprint button in `InputCommand::buttons`.
const INTENT_DASH_BIT: u32 = 0x2;

/// The perk (specialty_unlimitedsprint) that makes sprinting last forever.
pub use playerstate_t6::perks::PERK_UNLIMITEDSPRINT;

/// A sprint can never be longer than this many milliseconds.
const SPRINT_TIME_CAP_MS: i32 = 0x3fff;

/// The server settings and weapon limits sprint reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DashArgs {
    /// How long the held weapon lets the player sprint, in milliseconds.
    pub gun_dash_cap_secs: i32,
    /// Sprinting never runs out.
    pub endless_dash: bool,
    pub dash_least_secs: f32,
    pub dash_warmup_secs: f32,
    /// The forward key has to be held past this to keep a sprint going.
    pub dash_thrust_floor: i32,
    /// There is room to stand up and start the sprint.
    pub rise_space_free: bool,
    pub dash_refill_hold_secs: f32,
}

/// What a sprint check decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashVerdict {
    /// Nothing changed.
    Unchanged,
    /// A sprint began.
    Started,
    /// The sprint ended.
    Ended,
}

/// Round half away from zero, the way the engine's float-to-int does.
fn round_ms(x: f32) -> i32 {
    (x.abs() + 0.5).copysign(x) as i32
}

/// The longest a sprint can run: the weapon's sprint-duration scale (1 when
/// unset) times the base time, capped.
#[must_use]
pub fn get_max_sprint_time(run_time_multiplier: f32, base_secs: f32) -> i32 {
    let scale = if run_time_multiplier > 0.0 {
        run_time_multiplier
    } else {
        1.0
    };
    round_ms(scale * (base_secs * 1000.0)).min(SPRINT_TIME_CAP_MS)
}

/// The pause before sprint stamina starts coming back, in milliseconds.
fn recharge_pause_ms(recharge_pause_seconds: f32) -> i32 {
    round_ms(recharge_pause_seconds * 1000.0)
}

/// The forward key is not held far enough to sprint.
#[must_use]
pub fn sprint_forward_below_minimum(thrust_ahead: i8, thrust_floor: i32) -> bool {
    i32::from(thrust_ahead) <= thrust_floor
}

/// Stop sprinting: note when it ended, and if the sprint button is still down
/// make the player let go of it before sprinting again.
pub fn end_sprint(avatar: &mut AvatarState, ucmd: &InputCommand) {
    if avatar.movement_state_bits & movement_state_bits::SPRINTING == 0 {
        return;
    }
    avatar.sprint_state.sprint_wait = 0;
    avatar.sprint_state.sprint_stop_ms = ucmd.authority_clock_ms;
    avatar.movement_state_bits &= !movement_state_bits::SPRINTING;
    if ucmd.buttons & INTENT_DASH_BIT != 0 {
        avatar.sprint_state.sprint_needs_release = 1;
    }
}

/// Milliseconds of sprint left at `authority_clock_ms`: the full time with the
/// marathon perk or no sprint yet, the time still to run during a sprint, and
/// the time recharged since the last one (after the recharge pause) otherwise.
#[must_use]
pub fn sprint_time_remaining(avatar: &AvatarState, authority_clock_ms: i32, frame_ctx: DashArgs) -> i32 {
    let maximum = frame_ctx.gun_dash_cap_secs;
    if avatar.has_perk_bit(PERK_UNLIMITEDSPRINT) {
        return maximum;
    }

    let state = &avatar.sprint_state;
    let begin = state.sprint_begin_ms;
    let remaining = if frame_ctx.endless_dash || begin == 0 {
        maximum
    } else if state.sprint_stop_ms < begin {
        // Mid-sprint: what the sprint was given, less what it has used.
        state.sprint_launch_cap.wrapping_sub(authority_clock_ms).wrapping_add(begin)
    } else {
        // Between sprints: the leftover from the last one plus recharge; a
        // delayed recharge also waits out its pause.
        let since_stop = state.sprint_stop_ms.wrapping_mul(-2);
        let pause = if state.sprint_wait == 0 {
            0
        } else {
            recharge_pause_ms(frame_ctx.dash_refill_hold_secs)
        };
        state
            .sprint_launch_cap
            .wrapping_add(since_stop)
            .wrapping_sub(pause)
            .wrapping_add(begin)
            .wrapping_add(authority_clock_ms)
    };
    // Never below empty, never above the cap (the cap wins when it is negative).
    remaining.max(0).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_ctx() -> DashArgs {
        DashArgs {
            gun_dash_cap_secs: 4000,
            endless_dash: false,
            dash_least_secs: 0.0,
            dash_warmup_secs: 0.0,
            dash_thrust_floor: 105,
            rise_space_free: true,
            dash_refill_hold_secs: 0.5,
        }
    }

    #[test]
    fn the_longest_sprint_follows_the_scale_and_is_capped() {
        assert_eq!(get_max_sprint_time(0.0, 4.0), 4000);
        assert_eq!(get_max_sprint_time(1.5, 4.0), 6000);
        assert_eq!(get_max_sprint_time(10.0, 4.0), SPRINT_TIME_CAP_MS);
    }

    #[test]
    fn the_forward_key_has_to_go_past_the_minimum() {
        assert!(sprint_forward_below_minimum(105, 105));
        assert!(!sprint_forward_below_minimum(106, 105));
        assert!(sprint_forward_below_minimum(-127, 105));
    }

    #[test]
    fn ending_a_sprint_with_the_button_held_demands_a_release() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::SPRINTING;
        let ucmd = InputCommand {
            authority_clock_ms: 900,
            buttons: INTENT_DASH_BIT,
            ..InputCommand::default()
        };
        end_sprint(&mut avatar, &ucmd);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::SPRINTING, 0);
        assert_eq!(avatar.sprint_state.sprint_stop_ms, 900);
        assert_eq!(avatar.sprint_state.sprint_needs_release, 1);
    }

    #[test]
    fn ending_a_sprint_that_never_began_changes_nothing() {
        let mut avatar = AvatarState::ZERO;
        end_sprint(
            &mut avatar,
            &InputCommand {
                authority_clock_ms: 900,
                ..InputCommand::default()
            },
        );
        assert_eq!(avatar.sprint_state.sprint_stop_ms, 0);
    }

    #[test]
    fn nobody_has_used_any_sprint_before_the_first_one() {
        assert_eq!(
            sprint_time_remaining(&AvatarState::ZERO, 5000, frame_ctx()),
            4000
        );
    }

    #[test]
    fn marathon_and_sprint_forever_never_run_out() {
        let mut avatar = AvatarState::ZERO;
        avatar.sprint_state.sprint_begin_ms = 1000;
        avatar.sprint_state.sprint_launch_cap = 4000;
        avatar.set_perk(PERK_UNLIMITEDSPRINT, true);
        assert_eq!(avatar.perks, [0, 1 << 30]);
        assert_eq!(sprint_time_remaining(&avatar, 9000, frame_ctx()), 4000);
        avatar.set_perk(PERK_UNLIMITEDSPRINT, false);
        let forever = DashArgs {
            endless_dash: true,
            ..frame_ctx()
        };
        assert_eq!(sprint_time_remaining(&avatar, 9000, forever), 4000);
    }

    #[test]
    fn a_running_sprint_counts_down_to_zero() {
        let mut avatar = AvatarState::ZERO;
        avatar.sprint_state.sprint_begin_ms = 1000;
        avatar.sprint_state.sprint_launch_cap = 4000;
        assert_eq!(sprint_time_remaining(&avatar, 2500, frame_ctx()), 2500);
        assert_eq!(sprint_time_remaining(&avatar, 6000, frame_ctx()), 0);
    }
}
