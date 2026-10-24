//! How fast the player's input asks them to walk.

use playerstate_t6::consts::pm_type;
use playerstate_t6::game::{PlayerState, UserCmd, pm_flags};

use crate::bridge::stance::view_height;
use crate::bridge::stance_speed_scale;

/// A key held at full travel; the input is scaled against this.
const FULL_KEY: f32 = 127.0;
/// What leaning, or aiming while sprinting, slows the walk to.
const SLOW_WALK: f32 = 0.4;
/// The no-clip and ufo flying speeds, as multiples of the player's speed.
const NOCLIP_SCALE: f32 = 3.0;
const UFO_SCALE: f32 = 6.0;

/// The server-side settings the walk scale reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CmdScaleWalkContext {
    pub player_back_speed_scale: f32,
    pub player_strafe_speed_scale: f32,
    pub player_sprint_speed_scale: f32,
    pub player_last_stand_crawl_speed_scale: f32,
    pub weapon_move_speed_scale: f32,
    pub weapon_ads_move_speed_scale: f32,
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
pub fn cmd_scale_walk(ps: &PlayerState, cmd: &UserCmd, context: CmdScaleWalkContext) -> f32 {
    let flags = ps.pm_flags;
    let prone_and_aiming = flags & pm_flags::PRONE != 0 && ps.f_weapon_pos_frac > 0.0;

    let Some(mut scale) = input_scale(cmd, &context, ps.speed as f32) else {
        return 0.0;
    };

    let slowed = flags & pm_flags::WALKING != 0 || ps.leanf != 0.0 || prone_and_aiming;
    if slowed {
        scale *= SLOW_WALK;
    }

    // Sprint speed only counts while the eye is headed for standing height.
    if flags & pm_flags::SPRINTING != 0 && ps.view_height_target == view_height::STAND {
        scale *= context.player_sprint_speed_scale;
    }

    scale *= match ps.pm_type {
        pm_type::NOCLIP => NOCLIP_SCALE,
        pm_type::UFO => UFO_SCALE,
        _ => stance_speed_scale(
            ps,
            cmd.server_time,
            context.player_last_stand_crawl_speed_scale,
            context.prone_lerp_ms,
        ),
    };

    if ps.weapon != 0 {
        let moving_scale_applies = context.weapon_move_speed_scale > 0.0
            && flags & pm_flags::WALKING == 0
            && !prone_and_aiming;
        if moving_scale_applies {
            scale *= context.weapon_move_speed_scale;
        } else if context.weapon_ads_move_speed_scale > 0.0 {
            scale *= context.weapon_ads_move_speed_scale;
        }
    }

    if flags & pm_flags::SHELLSHOCKED != 0 {
        scale *= context.shellshock_movement_scale;
    }

    scale * ps.move_speed_scale_multiplier
}

/// The strongest held key against full travel, scaled to `speed`. Back and
/// strafe keys count for their own speed scales; a diagonal is divided by its
/// length so it is no faster than a straight line. `None` with no keys held.
fn input_scale(cmd: &UserCmd, context: &CmdScaleWalkContext, speed: f32) -> Option<f32> {
    let forward = f32::from(cmd.forwardmove);
    let right = f32::from(cmd.rightmove);
    let forward_part = if cmd.forwardmove < 0 {
        forward * context.player_back_speed_scale
    } else {
        forward
    }
    .abs();
    let right_part = (right * context.player_strafe_speed_scale).abs();
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

    fn settings() -> CmdScaleWalkContext {
        CmdScaleWalkContext {
            player_back_speed_scale: 0.7,
            player_strafe_speed_scale: 0.8,
            player_sprint_speed_scale: 1.5,
            player_last_stand_crawl_speed_scale: 0.3,
            weapon_move_speed_scale: 0.0,
            weapon_ads_move_speed_scale: 0.0,
            shellshock_movement_scale: 1.0,
            prone_lerp_ms: 600,
        }
    }

    fn walker() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.view_height_target = view_height::STAND;
        ps
    }

    fn pushing(forwardmove: i8, rightmove: i8) -> UserCmd {
        UserCmd {
            forwardmove,
            rightmove,
            ..UserCmd::default()
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
        let mut ps = walker();
        ps.pm_flags |= pm_flags::WALKING;
        let slow = cmd_scale_walk(&ps, &pushing(127, 0), settings());
        assert!((slow - FULL_KEY_AT_190 * SLOW_WALK).abs() < 1e-4, "{slow}");
        ps.pm_flags = 0;
        ps.leanf = 0.5;
        assert!((cmd_scale_walk(&ps, &pushing(127, 0), settings()) - slow).abs() < 1e-4);
    }

    #[test]
    fn sprint_only_speeds_up_toward_standing_eye_height() {
        let mut ps = walker();
        ps.pm_flags |= pm_flags::SPRINTING;
        let sprint = FULL_KEY_AT_190 * 1.5;
        assert!((cmd_scale_walk(&ps, &pushing(127, 0), settings()) - sprint).abs() < 1e-4);
        ps.view_height_target = view_height::CROUCH;
        assert!(cmd_scale_walk(&ps, &pushing(127, 0), settings()) < sprint);
    }

    #[test]
    fn the_flying_move_types_run_three_and_six_times_faster() {
        let mut ps = walker();
        ps.pm_type = pm_type::NOCLIP;
        let noclip = cmd_scale_walk(&ps, &pushing(127, 0), settings());
        assert!((noclip - FULL_KEY_AT_190 * 3.0).abs() < 1e-3, "{noclip}");
        ps.pm_type = pm_type::UFO;
        let ufo = cmd_scale_walk(&ps, &pushing(127, 0), settings());
        assert!((ufo - FULL_KEY_AT_190 * 6.0).abs() < 1e-3, "{ufo}");
    }

    #[test]
    fn shellshock_and_the_weapon_scale_in() {
        let mut ps = walker();
        ps.weapon = 4;
        let mut context = settings();
        context.weapon_move_speed_scale = 0.9;
        ps.pm_flags |= pm_flags::SHELLSHOCKED;
        context.shellshock_movement_scale = 0.5;
        let scale = cmd_scale_walk(&ps, &pushing(127, 0), context);
        assert!(
            (scale - FULL_KEY_AT_190 * 0.9 * 0.5).abs() < 1e-3,
            "{scale}"
        );
    }
}
