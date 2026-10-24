//! bo2mp: UFO mode (the mod menu's fly mode, the engine's own pm_type 3,
//! `PM_UFO`): he flies where he looks, through walls, with no gravity.
//! Forward and back follow the view (pitch too), Jump rises, Crouch or Prone
//! sinks, and Sprint flies faster. The client predicts it like any move,
//! so it is smooth.

use playerstate_t6::game::{PlayerState, UserCmd, buttons};

use crate::bridge::Pml;

/// The engine's pm_types for flying through walls (`PM_NOCLIP`, `PM_UFO`).
pub const PM_TYPE_NOCLIP: i32 = 2;
pub const PM_TYPE_UFO: i32 = 3;

/// Top speed (units a second), and with Sprint held.
const SPEED: f32 = 400.0;
const SPRINT_SPEED: f32 = 1100.0;
/// How fast he reaches it and how fast he stops (Quake's noclip numbers:
/// accelerate 10, friction 6 x 1.5).
const ACCELERATE: f32 = 10.0;
const FRICTION: f32 = 9.0;
const STOP_SPEED: f32 = 100.0;

pub fn active(ps: &PlayerState) -> bool {
    matches!(ps.pm_type, PM_TYPE_NOCLIP | PM_TYPE_UFO)
}

pub fn fly(ps: &mut PlayerState, cmd: &UserCmd, pml: &Pml) {
    let dt = pml.frametime;
    let speed = libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1] + ps.velocity[2] * ps.velocity[2]);
    if speed < 1.0 {
        ps.velocity = [0.0; 3];
    } else {
        let drop = speed.max(STOP_SPEED) * FRICTION * dt;
        let scale = (speed - drop).max(0.0) / speed;
        for v in &mut ps.velocity {
            *v *= scale;
        }
    }

    let fmove = f32::from(cmd.forwardmove) / 127.0;
    let smove = f32::from(cmd.rightmove) / 127.0;
    let mut umove = 0.0;
    if cmd.buttons & buttons::JUMP != 0 {
        umove += 1.0;
    }
    if cmd.buttons & (buttons::CROUCH | buttons::PRONE) != 0 {
        umove -= 1.0;
    }
    let mut wish = [0.0f32; 3];
    for i in 0..3 {
        wish[i] = pml.forward[i] * fmove + pml.right[i] * smove;
    }
    wish[2] += umove;
    let len = libm::sqrtf(wish[0] * wish[0] + wish[1] * wish[1] + wish[2] * wish[2]);
    if len > 0.0 {
        let top = if cmd.buttons & buttons::SPRINT != 0 { SPRINT_SPEED } else { SPEED };
        let wishspeed = top * len.min(1.0);
        let dir = [wish[0] / len, wish[1] / len, wish[2] / len];
        let current = ps.velocity[0] * dir[0] + ps.velocity[1] * dir[1] + ps.velocity[2] * dir[2];
        let add = wishspeed - current;
        if add > 0.0 {
            let accel = (ACCELERATE * dt * wishspeed).min(add);
            for i in 0..3 {
                ps.velocity[i] += accel * dir[i];
            }
        }
    }
    for i in 0..3 {
        ps.origin[i] += ps.velocity[i] * dt;
    }
    ps.ground_entity_num = playerstate_t6::game::ENTITYNUM_NONE;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flies_where_he_looks_and_rises_on_jump() {
        let mut ps = PlayerState {
            pm_type: PM_TYPE_UFO,
            ..PlayerState::ZERO
        };
        let pml = Pml {
            forward: [0.0, 0.0, 1.0],
            right: [0.0, -1.0, 0.0],
            up: [1.0, 0.0, 0.0],
            frametime: 0.05,
            msec: 50,
            walking: 0,
            ground_plane: 0,
            almost_ground_plane: 0,
            ground_trace: [0; 11],
            previous_origin: [0.0; 3],
            previous_velocity: [0.0; 3],
            holdrand: 0,
        };
        let mut cmd = UserCmd::default();
        cmd.forwardmove = 127;
        for _ in 0..40 {
            fly(&mut ps, &cmd, &pml);
        }
        assert!(ps.origin[2] > 300.0, "{:?}", ps.origin);
        let z = ps.origin[2];
        cmd.forwardmove = 0;
        cmd.buttons = buttons::CROUCH;
        for _ in 0..40 {
            fly(&mut ps, &cmd, &pml);
        }
        assert!(ps.origin[2] < z, "{:?}", ps.origin);
    }
}
