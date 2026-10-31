//! Whether the player counts as airborne.

use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState};

use crate::bridge::TickLocals;

/// The pm_types that never count as airborne: a linked player rides whatever
/// carries them, and a downed one is held where they lie.
const NEVER_AIRBORNE: [i32; 2] = [movement_mode::NORMAL_LINKED, movement_mode::LASTSTAND];

/// In the air: nothing underfoot, no ground plane near, and not a type that
/// is held in place.
pub fn is_in_air(avatar: &AvatarState, tick_vars: &TickLocals) -> bool {
    let standing_on_something = avatar.standing_on != NO_ENTITY_SLOT;
    let carried = NEVER_AIRBORNE.contains(&avatar.movement_mode);
    !standing_on_something && !carried && tick_vars.near_floor_contact == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::tick_vars;

    fn falling() -> AvatarState {
        AvatarState {
            standing_on: NO_ENTITY_SLOT,
            ..AvatarState::ZERO
        }
    }

    #[test]
    fn a_player_with_nothing_beneath_is_in_the_air() {
        assert!(is_in_air(&falling(), &tick_vars(false)));
    }

    #[test]
    fn standing_on_an_entity_or_near_the_ground_is_not_the_air() {
        let mut avatar = falling();
        avatar.standing_on = 5;
        assert!(!is_in_air(&avatar, &tick_vars(false)));
        let mut near = tick_vars(false);
        near.near_floor_contact = 1;
        assert!(!is_in_air(&falling(), &near));
    }

    #[test]
    fn spectators_never_fall() {
        for movement_mode in [1, 7] {
            let mut avatar = falling();
            avatar.movement_mode = movement_mode;
            assert!(!is_in_air(&avatar, &tick_vars(false)));
        }
    }
}
