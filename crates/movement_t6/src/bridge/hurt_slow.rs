//! The flinch a hit leaves in the player's walk.

use game_math::heading_of;
use playerstate_t6::game::AvatarState;

/// Milliseconds of flinch each point of damage adds.
const TIME_PER_POINT: f32 = 100.0;
/// The most flinch the timer ever holds.
const MAX_TIME: f32 = 750.0;
/// The walk-speed floor a full timer scales down to.
const MIN_SCALE: f32 = 0.0;

/// How long a stumble lasts, in milliseconds.
pub const HURT_STAGGER_SPAN_MS: i32 = 500;
/// How long a flinch animation lasts, in milliseconds.
pub const HURT_FLINCH_SPAN_MS: i32 = 500;
/// The animation id of the forward flinch; the other three follow it.
pub const CLIP_RECOIL_AHEAD: u8 = 32;

/// Add a hit to the flinch timer and pick the direction the player flinches
/// from: `dir` is the way the hit came in (`None` for no direction).
pub fn update_damage_timer(avatar: &mut AvatarState, hurt: i32, push: Option<[f32; 3]>) {
    let added = (TIME_PER_POINT * hurt as f32) as i32;
    avatar.hurt_clock = avatar.hurt_clock.saturating_add(added);
    if MAX_TIME < avatar.hurt_clock as f32 {
        avatar.hurt_clock = MAX_TIME as i32;
    }
    avatar.hurt_length = avatar.hurt_clock;
    avatar.flinch_yaw_anim = push.map_or(0, |push| flinch_sector(avatar.look_angles[1], push));
}

/// Which way the player flinches: 0 for a hit from ahead or behind-ish, then
/// 1, 2 and 3 for the other quarters, measured from the player's own yaw.
fn flinch_sector(look_heading: f32, dir: [f32; 3]) -> i32 {
    let facing = if look_heading < 0.0 {
        look_heading + 360.0
    } else {
        look_heading
    };
    let mut travel_azimuth = heading_of(dir[0], dir[1]) - (facing as i32) as f32;
    if travel_azimuth < 0.0 {
        travel_azimuth += 360.0;
    }
    if !(45.0..315.0).contains(&travel_azimuth) {
        0
    } else if (135.0..225.0).contains(&travel_azimuth) {
        1
    } else if travel_azimuth < 135.0 {
        2
    } else {
        3
    }
}

/// The walk-speed factor a flinch applies.
#[must_use]
pub fn damage_scale_walk(hurt_clock: i32) -> f32 {
    if hurt_clock == 0 {
        return 1.0;
    }
    (-MIN_SCALE / MAX_TIME) * hurt_clock as f32 + 1.0
}

/// Run one frame off the flinch timer.
pub fn walk_move_drop_damage_timer(avatar: &mut AvatarState, frame_secs: f32) {
    avatar.hurt_clock -= (frame_secs * 1000.0) as i32;
    if avatar.hurt_clock < 1 {
        avatar.hurt_clock = 0;
    }
}

/// Still inside the flinch: more than `window_ms` before the end of
/// `hurt_length`.
#[must_use]
pub fn damage_window_open(hurt_clock: i32, hurt_length: i32, window: i32) -> bool {
    hurt_length.saturating_sub(window).max(0) < hurt_clock
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facing_east() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.look_angles[1] = 0.0;
        avatar
    }

    #[test]
    fn damage_adds_a_hundred_milliseconds_a_point_up_to_the_cap() {
        let mut avatar = facing_east();
        update_damage_timer(&mut avatar, 3, None);
        assert_eq!((avatar.hurt_clock, avatar.hurt_length), (300, 300));
        update_damage_timer(&mut avatar, 50, None);
        assert_eq!((avatar.hurt_clock, avatar.hurt_length), (750, 750));
    }

    #[test]
    fn the_flinch_direction_follows_where_the_hit_came_from() {
        let mut avatar = facing_east();
        for (dir, expected) in [
            ([1.0, 0.0, 0.0], 0),
            ([0.0, 1.0, 0.0], 2),
            ([-1.0, 0.0, 0.0], 1),
            ([0.0, -1.0, 0.0], 3),
        ] {
            update_damage_timer(&mut avatar, 1, Some(dir));
            assert_eq!(avatar.flinch_yaw_anim, expected, "{dir:?}");
        }
        update_damage_timer(&mut avatar, 1, None);
        assert_eq!(avatar.flinch_yaw_anim, 0);
    }

    #[test]
    fn the_timer_runs_down_to_zero_and_the_walk_is_never_slowed_with_a_zero_floor() {
        let mut avatar = facing_east();
        avatar.hurt_clock = 120;
        walk_move_drop_damage_timer(&mut avatar, 0.05);
        assert_eq!(avatar.hurt_clock, 70);
        walk_move_drop_damage_timer(&mut avatar, 0.1);
        assert_eq!(avatar.hurt_clock, 0);
        assert_eq!(damage_scale_walk(0), 1.0);
        assert_eq!(damage_scale_walk(500), 1.0);
    }

    #[test]
    fn the_window_opens_until_the_last_stretch_of_the_flinch() {
        assert!(damage_window_open(400, 500, 200));
        assert!(!damage_window_open(300, 500, 200));
        assert!(damage_window_open(1, 100, 500));
    }
}
