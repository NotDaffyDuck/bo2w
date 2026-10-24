//! Counting down the timers a move frame owns.

use playerstate_t6::game::{PlayerState, pm_flags};

use crate::bridge::{Pml, jump};

/// The knockback bit, which has no name in `pm_flags` yet.
const TIME_KNOCKBACK: u32 = 0x100;

/// What a `pm_time` that runs out lets go of: the jump slowdown, the hard
/// landing and the knockback.
const PM_TIME_FLAGS: u32 = pm_flags::JUMPING | pm_flags::TIME_HARDLANDING | TIME_KNOCKBACK;

/// Run the frame's `msec` off the movement-lock, legs and torso timers.
pub fn drop_timers(ps: &mut PlayerState, pml: &Pml) {
    if ps.pm_time != 0 {
        if pml.msec < ps.pm_time {
            ps.pm_time -= pml.msec;
        } else {
            if ps.pm_flags & pm_flags::JUMPING != 0 {
                jump::clear_state(ps);
            }
            ps.pm_flags &= !PM_TIME_FLAGS;
            ps.pm_time = 0;
        }
    }
    ps.legs_timer = counted_down(ps.legs_timer, pml.msec);
    ps.torso_timer = counted_down(ps.torso_timer, pml.msec);
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
    use crate::testing::pml;

    #[test]
    fn the_lock_counts_down_and_its_flags_drop_when_it_runs_out() {
        let mut ps = PlayerState::ZERO;
        ps.pm_time = 120;
        ps.pm_flags = pm_flags::TIME_HARDLANDING | pm_flags::WALKING;
        drop_timers(&mut ps, &pml(true));
        assert_eq!(
            (ps.pm_time, ps.pm_flags),
            (70, pm_flags::TIME_HARDLANDING | pm_flags::WALKING)
        );
        drop_timers(&mut ps, &pml(true));
        drop_timers(&mut ps, &pml(true));
        assert_eq!((ps.pm_time, ps.pm_flags), (0, pm_flags::WALKING));
    }

    #[test]
    fn the_jump_slowdown_ends_with_the_lock_and_forgets_the_jump_height() {
        let mut ps = PlayerState::ZERO;
        ps.pm_time = 50;
        ps.pm_flags = pm_flags::JUMPING;
        ps.jump_origin_z = 12.0;
        drop_timers(&mut ps, &pml(true));
        assert_eq!((ps.pm_time, ps.pm_flags, ps.jump_origin_z), (0, 0, 0.0));
    }

    #[test]
    fn the_leg_and_torso_timers_stop_at_zero() {
        let mut ps = PlayerState::ZERO;
        ps.legs_timer = 30;
        ps.torso_timer = 500;
        drop_timers(&mut ps, &pml(true));
        assert_eq!((ps.legs_timer, ps.torso_timer), (0, 450));
        drop_timers(&mut ps, &pml(true));
        assert_eq!(ps.legs_timer, 0);
    }
}
