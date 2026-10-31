//! Counting down the timers a move frame owns.

use playerstate_t6::game::{AvatarState, movement_state_bits};

use crate::bridge::{TickLocals, leap};

/// The knockback bit, which has no name in `movement_state_bits` yet.
const TIME_KNOCKBACK: u32 = 0x100;

/// What a `pm_time` that runs out lets go of: the jump slowdown, the hard
/// landing and the knockback.
const PM_TIME_FLAGS: u32 = movement_state_bits::JUMPING | movement_state_bits::TIME_HARDLANDING | TIME_KNOCKBACK;

/// Run the frame's `msec` off the movement-lock, legs and torso timers.
pub fn drop_timers(avatar: &mut AvatarState, tick_vars: &TickLocals) {
    if avatar.move_timer != 0 {
        if tick_vars.frame_ms < avatar.move_timer {
            avatar.move_timer -= tick_vars.frame_ms;
        } else {
            if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 {
                leap::clear_state(avatar);
            }
            avatar.movement_state_bits &= !PM_TIME_FLAGS;
            avatar.move_timer = 0;
        }
    }
    avatar.lower_body_clock = counted_down(avatar.lower_body_clock, tick_vars.frame_ms);
    avatar.upper_body_clock = counted_down(avatar.upper_body_clock, tick_vars.frame_ms);
}

/// A timer that runs down by `msec` and stops at zero; zero stays zero.
fn counted_down(timer: i32, msec: i32) -> i32 {
    if timer > 0 {
        (timer - msec).max(0)
    } else {
        timer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;

    #[test]
    fn the_lock_counts_down_and_its_flags_drop_when_it_runs_out() {
        let mut avatar = AvatarState::ZERO;
        avatar.move_timer = 120;
        avatar.movement_state_bits = movement_state_bits::TIME_HARDLANDING | movement_state_bits::WALKING;
        drop_timers(&mut avatar, &tick_vars(true));
        assert_eq!(
            (avatar.move_timer, avatar.movement_state_bits),
            (70, movement_state_bits::TIME_HARDLANDING | movement_state_bits::WALKING)
        );
        drop_timers(&mut avatar, &tick_vars(true));
        drop_timers(&mut avatar, &tick_vars(true));
        assert_eq!((avatar.move_timer, avatar.movement_state_bits), (0, movement_state_bits::WALKING));
    }

    #[test]
    fn the_jump_slowdown_ends_with_the_lock_and_forgets_the_jump_height() {
        let mut avatar = AvatarState::ZERO;
        avatar.move_timer = 50;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.takeoff_height = 12.0;
        drop_timers(&mut avatar, &tick_vars(true));
        assert_eq!((avatar.move_timer, avatar.movement_state_bits, avatar.takeoff_height), (0, 0, 0.0));
    }

    #[test]
    fn the_leg_and_torso_timers_stop_at_zero() {
        let mut avatar = AvatarState::ZERO;
        avatar.lower_body_clock = 30;
        avatar.upper_body_clock = 500;
        drop_timers(&mut avatar, &tick_vars(true));
        assert_eq!((avatar.lower_body_clock, avatar.upper_body_clock), (0, 450));
        drop_timers(&mut avatar, &tick_vars(true));
        assert_eq!(avatar.lower_body_clock, 0);
    }
}
