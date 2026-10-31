//! bo2mp: UFO mode (the mod menu's fly mode, the engine's own movement_mode 3,
//! `PM_UFO`): he flies where he looks, through walls, with no gravity.
//! Forward and back follow the view (pitch too), Jump rises, Crouch or Prone
//! sinks, and Sprint flies faster. The client predicts it like any move,
//! so it is smooth.

use playerstate_t6::game::{AvatarState, InputCommand, buttons};

use crate::bridge::TickLocals;

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

pub fn active(avatar: &AvatarState) -> bool {
    matches!(avatar.movement_mode, PM_TYPE_NOCLIP | PM_TYPE_UFO)
}

pub fn fly(avatar: &mut AvatarState, ucmd: &InputCommand, tick_vars: &TickLocals) {
    let dt = tick_vars.step_millis_elapsed;
    let speed = libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1] + avatar.velocity[2] * avatar.velocity[2]);
    if 1.0 > speed {
        avatar.velocity = Default::default();
    } else {
        let drop = speed.max(STOP_SPEED) * FRICTION * dt;
        let scale = (speed - drop).max(0.0) / speed;
        for v in &mut avatar.velocity {
            *v *= scale;
        }
    }

    let fmove = f32::from(ucmd.thrust_ahead) / 127.0;
    let smove = f32::from(ucmd.thrust_lateral) / 127.0;
    let mut umove = 0.0;
    if ucmd.buttons & buttons::LEAP != 0 {
        umove += 1.0;
    }
    if ucmd.buttons & (buttons::DUCK_BUTTON_BIT | buttons::LYING_FLAT) != 0 {
        umove -= 1.0;
    }
    let mut wish = [0.0f32; 3];
    for i in 0..3 {
        wish[i] = tick_vars.fwd_axis[i] * fmove + tick_vars.right_axis[i] * smove;
    }
    wish[2] += umove;
    let len = libm::sqrtf(wish[0] * wish[0] + wish[1] * wish[1] + wish[2] * wish[2]);
    if len > 0.0 {
        let top = if ucmd.buttons & buttons::SPRINT != 0 { SPRINT_SPEED } else { SPEED };
        let intended_pace = top * len.min(1.0);
        let dir = [wish[0] / len, wish[1] / len, wish[2] / len];
        let current = avatar.velocity[0] * dir[0] + avatar.velocity[1] * dir[1] + avatar.velocity[2] * dir[2];
        let add = intended_pace - current;
        if add > 0.0 {
            let speed_up_rate = (ACCELERATE * dt * intended_pace).min(add);
            for i in 0..3 {
                avatar.velocity[i] += speed_up_rate * dir[i];
            }
        }
    }
    for i in 0..3 {
        avatar.origin[i] += avatar.velocity[i] * dt;
    }
    avatar.standing_on = playerstate_t6::game::NO_ENTITY_SLOT;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flies_where_he_looks_and_rises_on_jump() {
        let mut avatar = AvatarState {
            movement_mode: PM_TYPE_UFO,
            ..AvatarState::ZERO
        };
        let tick_vars = TickLocals {
            fwd_axis: [0.0_f32, 0.0, 1.0],
            right_axis: [0.0_f32, -1.0, 0.0],
            up_axis: [1.0, 0.0, 0.0],
            step_millis_elapsed: 0.05,
            frame_ms: 50,
            walking: 0,
            standing_on_floor: 0,
            near_floor_contact: 0,
            floor_probe: [0; 11],
            prior_tick_position: [0.0; 3],
            prior_tick_speed_vec: [0.0; 3],
            frame_seed: 0_i32,
        };
        let mut ucmd = InputCommand::default();
        ucmd.thrust_ahead = 127;
        for _ in 0..40 {
            fly(&mut avatar, &ucmd, &tick_vars);
        }
        assert!(avatar.origin[2] > 300.0, "{:?}", avatar.origin);
        let z = avatar.origin[2];
        ucmd.thrust_ahead = 0;
        ucmd.buttons = buttons::DUCK_BUTTON_BIT;
        for _ in 0..40 {
            fly(&mut avatar, &ucmd, &tick_vars);
        }
        assert!(avatar.origin[2] < z, "{:?}", avatar.origin);
    }
}
