//! The speed a shooter inherits from a moving platform they stand on.

use crate::step_view::StepWorldView;

/// The lookback, in ms, over which a platform's speed is measured.
const WINDOW_MS: i32 = 200;

/// How fast the script mover under `who` is travelling at `now`, measured as its displacement over the last 200 ms
/// scaled up to a second. Zero when they stand on nothing that moves. Rockets, thrown grenades and launched grenades
/// all carry this speed.
pub(crate) fn platform_velocity(world: &StepWorldView, who: crate::SeatNo, now: i32) -> [f32; 3] {
    let Some(ground) = world.avatar(who).map(|ps| ps.standing_on) else {
        return [0.0; 3];
    };
    let Some(mover) = world.script_mover_by_number(ground) else {
        return [0.0; 3];
    };
    let path = crate::ent_slots::move_path_of(&mover.state);
    displacement_per_second(
        entity_t6::evaluate_trajectory(&path, now - WINDOW_MS),
        entity_t6::evaluate_trajectory(&path, now),
    )
}

/// Speed implied by moving from `before` to `after` in a fifth of a second.
fn displacement_per_second(before: [f32; 3], after: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|i| (after[i] - before[i]) * 5.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fifth_of_a_second_scales_up_five_times() {
        assert_eq!(displacement_per_second([0.0; 3], [2.0, -4.0, 0.0]), [10.0, -20.0, 0.0]);
    }

    #[test]
    fn a_platform_that_has_not_moved_gives_nothing() {
        assert_eq!(displacement_per_second([3.0, 4.0, 5.0], [3.0, 4.0, 5.0]), [0.0; 3]);
    }

    #[test]
    fn the_window_is_a_fifth_of_a_second() {
        assert_eq!(WINDOW_MS * 5, 1000);
    }
}
