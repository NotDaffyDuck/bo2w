//! The flinch a hit leaves in the player's walk.

use game_math::vec_to_yaw;
use playerstate_t6::game::PlayerState;

/// Milliseconds of flinch each point of damage adds.
const TIME_PER_POINT: f32 = 100.0;
/// The most flinch the timer ever holds.
const MAX_TIME: f32 = 750.0;
/// The walk-speed floor a full timer scales down to.
const MIN_SCALE: f32 = 0.0;

/// How long a stumble lasts, in milliseconds.
pub const PLAYER_DMGTIMER_STUMBLE_TIME_MS: i32 = 500;
/// How long a flinch animation lasts, in milliseconds.
pub const PLAYER_DMGTIMER_FLINCH_TIME_MS: i32 = 500;
/// The animation id of the forward flinch; the other three follow it.
pub const ANIM_MT_FLINCH_FORWARD: u8 = 32;

/// Add a hit to the flinch timer and pick the direction the player flinches
/// from: `dir` is the way the hit came in (`None` for no direction).
pub fn update_damage_timer(ps: &mut PlayerState, damage: i32, dir: Option<[f32; 3]>) {
    let added = (TIME_PER_POINT * damage as f32) as i32;
    ps.damage_timer = ps.damage_timer.saturating_add(added);
    if MAX_TIME < ps.damage_timer as f32 {
        ps.damage_timer = MAX_TIME as i32;
    }
    ps.damage_duration = ps.damage_timer;
    ps.flinch_yaw_anim = dir.map_or(0, |dir| flinch_sector(ps.viewangles[1], dir));
}

/// Which way the player flinches: 0 for a hit from ahead or behind-ish, then
/// 1, 2 and 3 for the other quarters, measured from the player's own yaw.
fn flinch_sector(view_yaw: f32, dir: [f32; 3]) -> i32 {
    let facing = if view_yaw < 0.0 {
        view_yaw + 360.0
    } else {
        view_yaw
    };
    let mut bearing = vec_to_yaw(dir[0], dir[1]) - (facing as i32) as f32;
    if bearing < 0.0 {
        bearing += 360.0;
    }
    if !(45.0..315.0).contains(&bearing) {
        0
    } else if (135.0..225.0).contains(&bearing) {
        1
    } else if bearing < 135.0 {
        2
    } else {
        3
    }
}

/// The walk-speed factor a flinch applies.
#[must_use]
pub fn damage_scale_walk(damage_timer: i32) -> f32 {
    if damage_timer == 0 {
        return 1.0;
    }
    (-MIN_SCALE / MAX_TIME) * damage_timer as f32 + 1.0
}

/// Run one frame off the flinch timer.
pub fn walk_move_drop_damage_timer(ps: &mut PlayerState, frametime_seconds: f32) {
    ps.damage_timer -= (frametime_seconds * 1000.0) as i32;
    if ps.damage_timer < 1 {
        ps.damage_timer = 0;
    }
}

/// Still inside the flinch: more than `window_ms` before the end of
/// `damage_duration`.
#[must_use]
pub fn damage_window_open(damage_timer: i32, damage_duration: i32, window_ms: i32) -> bool {
    damage_duration.saturating_sub(window_ms).max(0) < damage_timer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facing_east() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.viewangles[1] = 0.0;
        ps
    }

    #[test]
    fn damage_adds_a_hundred_milliseconds_a_point_up_to_the_cap() {
        let mut ps = facing_east();
        update_damage_timer(&mut ps, 3, None);
        assert_eq!((ps.damage_timer, ps.damage_duration), (300, 300));
        update_damage_timer(&mut ps, 50, None);
        assert_eq!((ps.damage_timer, ps.damage_duration), (750, 750));
    }

    #[test]
    fn the_flinch_direction_follows_where_the_hit_came_from() {
        let mut ps = facing_east();
        for (dir, expected) in [
            ([1.0, 0.0, 0.0], 0),
            ([0.0, 1.0, 0.0], 2),
            ([-1.0, 0.0, 0.0], 1),
            ([0.0, -1.0, 0.0], 3),
        ] {
            update_damage_timer(&mut ps, 1, Some(dir));
            assert_eq!(ps.flinch_yaw_anim, expected, "{dir:?}");
        }
        update_damage_timer(&mut ps, 1, None);
        assert_eq!(ps.flinch_yaw_anim, 0);
    }

    #[test]
    fn the_timer_runs_down_to_zero_and_the_walk_is_never_slowed_with_a_zero_floor() {
        let mut ps = facing_east();
        ps.damage_timer = 120;
        walk_move_drop_damage_timer(&mut ps, 0.05);
        assert_eq!(ps.damage_timer, 70);
        walk_move_drop_damage_timer(&mut ps, 0.1);
        assert_eq!(ps.damage_timer, 0);
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
