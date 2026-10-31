//! The end of Black Ops II's PmoveSingle: when the player moved much less
//! than the velocity says (blocked), the velocity becomes what was really
//! moved; then every part of it is cut to a whole number. BO2's
//! Sys_SnapVector cuts toward zero (189.7 becomes 189, -189.7 becomes -189),
//! where Modern Warfare 2's rounds to the nearest.

use crate::bridge::TickLocals;
use playerstate_t6::game::AvatarState;

/// The velocity is replaced when the real move's speed squared is under
/// this share of the velocity's.
const MOVE_RATIO: f32 = 0.25;

pub(crate) fn end_tick_velocity(avatar: &mut AvatarState, tick_vars: &TickLocals) {
    let dt = tick_vars.step_millis_elapsed;
    let moved = [
        avatar.origin[0] - tick_vars.prior_tick_position[0],
        avatar.origin[1] - tick_vars.prior_tick_position[1],
        avatar.origin[2] - tick_vars.prior_tick_position[2],
    ];
    let moved_sq = (moved[1] * moved[1] + moved[0] * moved[0] + moved[2] * moved[2]) / (dt * dt);
    let v = avatar.velocity;
    if moved_sq < (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) * MOVE_RATIO {
        let stow_kind = 1.0 / dt;
        avatar.velocity = [moved[0] * stow_kind, moved[1] * stow_kind, moved[2] * stow_kind];
    }
    round_vec_to_grid(&mut avatar.velocity);
}

/// Sys_SnapVector: each part cut toward zero to a whole number.
fn round_vec_to_grid(v: &mut [f32; 3]) {
    for part in v {
        *part = *part as i32 as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_vars(moved_from: [f32; 3]) -> TickLocals {
        let mut tick_vars = crate::testing::tick_vars(true);
        tick_vars.step_millis_elapsed = 0.05;
        tick_vars.prior_tick_position = moved_from;
        tick_vars
    }

    #[test]
    fn the_velocity_is_cut_toward_zero() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [189.7, -189.7, 0.6];
        avatar.origin = [9.485, -9.485, 0.03];
        end_tick_velocity(&mut avatar, &tick_vars([0.0; 3]));
        assert_eq!(avatar.velocity, [189.0, -189.0, 0.0]);
    }

    #[test]
    fn a_blocked_player_keeps_what_was_really_moved() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [200.0, 0.0, 0.0];
        // Moved 4 units in 50 ms: 80 a second, under half the velocity.
        avatar.origin = [4.0, 0.0, 0.0];
        end_tick_velocity(&mut avatar, &tick_vars([0.0; 3]));
        assert_eq!(avatar.velocity, [80.0, 0.0, 0.0]);
        // Moved 6 units: 120 a second, over half, so the velocity stays.
        avatar.velocity = [200.0, 0.0, 0.0];
        avatar.origin = [6.0, 0.0, 0.0];
        end_tick_velocity(&mut avatar, &tick_vars([0.0; 3]));
        assert_eq!(avatar.velocity, [200.0, 0.0, 0.0]);
    }
}
