//! Black Ops II's PM_DeadMove: on the ground a dead body loses 20 units a
//! second of speed each frame until it stops (on slide ground it keeps it).
//! Black Ops II runs it for every move type past intermission, so a player
//! down in last stand slows the same way.

use crate::bridge::TickLocals;
use playerstate_t6::game::AvatarState;

use crate::ground::PMF_SLIDING;

/// The speed a dead body loses each frame.
const DEAD_DRAG: f32 = 20.0;

pub(crate) fn dead_move(avatar: &mut AvatarState, tick_vars: &TickLocals) {
    if tick_vars.walking == 0 {
        return;
    }
    let mut speed = crate::project::length(&avatar.velocity);
    if avatar.movement_state_bits & PMF_SLIDING == 0 {
        speed -= DEAD_DRAG;
    }
    if speed <= 0.0 {
        avatar.velocity = Default::default();
        return;
    }
    crate::project::scale_to_unit(&mut avatar.velocity);
    for axis in &mut avatar.velocity {
        *axis *= speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;

    #[test]
    fn a_body_slows_by_20() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [30.0, 40.0, 0.0];
        dead_move(&mut avatar, &tick_vars(true));
        assert!((avatar.velocity[0] - 18.0).abs() < 1e-4 && (avatar.velocity[1] - 24.0).abs() < 1e-4);
        dead_move(&mut avatar, &tick_vars(true));
        dead_move(&mut avatar, &tick_vars(true));
        assert_eq!(avatar.velocity, [0.0; 3]);
    }

    #[test]
    fn in_the_air_or_on_slide_ground_it_keeps_its_speed() {
        let mut avatar = AvatarState::ZERO;
        avatar.velocity = [30.0, 40.0, 0.0];
        dead_move(&mut avatar, &tick_vars(false));
        assert_eq!(avatar.velocity, [30.0, 40.0, 0.0]);
        avatar.movement_state_bits = PMF_SLIDING;
        dead_move(&mut avatar, &tick_vars(true));
        assert!((crate::project::length(&avatar.velocity) - 50.0).abs() < 1e-4);
    }
}
