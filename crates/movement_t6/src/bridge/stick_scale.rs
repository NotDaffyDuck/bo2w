//! How fast the player's input asks them to walk.

use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};

use crate::bridge::eye_level;
use crate::bridge::posture_pace_ratio;

/// A key held at full travel; the input is scaled against this.
const FULL_KEY: f32 = 127.0;
/// What leaning, or aiming while sprinting, slows the walk to.
const SLOW_WALK: f32 = 0.4;
/// The no-clip and ufo flying speeds, as multiples of the player's speed.
const NOCLIP_SCALE: f32 = 3.0;
const UFO_SCALE: f32 = 6.0;

/// The server-side settings the walk scale reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrideScaleArgs {
    pub retreat_pace_ratio: f32,
    pub sidestep_pace_ratio: f32,
    pub dash_pace_ratio: f32,
    pub downed_crawl_pace_ratio: f32,
    pub gun_stride_ratio: f32,
    pub scoped_carry_pace: f32,
    /// Walking speed scale while shellshocked (1 = not slowed).
    pub shellshock_movement_scale: f32,
    /// How long the crouch-to-prone stage of an eye-height change takes, in
    /// ms (BO2 600). 0 = the old 400.
    pub prone_lerp_ms: i32,
}

/// The factor to multiply the wish speed by for this frame's input: the held
/// keys against full travel, slowed by lean, aim, stance, weapon and
/// shellshock, sped up by sprint and by the flying move types.
#[must_use]
pub fn cmd_scale_walk(avatar: &AvatarState, ucmd: &InputCommand, frame_ctx: StrideScaleArgs) -> f32 {
    let flags = avatar.movement_state_bits;
    let prone_and_aiming = flags & movement_state_bits::LYING_FLAT != 0 && avatar.weapon_raise_fraction > 0.0;

    let Some(mut scale) = input_scale(ucmd, &frame_ctx, avatar.speed as f32) else {
        return 0.0;
    };

    let slowed = flags & movement_state_bits::WALKING != 0 || avatar.tilt_sideways != 0.0 || prone_and_aiming;
    if slowed {
        scale *= SLOW_WALK;
    }

    // Sprint speed only counts while the eye is headed for standing height.
    if flags & movement_state_bits::SPRINTING != 0 && avatar.eye_height_goal == eye_level::UPRIGHT {
        scale *= frame_ctx.dash_pace_ratio;
    }

    scale *= match avatar.movement_mode {
        movement_mode::NOCLIP => NOCLIP_SCALE,
        movement_mode::UFO => UFO_SCALE,
        _ => posture_pace_ratio(
            avatar,
            ucmd.authority_clock_ms,
            frame_ctx.downed_crawl_pace_ratio,
            frame_ctx.prone_lerp_ms,
        ),
    };

    if avatar.weapon != 0 {
        let moving_scale_applies = frame_ctx.gun_stride_ratio > 0.0
            && flags & movement_state_bits::WALKING == 0
            && !prone_and_aiming;
        if moving_scale_applies {
            scale *= frame_ctx.gun_stride_ratio;
        } else if frame_ctx.scoped_carry_pace > 0.0 {
            scale *= frame_ctx.scoped_carry_pace;
        }
    }

    if flags & movement_state_bits::SHELLSHOCKED != 0 {
        scale *= frame_ctx.shellshock_movement_scale;
    }

    scale * avatar.run_speed_modifier
}

/// The strongest held key against full travel, scaled to `speed`. Back and
/// strafe keys count for their own speed scales; a diagonal is divided by its
/// length so it is no faster than a straight line. `None` with no keys held.
fn input_scale(ucmd: &InputCommand, frame_ctx: &StrideScaleArgs, speed: f32) -> Option<f32> {
    let forward = f32::from(ucmd.thrust_ahead);
    let right = f32::from(ucmd.thrust_lateral);
    let forward_part = if ucmd.thrust_ahead < 0 {
        forward * frame_ctx.retreat_pace_ratio
    } else {
        forward
    }
    .abs();
    let right_part = (right * frame_ctx.sidestep_pace_ratio).abs();
    let strongest = if right_part > forward_part {
        right_part
    } else {
        forward_part
    };
    if strongest == 0.0 {
        return None;
    }
    let length = libm::sqrtf(forward * forward + right * right);
    Some((strongest * speed) / (length * FULL_KEY))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> StrideScaleArgs {
        StrideScaleArgs {
            retreat_pace_ratio: 0.7,
            sidestep_pace_ratio: 0.8,
            dash_pace_ratio: 1.5,
            downed_crawl_pace_ratio: 0.3,
            gun_stride_ratio: 0.0,
            scoped_carry_pace: 0.0,
            shellshock_movement_scale: 1.0,
            prone_lerp_ms: 600,
        }
    }

    fn walker() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.eye_height_goal = eye_level::UPRIGHT;
        avatar
    }

    fn pushing(thrust_ahead: i8, thrust_lateral: i8) -> InputCommand {
        InputCommand {
            thrust_ahead,
            thrust_lateral,
            ..InputCommand::default()
        }
    }

    /// A full key at speed 190: the walker's speed spread over the key travel.
    const FULL_KEY_AT_190: f32 = 190.0 / FULL_KEY;

    #[test]
    fn no_keys_means_no_movement() {
        assert_eq!(cmd_scale_walk(&walker(), &pushing(0, 0), settings()), 0.0);
    }

    #[test]
    fn full_forward_is_the_player_speed_over_the_key_travel() {
        let scale = cmd_scale_walk(&walker(), &pushing(127, 0), settings());
        assert!((scale - FULL_KEY_AT_190).abs() < 1e-4, "{scale}");
    }

    #[test]
    fn backwards_is_slowed_by_the_back_scale() {
        let scale = cmd_scale_walk(&walker(), &pushing(-127, 0), settings());
        assert!((scale - FULL_KEY_AT_190 * 0.7).abs() < 1e-4, "{scale}");
    }

    #[test]
    fn how_far_the_key_is_pushed_does_not_change_the_scale() {
        let scale = cmd_scale_walk(&walker(), &pushing(64, 0), settings());
        assert!((scale - FULL_KEY_AT_190).abs() < 1e-4, "{scale}");
    }

    #[test]
    fn a_diagonal_is_no_faster_than_the_strongest_key() {
        let diagonal = cmd_scale_walk(&walker(), &pushing(127, 127), settings());
        let straight = cmd_scale_walk(&walker(), &pushing(127, 0), settings());
        assert!(diagonal < straight);
    }

    #[test]
    fn walking_slow_and_leaning_cut_the_speed_to_forty_percent() {
        let mut avatar = walker();
        avatar.movement_state_bits |= movement_state_bits::WALKING;
        let slow = cmd_scale_walk(&avatar, &pushing(127, 0), settings());
        assert!((slow - FULL_KEY_AT_190 * SLOW_WALK).abs() < 1e-4, "{slow}");
        avatar.movement_state_bits = 0;
        avatar.tilt_sideways = 0.5;
        assert!((cmd_scale_walk(&avatar, &pushing(127, 0), settings()) - slow).abs() < 1e-4);
    }

    #[test]
    fn sprint_only_speeds_up_toward_standing_eye_height() {
        let mut avatar = walker();
        avatar.movement_state_bits |= movement_state_bits::SPRINTING;
        let sprint = FULL_KEY_AT_190 * 1.5;
        assert!((cmd_scale_walk(&avatar, &pushing(127, 0), settings()) - sprint).abs() < 1e-4);
        avatar.eye_height_goal = eye_level::DUCK_BUTTON_BIT;
        assert!(cmd_scale_walk(&avatar, &pushing(127, 0), settings()) < sprint);
    }

    #[test]
    fn the_flying_move_types_run_three_and_six_times_faster() {
        let mut avatar = walker();
        avatar.movement_mode = movement_mode::NOCLIP;
        let noclip = cmd_scale_walk(&avatar, &pushing(127, 0), settings());
        assert!((noclip - FULL_KEY_AT_190 * 3.0).abs() < 1e-3, "{noclip}");
        avatar.movement_mode = movement_mode::UFO;
        let ufo = cmd_scale_walk(&avatar, &pushing(127, 0), settings());
        assert!((ufo - FULL_KEY_AT_190 * 6.0).abs() < 1e-3, "{ufo}");
    }

    #[test]
    fn shellshock_and_the_weapon_scale_in() {
        let mut avatar = walker();
        avatar.weapon = 4;
        let mut frame_ctx = settings();
        frame_ctx.gun_stride_ratio = 0.9;
        avatar.movement_state_bits |= movement_state_bits::SHELLSHOCKED;
        frame_ctx.shellshock_movement_scale = 0.5;
        let scale = cmd_scale_walk(&avatar, &pushing(127, 0), frame_ctx);
        assert!(
            (scale - FULL_KEY_AT_190 * 0.9 * 0.5).abs() < 1e-3,
            "{scale}"
        );
    }
}
