//! Whether the player counts as airborne.

use playerstate_t6::consts::pm_type;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState};

use crate::bridge::Pml;

/// The pm_types that never count as airborne: a linked player rides whatever
/// carries them, and a downed one is held where they lie.
const NEVER_AIRBORNE: [i32; 2] = [pm_type::NORMAL_LINKED, pm_type::LASTSTAND];

/// In the air: nothing underfoot, no ground plane near, and not a type that
/// is held in place.
pub fn is_in_air(ps: &PlayerState, pml: &Pml) -> bool {
    let standing_on_something = ps.ground_entity_num != ENTITYNUM_NONE;
    let carried = NEVER_AIRBORNE.contains(&ps.pm_type);
    !standing_on_something && !carried && pml.almost_ground_plane == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::pml;

    fn falling() -> PlayerState {
        PlayerState {
            ground_entity_num: ENTITYNUM_NONE,
            ..PlayerState::ZERO
        }
    }

    #[test]
    fn a_player_with_nothing_beneath_is_in_the_air() {
        assert!(is_in_air(&falling(), &pml(false)));
    }

    #[test]
    fn standing_on_an_entity_or_near_the_ground_is_not_the_air() {
        let mut ps = falling();
        ps.ground_entity_num = 5;
        assert!(!is_in_air(&ps, &pml(false)));
        let mut near = pml(false);
        near.almost_ground_plane = 1;
        assert!(!is_in_air(&falling(), &near));
    }

    #[test]
    fn spectators_never_fall() {
        for pm_type in [1, 7] {
            let mut ps = falling();
            ps.pm_type = pm_type;
            assert!(!is_in_air(&ps, &pml(false)));
        }
    }
}
