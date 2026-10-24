//! Standing, crouching and prone: eye heights, box heights and how fast each
//! stance walks.

use playerstate_t6::game::PlayerState;

/// The eye heights a stance settles at, and the targets the view lerps to.
pub mod view_height {
    pub const PRONE: i32 = 0x0b;
    pub const LAST_STAND: i32 = 0x16;
    pub const CROUCH: i32 = 0x28;
    pub const STAND: i32 = 0x3c;
}

/// The top of the player's box in each stance.
pub const STAND_MAXS_Z: f32 = 70.0;
pub const CROUCH_MAXS_Z: f32 = 50.0;
pub const PRONE_MAXS_Z: f32 = 30.0;

/// Walk-speed factors of the settled stances.
const PRONE_SPEED: f32 = 0.15;
const CROUCH_SPEED: f32 = 0.65;

/// Which stance the player's footsteps and surface sounds go by; the numbers
/// are the order the sound tables list them in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StanceSurface {
    Stand = 0,
    Prone = 1,
    Crouch = 2,
    LastStand = 3,
}

/// What a stance update did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StanceChange {
    Unchanged,
    Changed,
    EnteredProne,
}

/// The stance the eye is headed for.
#[must_use]
pub fn stance_surface_type(ps: &PlayerState) -> StanceSurface {
    match ps.view_height_target {
        view_height::LAST_STAND => StanceSurface::LastStand,
        view_height::CROUCH => StanceSurface::Crouch,
        view_height::PRONE => StanceSurface::Prone,
        _ => StanceSurface::Stand,
    }
}

/// How long the eye takes to travel to `target`, in milliseconds. Dropping
/// into a crouch is quicker than rising out of one; `prone_ms`, when set,
/// replaces the time for the crouch-to-prone stage.
fn lerp_duration(target: i32, going_down: bool, prone_ms: i32) -> i32 {
    match target {
        view_height::PRONE if prone_ms > 0 => prone_ms,
        view_height::PRONE => 400,
        view_height::CROUCH if going_down => 200,
        view_height::CROUCH => 400,
        _ => 200,
    }
}

/// How far into a stance change the clock is, from just after the start to
/// done (1); `None` before it starts or when it has not moved yet.
fn progress(ps: &PlayerState, server_time: i32, duration_ms: i32) -> Option<f32> {
    let elapsed = server_time.wrapping_sub(ps.view_height_lerp_time) as f32;
    let fraction = elapsed / duration_ms as f32;
    if !(fraction >= 0.0) {
        return None;
    }
    let fraction = if fraction > 1.0 { 1.0 } else { fraction };
    (fraction != 0.0).then_some(fraction)
}

/// The walk-speed factor of the player's stance: the settled factor, or a
/// blend while the eye is still travelling between crouch and prone.
#[must_use]
pub fn stance_speed_scale(
    ps: &PlayerState,
    server_time: i32,
    last_stand_scale: f32,
    prone_lerp_ms: i32,
) -> f32 {
    let lerping = ps.view_height_lerp_time != 0;

    if lerping && ps.view_height_lerp_target == view_height::PRONE {
        let duration = lerp_duration(view_height::PRONE, true, prone_lerp_ms);
        if let Some(fraction) = progress(ps, server_time, duration) {
            return fraction * PRONE_SPEED + (1.0 - fraction) * CROUCH_SPEED;
        }
    }

    if lerping && ps.view_height_lerp_target == view_height::CROUCH && ps.view_height_lerp_down == 0
    {
        let duration = lerp_duration(view_height::CROUCH, false, prone_lerp_ms);
        if let Some(fraction) = progress(ps, server_time, duration) {
            return fraction * CROUCH_SPEED + (1.0 - fraction) * PRONE_SPEED;
        }
    }

    match stance_surface_type(ps) {
        StanceSurface::LastStand => last_stand_scale,
        StanceSurface::Crouch => CROUCH_SPEED,
        StanceSurface::Prone => PRONE_SPEED,
        StanceSurface::Stand => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading_to(target: i32) -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.view_height_target = target;
        ps
    }

    #[test]
    fn each_settled_stance_has_its_own_speed() {
        assert_eq!(
            stance_speed_scale(&heading_to(view_height::STAND), 0, 0.3, 600),
            1.0
        );
        assert_eq!(
            stance_speed_scale(&heading_to(view_height::CROUCH), 0, 0.3, 600),
            0.65
        );
        assert_eq!(
            stance_speed_scale(&heading_to(view_height::PRONE), 0, 0.3, 600),
            0.15
        );
        assert_eq!(
            stance_speed_scale(&heading_to(view_height::LAST_STAND), 0, 0.3, 600),
            0.3
        );
    }

    #[test]
    fn the_surface_follows_the_eye_target() {
        assert_eq!(
            stance_surface_type(&heading_to(view_height::PRONE)),
            StanceSurface::Prone
        );
        assert_eq!(
            stance_surface_type(&heading_to(view_height::LAST_STAND)),
            StanceSurface::LastStand
        );
        assert_eq!(stance_surface_type(&heading_to(0)), StanceSurface::Stand);
    }

    #[test]
    fn going_prone_slows_from_crouch_speed_over_the_prone_time() {
        let mut ps = heading_to(view_height::PRONE);
        ps.view_height_lerp_time = 1000;
        ps.view_height_lerp_target = view_height::PRONE;
        // Half of the 600 ms stage in: halfway between 0.65 and 0.15.
        let halfway = stance_speed_scale(&ps, 1300, 0.3, 600);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
        // Past the end it holds the prone speed.
        assert_eq!(stance_speed_scale(&ps, 5000, 0.3, 600), 0.15);
    }

    #[test]
    fn without_a_prone_time_the_stage_takes_four_hundred() {
        let mut ps = heading_to(view_height::PRONE);
        ps.view_height_lerp_time = 1000;
        ps.view_height_lerp_target = view_height::PRONE;
        let halfway = stance_speed_scale(&ps, 1200, 0.3, 0);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
    }

    #[test]
    fn rising_from_prone_to_a_crouch_speeds_up() {
        let mut ps = heading_to(view_height::CROUCH);
        ps.view_height_lerp_time = 1000;
        ps.view_height_lerp_target = view_height::CROUCH;
        ps.view_height_lerp_down = 0;
        let halfway = stance_speed_scale(&ps, 1200, 0.3, 600);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
    }

    #[test]
    fn a_change_that_has_not_started_yet_uses_the_settled_speed() {
        let mut ps = heading_to(view_height::PRONE);
        ps.view_height_lerp_time = 1000;
        ps.view_height_lerp_target = view_height::PRONE;
        assert_eq!(stance_speed_scale(&ps, 1000, 0.3, 600), 0.15);
        assert_eq!(stance_speed_scale(&ps, 900, 0.3, 600), 0.15);
    }
}
