//! Sprint: how long it lasts, when it ends, and how much is left.

use playerstate_t6::game::{PlayerState, UserCmd, pm_flags};

/// The sprint button in `UserCmd::buttons`.
const BUTTON_SPRINT: u32 = 0x2;

/// The perk bit (in `perks[0]`) that makes sprinting last forever.
pub const PERK_MARATHON: u32 = 0x0200_0000;

/// A sprint can never be longer than this many milliseconds.
const SPRINT_TIME_CAP_MS: i32 = 0x3fff;

/// The server settings and weapon limits sprint reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SprintContext {
    /// How long the held weapon lets the player sprint, in milliseconds.
    pub weapon_max_sprint_time: i32,
    /// Sprinting never runs out.
    pub sprint_forever: bool,
    pub min_sprint_time_seconds: f32,
    pub sprint_delay_seconds: f32,
    /// The forward key has to be held past this to keep a sprint going.
    pub sprint_forward_minimum: i32,
    /// There is room to stand up and start the sprint.
    pub stand_up_clear: bool,
    pub sprint_recharge_pause_seconds: f32,
}

/// What a sprint check decided.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SprintResult {
    /// Nothing changed.
    Unchanged,
    /// A sprint began.
    Started,
    /// The sprint ended.
    Ended,
}

/// Round half away from zero, the way the engine's float-to-int does.
fn round_ms(x: f32) -> i32 {
    if x >= 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// The longest a sprint can run: the weapon's sprint-duration scale (1 when
/// unset) times the base time, capped.
#[must_use]
pub fn get_max_sprint_time(sprint_duration_scale: f32, player_sprint_time_seconds: f32) -> i32 {
    let scale = if sprint_duration_scale > 0.0 {
        sprint_duration_scale
    } else {
        1.0
    };
    round_ms(scale * (player_sprint_time_seconds * 1000.0)).min(SPRINT_TIME_CAP_MS)
}

/// The pause before sprint stamina starts coming back, in milliseconds.
fn recharge_pause_ms(recharge_pause_seconds: f32) -> i32 {
    round_ms(recharge_pause_seconds * 1000.0)
}

/// The forward key is not held far enough to sprint.
#[must_use]
pub fn sprint_forward_below_minimum(forwardmove: i8, forward_minimum: i32) -> bool {
    i32::from(forwardmove) <= forward_minimum
}

/// Stop sprinting: note when it ended, and if the sprint button is still down
/// make the player let go of it before sprinting again.
pub fn end_sprint(ps: &mut PlayerState, cmd: &UserCmd) {
    if ps.pm_flags & pm_flags::SPRINTING == 0 {
        return;
    }
    ps.sprint_state.sprint_delay = 0;
    ps.sprint_state.last_sprint_end = cmd.server_time;
    ps.pm_flags &= !pm_flags::SPRINTING;
    if cmd.buttons & BUTTON_SPRINT != 0 {
        ps.sprint_state.sprint_button_up_required = 1;
    }
}

/// Milliseconds of sprint left at `server_time`: the full time with the
/// marathon perk or no sprint yet, the time still to run during a sprint, and
/// the time recharged since the last one (after the recharge pause) otherwise.
#[must_use]
pub fn sprint_time_remaining(ps: &PlayerState, server_time: i32, context: SprintContext) -> i32 {
    let maximum = context.weapon_max_sprint_time;
    if ps.perks[0] & PERK_MARATHON != 0 {
        return maximum;
    }

    let state = &ps.sprint_state;
    let mut remaining = maximum;
    if !context.sprint_forever && state.last_sprint_start != 0 {
        let start = state.last_sprint_start;
        remaining = if state.last_sprint_end < start {
            // Mid-sprint: what the sprint was given, less what it has used.
            state
                .sprint_start_max_length
                .wrapping_sub(server_time)
                .wrapping_add(start)
        } else {
            // Between sprints: the leftover from the last one plus recharge.
            let leftover = if state.sprint_delay == 0 {
                state
                    .sprint_start_max_length
                    .wrapping_add(state.last_sprint_end.wrapping_mul(-2))
            } else {
                state
                    .last_sprint_end
                    .wrapping_mul(-2)
                    .wrapping_sub(recharge_pause_ms(context.sprint_recharge_pause_seconds))
                    .wrapping_add(state.sprint_start_max_length)
            };
            leftover.wrapping_add(start).wrapping_add(server_time)
        };
    }

    if remaining < 0 {
        remaining = 0;
    }
    if maximum < remaining {
        remaining = maximum;
    }
    remaining
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> SprintContext {
        SprintContext {
            weapon_max_sprint_time: 4000,
            sprint_forever: false,
            min_sprint_time_seconds: 0.0,
            sprint_delay_seconds: 0.0,
            sprint_forward_minimum: 105,
            stand_up_clear: true,
            sprint_recharge_pause_seconds: 0.5,
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
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::SPRINTING;
        let cmd = UserCmd {
            server_time: 900,
            buttons: BUTTON_SPRINT,
            ..UserCmd::default()
        };
        end_sprint(&mut ps, &cmd);
        assert_eq!(ps.pm_flags & pm_flags::SPRINTING, 0);
        assert_eq!(ps.sprint_state.last_sprint_end, 900);
        assert_eq!(ps.sprint_state.sprint_button_up_required, 1);
    }

    #[test]
    fn ending_a_sprint_that_never_began_changes_nothing() {
        let mut ps = PlayerState::ZERO;
        end_sprint(
            &mut ps,
            &UserCmd {
                server_time: 900,
                ..UserCmd::default()
            },
        );
        assert_eq!(ps.sprint_state.last_sprint_end, 0);
    }

    #[test]
    fn nobody_has_used_any_sprint_before_the_first_one() {
        assert_eq!(
            sprint_time_remaining(&PlayerState::ZERO, 5000, context()),
            4000
        );
    }

    #[test]
    fn marathon_and_sprint_forever_never_run_out() {
        let mut ps = PlayerState::ZERO;
        ps.sprint_state.last_sprint_start = 1000;
        ps.sprint_state.sprint_start_max_length = 4000;
        ps.perks[0] = PERK_MARATHON;
        assert_eq!(sprint_time_remaining(&ps, 9000, context()), 4000);
        ps.perks[0] = 0;
        let forever = SprintContext {
            sprint_forever: true,
            ..context()
        };
        assert_eq!(sprint_time_remaining(&ps, 9000, forever), 4000);
    }

    #[test]
    fn a_running_sprint_counts_down_to_zero() {
        let mut ps = PlayerState::ZERO;
        ps.sprint_state.last_sprint_start = 1000;
        ps.sprint_state.sprint_start_max_length = 4000;
        assert_eq!(sprint_time_remaining(&ps, 2500, context()), 2500);
        assert_eq!(sprint_time_remaining(&ps, 6000, context()), 0);
    }
}
