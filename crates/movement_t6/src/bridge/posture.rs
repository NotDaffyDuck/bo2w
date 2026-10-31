//! Standing, crouching and prone: eye heights, box heights and how fast each
//! stance walks.

use playerstate_t6::game::AvatarState;

/// The eye heights a stance settles at, and the targets the view lerps to.
pub mod eye_level {
    pub const LYING_FLAT: i32 = 0x0b;
    pub const DOWNED_GRACE: i32 = 0x16;
    pub const DUCK_BUTTON_BIT: i32 = 0x28;
    pub const UPRIGHT: i32 = 0x3c;
}

/// The top of the player's box in each stance.
pub const UPRIGHT_HULL_TOP: f32 = 70.0;
pub const SQUAT_HULL_TOP: f32 = 50.0;
pub const BELLY_HULL_TOP: f32 = 30.0;

/// Walk-speed factors of the settled stances.
const PRONE_SPEED: f32 = 0.15;
const CROUCH_SPEED: f32 = 0.65;

/// Which stance the player's footsteps and surface sounds go by; the numbers
/// are the order the sound tables list them in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PostureFooting {
    Stand = 0,
    Prone = 1,
    Crouch = 2,
    DownedGrace = 3,
}

/// What a stance update did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostureShift {
    Unchanged,
    Changed,
    WentFlat,
}

/// The stance the eye is headed for.
#[must_use]
pub fn posture_footing_kind(avatar: &AvatarState) -> PostureFooting {
    match avatar.eye_height_goal {
        eye_level::DOWNED_GRACE => PostureFooting::DownedGrace,
        eye_level::DUCK_BUTTON_BIT => PostureFooting::Crouch,
        eye_level::LYING_FLAT => PostureFooting::Prone,
        _ => PostureFooting::Stand,
    }
}

/// How long the eye takes to travel to `target`, in milliseconds. Dropping
/// into a crouch is quicker than rising out of one; `prone_ms`, when set,
/// replaces the time for the crouch-to-prone stage.
fn lerp_duration(goal: i32, going_down: bool, prone_ms: i32) -> i32 {
    match goal {
        eye_level::LYING_FLAT if prone_ms > 0 => prone_ms,
        eye_level::LYING_FLAT => 400,
        eye_level::DUCK_BUTTON_BIT if going_down => 200,
        eye_level::DUCK_BUTTON_BIT => 400,
        _ => 200,
    }
}

/// How far into a stance change the clock is, from just after the start to
/// done (1); `None` before it starts or when it has not moved yet.
fn progress(avatar: &AvatarState, authority_clock_ms: i32, duration_ms: i32) -> Option<f32> {
    let elapsed = authority_clock_ms.wrapping_sub(avatar.eye_blend_clock) as f32;
    let fraction = elapsed / duration_ms as f32;
    if !(fraction >= 0.0) {
        return None;
    }
    let fraction = if fraction > 1.0_f32 { 1.0 } else { fraction };
    (fraction != 0.0).then_some(fraction)
}

/// The walk-speed factor of the player's stance: the settled factor, or a
/// blend while the eye is still travelling between crouch and prone.
#[must_use]
pub fn posture_pace_ratio(
    avatar: &AvatarState,
    authority_clock_ms: i32,
    last_stand_scale: f32,
    prone_lerp_ms: i32,
) -> f32 {
    let lerping = avatar.eye_blend_clock != 0;

    if lerping && avatar.eye_blend_goal == eye_level::LYING_FLAT {
        let duration = lerp_duration(eye_level::LYING_FLAT, true, prone_lerp_ms);
        if let Some(fraction) = progress(avatar, authority_clock_ms, duration) {
            return fraction * PRONE_SPEED + (1.0 - fraction) * CROUCH_SPEED;
        }
    }

    if lerping && avatar.eye_blend_goal == eye_level::DUCK_BUTTON_BIT && avatar.eye_lower_blend == 0
    {
        let duration = lerp_duration(eye_level::DUCK_BUTTON_BIT, false, prone_lerp_ms);
        if let Some(fraction) = progress(avatar, authority_clock_ms, duration) {
            return fraction * CROUCH_SPEED + (1.0 - fraction) * PRONE_SPEED;
        }
    }

    match posture_footing_kind(avatar) {
        PostureFooting::DownedGrace => last_stand_scale,
        PostureFooting::Crouch => CROUCH_SPEED,
        PostureFooting::Prone => PRONE_SPEED,
        PostureFooting::Stand => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heading_to(target: i32) -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.eye_height_goal = target;
        avatar
    }

    #[test]
    fn each_settled_stance_has_its_own_speed() {
        assert_eq!(
            posture_pace_ratio(&heading_to(eye_level::UPRIGHT), 0, 0.3, 600),
            1.0
        );
        assert_eq!(
            posture_pace_ratio(&heading_to(eye_level::DUCK_BUTTON_BIT), 0, 0.3, 600),
            0.65
        );
        assert_eq!(
            posture_pace_ratio(&heading_to(eye_level::LYING_FLAT), 0, 0.3, 600),
            0.15
        );
        assert_eq!(
            posture_pace_ratio(&heading_to(eye_level::DOWNED_GRACE), 0, 0.3, 600),
            0.3
        );
    }

    #[test]
    fn the_surface_follows_the_eye_target() {
        assert_eq!(
            posture_footing_kind(&heading_to(eye_level::LYING_FLAT)),
            PostureFooting::Prone
        );
        assert_eq!(
            posture_footing_kind(&heading_to(eye_level::DOWNED_GRACE)),
            PostureFooting::DownedGrace
        );
        assert_eq!(posture_footing_kind(&heading_to(0)), PostureFooting::Stand);
    }

    #[test]
    fn going_prone_slows_from_crouch_speed_over_the_prone_time() {
        let mut avatar = heading_to(eye_level::LYING_FLAT);
        avatar.eye_blend_clock = 1000;
        avatar.eye_blend_goal = eye_level::LYING_FLAT;
        // Half of the 600 ms stage in: halfway between 0.65 and 0.15.
        let halfway = posture_pace_ratio(&avatar, 1300, 0.3, 600);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
        // Past the end it holds the prone speed.
        assert_eq!(posture_pace_ratio(&avatar, 5000, 0.3, 600), 0.15);
    }

    #[test]
    fn without_a_prone_time_the_stage_takes_four_hundred() {
        let mut avatar = heading_to(eye_level::LYING_FLAT);
        avatar.eye_blend_clock = 1000;
        avatar.eye_blend_goal = eye_level::LYING_FLAT;
        let halfway = posture_pace_ratio(&avatar, 1200, 0.3, 0);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
    }

    #[test]
    fn rising_from_prone_to_a_crouch_speeds_up() {
        let mut avatar = heading_to(eye_level::DUCK_BUTTON_BIT);
        avatar.eye_blend_clock = 1000;
        avatar.eye_blend_goal = eye_level::DUCK_BUTTON_BIT;
        avatar.eye_lower_blend = 0;
        let halfway = posture_pace_ratio(&avatar, 1200, 0.3, 600);
        assert!((halfway - 0.4).abs() < 1e-6, "{halfway}");
    }

    #[test]
    fn a_change_that_has_not_started_yet_uses_the_settled_speed() {
        let mut avatar = heading_to(eye_level::LYING_FLAT);
        avatar.eye_blend_clock = 1000;
        avatar.eye_blend_goal = eye_level::LYING_FLAT;
        assert_eq!(posture_pace_ratio(&avatar, 1000, 0.3, 600), 0.15);
        assert_eq!(posture_pace_ratio(&avatar, 900, 0.3, 600), 0.15);
    }
}
