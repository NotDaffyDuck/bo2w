use playerstate_t6::game::{AvatarState, entity_flag_bits};

fn mix(lo: f32, hi: f32, share: f32) -> f32 {
    lo + (hi - lo) * share
}

fn mix_triple(lo: [f32; 3], hi: [f32; 3], share: f32) -> [f32; 3] {
    std::array::from_fn(|axis| mix(lo[axis], hi[axis], share))
}

fn turn_triple(lo: [f32; 3], hi: [f32; 3], share: f32) -> [f32; 3] {
    std::array::from_fn(|axis| lo[axis] + share * game_math::degree_gap(hi[axis], lo[axis]))
}

/// `from` moved `alpha` of the way to `to`. A teleport flag flip or a change of movement type
/// is a cut, not a motion: `from` is returned as it is.
pub(super) fn blend_avatar(from: &AvatarState, to: &AvatarState, alpha: f32) -> AvatarState {
    let mut out = *from;
    let teleported = (from.e_flags ^ to.e_flags) & entity_flag_bits::TELEPORT != 0;
    if teleported || from.movement_mode != to.movement_mode {
        return out;
    }
    out.origin = mix_triple(from.origin, to.origin, alpha);
    out.velocity = mix_triple(from.velocity, to.velocity, alpha);
    out.look_angles = turn_triple(from.look_angles, to.look_angles, alpha);
    out.view_offset_angles = turn_triple(from.view_offset_angles, to.view_offset_angles, alpha);
    out.tilt_sideways = mix(from.tilt_sideways, to.tilt_sideways, alpha);
    out.eye_height_now = mix(from.eye_height_now, to.eye_height_now, alpha);
    out.weapon_raise_fraction = mix(from.weapon_raise_fraction, to.weapon_raise_fraction, alpha);
    out.sight_spread_factor = mix(from.sight_spread_factor, to.sight_spread_factor, alpha);
    out.run_speed_modifier = mix(
        from.run_speed_modifier,
        to.run_speed_modifier,
        alpha,
    );
    out.takeoff_height = mix(from.takeoff_height, to.takeoff_height, alpha);
    out.mantle_state.yaw = mix(from.mantle_state.yaw, to.mantle_state.yaw, alpha);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin[0] = x;
        state
    }

    #[test]
    fn positions_blend_linearly() {
        let out = blend_avatar(&at(0.0), &at(8.0), 0.25);
        assert_eq!(out.origin[0], 2.0);
    }

    #[test]
    fn a_teleport_flag_flip_is_a_cut() {
        let mut to = at(8.0);
        to.e_flags ^= entity_flag_bits::TELEPORT;
        let out = blend_avatar(&at(0.0), &to, 0.5);
        assert_eq!(out.origin[0], 0.0);
    }

    #[test]
    fn a_movement_type_change_is_a_cut() {
        let mut to = at(8.0);
        to.movement_mode += 1;
        let out = blend_avatar(&at(0.0), &to, 0.5);
        assert_eq!(out.origin[0], 0.0);
    }

    #[test]
    fn angles_go_the_short_way_round() {
        let mut from = at(0.0);
        let mut to = at(0.0);
        from.look_angles[1] = 350.0;
        to.look_angles[1] = 10.0;
        let yaw = blend_avatar(&from, &to, 0.5).look_angles[1].rem_euclid(360.0);
        assert!(yaw > 359.0 || yaw < 1.0, "yaw went the long way: {yaw}");
    }
}
