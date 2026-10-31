use playerstate_t6::game::AvatarState;

/// Blend two consecutive player states by `blend_t` (clamped below at 0, not above:
/// a late frame extrapolates). Angles blend along the short way round; the bob cycle wraps at 256.
/// Everything not named here is taken from `next`.
pub(crate) fn blend_participant_state(
    previous: &AvatarState,
    next: &AvatarState,
    blend_t: f32,
) -> AvatarState {
    let t = blend_t.max(0.0);
    let lerp = |from: f32, to: f32| from + (to - from) * t;
    // Angles go the short way round.
    let slerp = |from: f32, to: f32| from + game_math::degree_gap(to, from) * t;
    let per_axis = |from: &[f32; 3], to: &[f32; 3], f: &dyn Fn(f32, f32) -> f32| {
        std::array::from_fn(|axis| f(from[axis], to[axis]))
    };

    // The bob cycle counts up and wraps at 256, so a smaller value ahead means it wrapped.
    let wrapped_target = next.sway_phase
        + if next.sway_phase < previous.sway_phase {
            256
        } else {
            0
        };
    let sway_phase = previous.sway_phase + ((wrapped_target - previous.sway_phase) as f32 * t) as i32;

    AvatarState {
        sway_phase,
        tilt_sideways: lerp(previous.tilt_sideways, next.tilt_sideways),
        sight_spread_factor: lerp(previous.sight_spread_factor, next.sight_spread_factor),
        weapon_raise_fraction: lerp(previous.weapon_raise_fraction, next.weapon_raise_fraction),
        eye_height_now: lerp(previous.eye_height_now, next.eye_height_now),
        origin: per_axis(&previous.origin, &next.origin, &lerp),
        velocity: per_axis(&previous.velocity, &next.velocity, &lerp),
        look_angles: per_axis(&previous.look_angles, &next.look_angles, &slerp),
        view_offset_angles: per_axis(&previous.view_offset_angles, &next.view_offset_angles, &slerp),
        ..*next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(x: f32, bob: i32, yaw: f32) -> AvatarState {
        let mut state = AvatarState::ZERO;
        state.origin[0] = x;
        state.sway_phase = bob;
        state.look_angles[1] = yaw;
        state
    }

    #[test]
    fn halfway_is_the_midpoint() {
        let out = blend_participant_state(&state(0.0, 10, 0.0), &state(10.0, 20, 0.0), 0.5);
        assert_eq!(out.origin[0], 5.0);
        assert_eq!(out.sway_phase, 15);
    }

    #[test]
    fn the_bob_cycle_wraps_forward_not_backward() {
        let out = blend_participant_state(&state(0.0, 250, 0.0), &state(0.0, 10, 0.0), 0.5);
        assert_eq!(out.sway_phase, 250 + 8);
    }

    #[test]
    fn yaw_takes_the_short_way_across_the_seam() {
        let out = blend_participant_state(&state(0.0, 0, 350.0), &state(0.0, 0, 10.0), 0.5);
        let yaw = out.look_angles[1].rem_euclid(360.0);
        assert!(yaw > 359.0 || yaw < 1.0, "yaw went the long way: {yaw}");
    }

    #[test]
    fn a_negative_fraction_is_treated_as_zero() {
        let out = blend_participant_state(&state(1.0, 0, 0.0), &state(9.0, 0, 0.0), -3.0);
        assert_eq!(out.origin[0], 1.0);
    }
}
