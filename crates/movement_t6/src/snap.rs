//! The end of Black Ops II's PmoveSingle: when the player moved much less
//! than the velocity says (blocked), the velocity becomes what was really
//! moved; then every part of it is cut to a whole number. BO2's
//! Sys_SnapVector cuts toward zero (189.7 becomes 189, -189.7 becomes -189),
//! where Modern Warfare 2's rounds to the nearest.

use crate::bridge::Pml;
use playerstate_t6::game::PlayerState;

/// The velocity is replaced when the real move's speed squared is under
/// this share of the velocity's.
const MOVE_RATIO: f32 = 0.25;

pub(crate) fn end_tick_velocity(ps: &mut PlayerState, pml: &Pml) {
    let dt = pml.frametime;
    let moved = [
        ps.origin[0] - pml.previous_origin[0],
        ps.origin[1] - pml.previous_origin[1],
        ps.origin[2] - pml.previous_origin[2],
    ];
    let moved_sq = (moved[1] * moved[1] + moved[0] * moved[0] + moved[2] * moved[2]) / (dt * dt);
    let v = ps.velocity;
    if moved_sq < (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) * MOVE_RATIO {
        let inv = 1.0 / dt;
        ps.velocity = [moved[0] * inv, moved[1] * inv, moved[2] * inv];
    }
    snap_vector(&mut ps.velocity);
}

/// Sys_SnapVector: each part cut toward zero to a whole number.
fn snap_vector(v: &mut [f32; 3]) {
    for part in v {
        *part = *part as i32 as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pml(moved_from: [f32; 3]) -> Pml {
        let mut pml = crate::testing::pml(true);
        pml.frametime = 0.05;
        pml.previous_origin = moved_from;
        pml
    }

    #[test]
    fn the_velocity_is_cut_toward_zero() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [189.7, -189.7, 0.6];
        ps.origin = [9.485, -9.485, 0.03];
        end_tick_velocity(&mut ps, &pml([0.0; 3]));
        assert_eq!(ps.velocity, [189.0, -189.0, 0.0]);
    }

    #[test]
    fn a_blocked_player_keeps_what_was_really_moved() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [200.0, 0.0, 0.0];
        // Moved 4 units in 50 ms: 80 a second, under half the velocity.
        ps.origin = [4.0, 0.0, 0.0];
        end_tick_velocity(&mut ps, &pml([0.0; 3]));
        assert_eq!(ps.velocity, [80.0, 0.0, 0.0]);
        // Moved 6 units: 120 a second, over half, so the velocity stays.
        ps.velocity = [200.0, 0.0, 0.0];
        ps.origin = [6.0, 0.0, 0.0];
        end_tick_velocity(&mut ps, &pml([0.0; 3]));
        assert_eq!(ps.velocity, [200.0, 0.0, 0.0]);
    }
}
