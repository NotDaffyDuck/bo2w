//! Black Ops II's PM_DeadMove: on the ground a dead body loses 20 units a
//! second of speed each frame until it stops (on slide ground it keeps it).
//! Black Ops II runs it for every move type past intermission, so a player
//! down in last stand slows the same way.

use crate::bridge::Pml;
use playerstate_t6::game::PlayerState;

use crate::ground::PMF_SLIDING;

/// The speed a dead body loses each frame.
const DEAD_DRAG: f32 = 20.0;

pub(crate) fn dead_move(ps: &mut PlayerState, pml: &Pml) {
    if pml.walking == 0 {
        return;
    }
    let mut speed = crate::project::length(&ps.velocity);
    if ps.pm_flags & PMF_SLIDING == 0 {
        speed -= DEAD_DRAG;
    }
    if speed <= 0.0 {
        ps.velocity = [0.0; 3];
        return;
    }
    crate::project::normalize(&mut ps.velocity);
    for axis in &mut ps.velocity {
        *axis *= speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pml;

    #[test]
    fn a_body_slows_by_20() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [30.0, 40.0, 0.0];
        dead_move(&mut ps, &pml(true));
        assert!((ps.velocity[0] - 18.0).abs() < 1e-4 && (ps.velocity[1] - 24.0).abs() < 1e-4);
        dead_move(&mut ps, &pml(true));
        dead_move(&mut ps, &pml(true));
        assert_eq!(ps.velocity, [0.0; 3]);
    }

    #[test]
    fn in_the_air_or_on_slide_ground_it_keeps_its_speed() {
        let mut ps = PlayerState::ZERO;
        ps.velocity = [30.0, 40.0, 0.0];
        dead_move(&mut ps, &pml(false));
        assert_eq!(ps.velocity, [30.0, 40.0, 0.0]);
        ps.pm_flags = PMF_SLIDING;
        dead_move(&mut ps, &pml(true));
        assert!((crate::project::length(&ps.velocity) - 50.0).abs() < 1e-4);
    }
}
