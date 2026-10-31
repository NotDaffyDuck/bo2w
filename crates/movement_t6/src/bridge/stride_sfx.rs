//! Walking sounds and leg poses: the ground material under a foot, the moment
//! a step lands, the lower-body animation to play, and the pace of the view bob.

use playerstate_t6::consts::movement_mode;
use playerstate_t6::game::{NO_ENTITY_SLOT, AvatarState, entity_flag_bits, movement_state_bits};

use crate::bridge::stick_scale::StrideScaleArgs;
use crate::bridge::hurt_slow::{
    CLIP_RECOIL_AHEAD, HURT_FLINCH_SPAN_MS,
    HURT_STAGGER_SPAN_MS, damage_window_open,
};
use crate::bridge::event_feed::queue_forecast_happening;
use crate::bridge::feel::{Bo2Feel, PLAYER_MOVE_THRESHHOLD, dive_bob_cycle, sway_phase};
use crate::bridge::posture::{PostureFooting, posture_pace_ratio, posture_footing_kind};

/// Material names, listed in the index order the sound tables use (32 slots).
pub const SURFACE_KIND_LABELS: [&'static str; 32] = [
    "default",
    "bark",
    "brick",
    "carpet",
    "cloth",
    "concrete",
    "dirt",
    "flesh",
    "foliage",
    "glass",
    "grass",
    "gravel",
    "ice",
    "metal",
    "mud",
    "paper",
    "plaster",
    "rock",
    "sand",
    "snow",
    "water",
    "wood",
    "asphalt",
    "ceramic",
    "plastic",
    "rubber",
    "cushion",
    "fruit",
    "paintedmetal",
    "player",
    "tallgrass",
    "riotshield",
];

/// Rungs ring out as metal.
pub const RUNG_MATERIAL: u32 = 13;

/// Surfaces carrying this bit never make a step sound.
const SILENT_SURFACE_FLAG: u32 = 0x2000;

/// Event numbers for a landed step, keyed by pace.
const STEP_EVENT_SPRINT: i32 = 128;
const STEP_EVENT_RUN: i32 = 129;
const STEP_EVENT_WALK: i32 = 130;
const STEP_EVENT_PRONE: i32 = 133;

/// Entity bits showing the player is manning a turret.
const ON_TURRET_BITS: u32 = entity_flag_bits::TURRET_MASK;

/// At or under this ground speed the legs stand still.
const CREEP_SPEED: f32 = 10.0;

/// Lower-body animation numbers chosen here.
const LEGS_IDLE: u8 = 1;
const LEGS_IDLE_CROUCH: u8 = 2;
const LEGS_IDLE_PRONE: u8 = 3;
const LEGS_SPRINT: u8 = 20;
const LEGS_STUMBLE_SPRINT: u8 = 42;
const LEGS_IDLE_LAST_STAND: u8 = 51;

/// Moving-leg animations: row is forward or backward travel, column is
/// `stance * 4 + walking * 2 + stumbling` with stances in `PostureFooting` order.
const LEG_TABLE: [[u8; 16]; 2] = [
    [10, 36, 4, 38, 8, 8, 8, 8, 12, 40, 6, 40, 52, 52, 52, 52],
    [11, 37, 5, 39, 9, 9, 9, 9, 13, 41, 7, 41, 53, 53, 53, 53],
];

/// Material index packed in bits 20 to 24 of a surface flag word.
#[must_use]
pub fn material_kind_number(word: u32) -> usize {
    ((word >> 20) & 0x1f) as usize
}

/// True when the 8-bit bob counter passed a half-turn mark between `old` and
/// `new`, which is the instant a foot comes down.
#[must_use]
pub fn sway_phase_looped(prev: u8, cur: u8) -> bool {
    let shifted = |value: u8| value.wrapping_add(0x40);
    ((shifted(prev) ^ shifted(cur)) as i8) < 0
}

fn flat_speed(avatar: &AvatarState) -> f32 {
    libm::sqrtf(avatar.velocity[0] * avatar.velocity[0] + avatar.velocity[1] * avatar.velocity[1])
}

/// Airborne, with no near-ground plane to excuse it.
fn airborne(avatar: &AvatarState, near_ground: bool) -> bool {
    avatar.standing_on == NO_ENTITY_SLOT
        && avatar.movement_mode != movement_mode::NORMAL_LINKED
        && avatar.movement_mode != movement_mode::LASTSTAND
        && !near_ground
}

/// Base speed after the direction keys: strafing and backing up are slower,
/// and a diagonal blends the two.
fn scaled_by_direction(base: f32, ahead: i8, across: i8, cmd_scales: StrideScaleArgs) -> f32 {
    const STRAFE_SHARE: f32 = 0.75;
    const HALF: f32 = 0.5_f32;

    let back = cmd_scales.retreat_pace_ratio;
    let side_factor = (cmd_scales.sidestep_pace_ratio - 1.0) * STRAFE_SHARE + 1.0;
    match (ahead == 0, across == 0) {
        (true, true) => base,
        (true, false) => base * side_factor,
        (false, true) if ahead < 0 => base * back,
        (false, true) => base,
        (false, false) => {
            let diagonal = base * ((side_factor + 1.0) * HALF);
            if ahead < 0 {
                HALF * (back + 1.0) * diagonal
            } else {
                diagonal
            }
        }
    }
}

/// Apply the held weapon's movement scale, or its aim-down-sights scale when
/// the normal one is off or the player is slow-walking.
fn scaled_by_weapon(avatar: &AvatarState, speed: f32, cmd_scales: StrideScaleArgs) -> f32 {
    if avatar.weapon == 0 {
        return speed;
    }
    let slow = avatar.movement_state_bits & movement_state_bits::WALKING != 0;
    if cmd_scales.gun_stride_ratio > 0.0 && !slow {
        speed * cmd_scales.gun_stride_ratio
    } else if cmd_scales.scoped_carry_pace > 0.0 {
        speed * cmd_scales.scoped_carry_pace
    } else {
        speed
    }
}

/// The top speed to measure the view bob against this frame: the base speed
/// bent by direction keys, walk or sprint, weapon, and stance.
#[must_use]
pub fn get_bob_max_speed(
    avatar: &AvatarState,
    ahead: i8,
    across: i8,
    slow_key: bool,
    run_key: bool,
    clock_millis: i32,
    cmd_scales: StrideScaleArgs,
) -> f32 {
    const WALK_FRACTION: f32 = 0.4;

    let directed = scaled_by_direction(avatar.speed as f32, ahead, across, cmd_scales);
    let paced = match (slow_key, run_key) {
        (true, _) => directed * WALK_FRACTION,
        (false, true) => directed * cmd_scales.dash_pace_ratio,
        (false, false) => directed,
    };
    let armed = scaled_by_weapon(avatar, paced, cmd_scales);
    armed
        * posture_pace_ratio(
            avatar,
            clock_millis,
            cmd_scales.downed_crawl_pace_ratio,
            cmd_scales.prone_lerp_ms,
        )
}

/// Step the view-bob counter on by one frame. Nothing moves while dead, on a
/// turret, airborne, nearly still, linked, or with no direction keys down; a
/// dive supplies its own beat.
pub fn footsteps_bob_cycle(
    avatar: &mut AvatarState,
    msec: i32,
    ahead: i8,
    across: i8,
    near_ground: bool,
    clock_millis: i32,
    cmd_scales: StrideScaleArgs,
    bo2_feel: Bo2Feel,
) {
    if movement_mode::is_dead(avatar.movement_mode) || dive_bob_cycle(avatar, msec, bo2_feel) {
        return;
    }
    if avatar.e_flags & ON_TURRET_BITS != 0 || airborne(avatar, near_ground) {
        return;
    }
    let pace = flat_speed(avatar);
    let too_slow = pace < PLAYER_MOVE_THRESHHOLD;
    if too_slow || avatar.movement_mode == movement_mode::NORMAL_LINKED || (ahead == 0 && across == 0) {
        return;
    }
    sway_phase(avatar, msec, ahead, across, pace, clock_millis, cmd_scales, bo2_feel);
}

/// What the legs do when the player is barely moving: a hit flinch while
/// standing, otherwise the stance's idle pose.
fn resting_legs(avatar: &AvatarState, stance: PostureFooting) -> u8 {
    let strike = damage_window_open(
        avatar.hurt_clock,
        avatar.hurt_length,
        HURT_FLINCH_SPAN_MS,
    );
    if stance == PostureFooting::Stand && strike {
        CLIP_RECOIL_AHEAD.saturating_add(avatar.flinch_yaw_anim as u8)
    } else {
        idle_pose(stance)
    }
}

/// Lower-body animation for the current action; `None` leaves it alone (dead
/// or in the air).
#[must_use]
pub fn footsteps_anim_move_type(
    avatar: &AvatarState,
    ahead: i8,
    across: i8,
    near_ground: bool,
) -> Option<u8> {
    if movement_mode::is_dead(avatar.movement_mode) {
        return None;
    }
    let stance = posture_footing_kind(avatar);
    if avatar.e_flags & ON_TURRET_BITS != 0 {
        return Some(idle_pose(stance));
    }
    if airborne(avatar, near_ground) {
        return None;
    }

    let stumble = damage_window_open(
        avatar.hurt_clock,
        avatar.hurt_length,
        HURT_STAGGER_SPAN_MS,
    );
    if flat_speed(avatar) <= CREEP_SPEED || avatar.movement_mode == movement_mode::NORMAL_LINKED {
        return Some(resting_legs(avatar, stance));
    }
    if ahead == 0 && across == 0 {
        return Some(idle_pose(stance));
    }

    let flags = avatar.movement_state_bits;
    let slow = flags & movement_state_bits::WALKING != 0 || avatar.tilt_sideways != 0.0;
    let reverse = flags & movement_state_bits::BACKWARDS_RUN != 0;
    if stance == PostureFooting::Stand && !reverse && flags & movement_state_bits::SPRINTING != 0 {
        return Some(if stumble {
            LEGS_STUMBLE_SPRINT
        } else {
            LEGS_SPRINT
        });
    }

    let slot = 4 * stance as usize + 2 * usize::from(slow) + usize::from(stumble);
    Some(LEG_TABLE[usize::from(reverse)][slot])
}

/// The motionless pose for a stance.
fn idle_pose(stance: PostureFooting) -> u8 {
    match stance {
        PostureFooting::Prone => LEGS_IDLE_PRONE,
        PostureFooting::Crouch => LEGS_IDLE_CROUCH,
        PostureFooting::DownedGrace => LEGS_IDLE_LAST_STAND,
        PostureFooting::Stand => LEGS_IDLE,
    }
}

/// Only a standing player who is not slow-walking is loud enough for steps.
#[must_use]
pub fn should_make_footsteps(avatar: &AvatarState) -> bool {
    posture_footing_kind(avatar) == PostureFooting::Stand && avatar.movement_state_bits & movement_state_bits::WALKING == 0
}

/// A sprint began and has not been closed off by a later end time.
fn sprint_underway(avatar: &AvatarState) -> bool {
    let times = &avatar.sprint_state;
    times.sprint_begin_ms != 0 && times.sprint_stop_ms < times.sprint_begin_ms
}

/// The event number for this step, or 0 when the surface stays quiet.
fn step_event_for(avatar: &AvatarState, flags: u32) -> i32 {
    if flags & SILENT_SURFACE_FLAG != 0 || material_kind_number(flags) == 0 {
        return 0;
    }
    if avatar.movement_state_bits & movement_state_bits::LYING_FLAT != 0 {
        return STEP_EVENT_PRONE;
    }
    let hushed = avatar.movement_state_bits & (movement_state_bits::WALKING | movement_state_bits::DUCK_BUTTON_BIT) != 0 || avatar.tilt_sideways != 0.0;
    match (hushed, sprint_underway(avatar)) {
        (true, _) => STEP_EVENT_WALK,
        (false, true) => STEP_EVENT_SPRINT,
        (false, false) => STEP_EVENT_RUN,
    }
}

/// Queue a step event when the bob counter crossed a landing mark from `old`
/// to `new`, the player stands on something, and `make_noise` allows it.
/// Says whether an event was queued.
pub fn footstep_event(
    avatar: &mut AvatarState,
    old: u8,
    new: u8,
    surface_word: u32,
    make_noise: bool,
) -> bool {
    let grounded = avatar.standing_on != NO_ENTITY_SLOT;
    if !(sway_phase_looped(old, new) && grounded && make_noise) {
        return false;
    }
    match step_event_for(avatar, surface_word) {
        0 => false,
        event => {
            queue_forecast_happening(avatar, event, material_kind_number(surface_word) as i32);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::eye_level;

    const SCALES: StrideScaleArgs = StrideScaleArgs {
        retreat_pace_ratio: 0.7,
        sidestep_pace_ratio: 0.8,
        dash_pace_ratio: 1.5,
        downed_crawl_pace_ratio: 0.3,
        gun_stride_ratio: 0.0,
        scoped_carry_pace: 0.0,
        shellshock_movement_scale: 1.0,
        prone_lerp_ms: 600,
    };

    /// Set the stance by eye height.
    fn at_height(avatar: &mut AvatarState, height: i32) {
        avatar.eye_height_goal = height;
    }

    /// On the ground at a full run.
    fn running() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 5;
        avatar.velocity = [190.0, 0.0, 0.0];
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        at_height(&mut avatar, eye_level::UPRIGHT);
        avatar
    }

    const STONE: u32 = 5 << 20;

    #[test]
    fn the_surface_type_is_in_bits_twenty_to_twenty_four() {
        assert_eq!(material_kind_number(13 << 20), 13);
        assert_eq!(material_kind_number((31 << 20) | 0xfffff), 31);
        assert_eq!(SURFACE_KIND_LABELS[RUNG_MATERIAL as usize], "metal");
    }

    #[test]
    fn the_bob_counter_wraps_at_the_half_way_mark_and_at_the_top() {
        assert!(sway_phase_looped(0x3f, 0x40));
        assert!(sway_phase_looped(0xbf, 0xc0));
        assert!(!sway_phase_looped(0x40, 0x41));
        assert!(!sway_phase_looped(0x00, 0x3f));
    }

    #[test]
    fn full_run_forward_is_the_players_speed() {
        assert_eq!(
            get_bob_max_speed(&running(), 127, 0, false, false, 0, SCALES),
            190.0
        );
    }

    #[test]
    fn backpedalling_and_strafing_slow_the_bob_target() {
        let avatar = running();
        let back = get_bob_max_speed(&avatar, -127, 0, false, false, 0, SCALES);
        assert!((back - 190.0 * 0.7).abs() < 1e-3, "{back}");
        let strafe = get_bob_max_speed(&avatar, 0, 127, false, false, 0, SCALES);
        assert!(
            (strafe - 190.0 * ((0.8 - 1.0) * 0.75 + 1.0)).abs() < 1e-3,
            "{strafe}"
        );
    }

    #[test]
    fn walking_beats_sprinting_in_the_bob_target() {
        let avatar = running();
        assert!((get_bob_max_speed(&avatar, 127, 0, true, true, 0, SCALES) - 190.0 * 0.4).abs() < 1e-3);
        assert!(
            (get_bob_max_speed(&avatar, 127, 0, false, true, 0, SCALES) - 190.0 * 1.5).abs() < 1e-3
        );
    }

    #[test]
    fn a_held_weapon_scales_the_bob_target_by_its_move_scale() {
        let mut avatar = running();
        avatar.weapon = 7;
        let scales = StrideScaleArgs {
            gun_stride_ratio: 0.9,
            ..SCALES
        };
        assert!((get_bob_max_speed(&avatar, 127, 0, false, false, 0, scales) - 171.0).abs() < 1e-3);
    }

    #[test]
    fn a_standing_run_makes_the_legs_run_and_a_sprint_makes_them_sprint() {
        let mut avatar = running();
        assert_eq!(footsteps_anim_move_type(&avatar, 127, 0, false), Some(10));
        avatar.movement_state_bits |= movement_state_bits::SPRINTING;
        assert_eq!(
            footsteps_anim_move_type(&avatar, 127, 0, false),
            Some(LEGS_SPRINT)
        );
    }

    #[test]
    fn crouched_backward_and_walking_pick_their_own_animations() {
        let mut avatar = running();
        at_height(&mut avatar, eye_level::DUCK_BUTTON_BIT);
        assert_eq!(footsteps_anim_move_type(&avatar, 127, 0, false), Some(12));
        avatar.movement_state_bits |= movement_state_bits::BACKWARDS_RUN | movement_state_bits::WALKING;
        assert_eq!(footsteps_anim_move_type(&avatar, -127, 0, false), Some(7));
        at_height(&mut avatar, eye_level::UPRIGHT);
        assert_eq!(footsteps_anim_move_type(&avatar, -127, 0, false), Some(5));
    }

    #[test]
    fn standing_still_idles_in_the_stance() {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 5;
        for (height, anim) in [
            (eye_level::UPRIGHT, LEGS_IDLE),
            (eye_level::DUCK_BUTTON_BIT, LEGS_IDLE_CROUCH),
            (eye_level::LYING_FLAT, LEGS_IDLE_PRONE),
            (eye_level::DOWNED_GRACE, LEGS_IDLE_LAST_STAND),
        ] {
            at_height(&mut avatar, height);
            assert_eq!(
                footsteps_anim_move_type(&avatar, 0, 0, false),
                Some(anim),
                "height {height}"
            );
        }
    }

    #[test]
    fn the_air_and_death_leave_the_legs_alone() {
        let mut avatar = running();
        avatar.standing_on = NO_ENTITY_SLOT;
        assert_eq!(footsteps_anim_move_type(&avatar, 127, 0, false), None);
        assert_eq!(footsteps_anim_move_type(&avatar, 127, 0, true), Some(10));
        avatar.movement_mode = movement_mode::DEAD;
        assert_eq!(footsteps_anim_move_type(&avatar, 127, 0, true), None);
    }

    #[test]
    fn a_standing_hit_flinches_while_the_window_is_open() {
        let mut avatar = AvatarState::ZERO;
        avatar.standing_on = 5;
        at_height(&mut avatar, eye_level::UPRIGHT);
        avatar.hurt_length = 1000;
        avatar.hurt_clock = 900;
        avatar.flinch_yaw_anim = 2;
        assert_eq!(
            footsteps_anim_move_type(&avatar, 0, 0, false),
            Some(CLIP_RECOIL_AHEAD + 2)
        );
    }

    #[test]
    fn only_an_upright_unhurried_player_makes_footsteps() {
        let mut avatar = running();
        assert!(should_make_footsteps(&avatar));
        avatar.movement_state_bits |= movement_state_bits::WALKING;
        assert!(!should_make_footsteps(&avatar));
        avatar.movement_state_bits = 0;
        at_height(&mut avatar, eye_level::DUCK_BUTTON_BIT);
        assert!(!should_make_footsteps(&avatar));
    }

    #[test]
    fn a_landing_foot_raises_the_event_for_how_the_player_moves() {
        let mut avatar = running();
        assert!(footstep_event(&mut avatar, 0x3f, 0x40, STONE, true));
        assert_eq!(avatar.predictable_events[0], STEP_EVENT_RUN);
        assert_eq!(avatar.predictable_event_parms[0], 5);

        avatar.sprint_state.sprint_begin_ms = 100;
        assert!(footstep_event(&mut avatar, 0x3f, 0x40, STONE, true));
        assert_eq!(avatar.predictable_events[1], STEP_EVENT_SPRINT);

        avatar.sprint_state.sprint_begin_ms = 0;
        avatar.movement_state_bits |= movement_state_bits::DUCK_BUTTON_BIT;
        assert!(footstep_event(&mut avatar, 0x3f, 0x40, STONE, true));
        assert_eq!(avatar.predictable_events[2], STEP_EVENT_WALK);

        avatar.movement_state_bits = movement_state_bits::LYING_FLAT;
        assert!(footstep_event(&mut avatar, 0x3f, 0x40, STONE, true));
        assert_eq!(avatar.predictable_events[3], STEP_EVENT_PRONE);
    }

    #[test]
    fn no_event_without_a_landing_ground_noise_or_a_loud_surface() {
        let mut avatar = running();
        assert!(!footstep_event(&mut avatar, 0x40, 0x41, STONE, true), "no wrap");
        assert!(
            !footstep_event(&mut avatar, 0x3f, 0x40, STONE, false),
            "noise off"
        );
        assert!(
            !footstep_event(&mut avatar, 0x3f, 0x40, 0, true),
            "default surface is silent"
        );
        assert!(
            !footstep_event(&mut avatar, 0x3f, 0x40, STONE | SILENT_SURFACE_FLAG, true),
            "no-steps surface"
        );
        avatar.standing_on = NO_ENTITY_SLOT;
        assert!(
            !footstep_event(&mut avatar, 0x3f, 0x40, STONE, true),
            "in the air"
        );
    }

    #[test]
    fn the_bob_counter_moves_only_while_walking_on_the_ground_with_keys_held() {
        let feel = Bo2Feel::BO2_MP;
        let mut avatar = running();
        footsteps_bob_cycle(&mut avatar, 50, 127, 0, false, 0, SCALES, feel);
        assert_ne!(avatar.sway_phase, 0, "a run advances the bob");

        let mut idle = running();
        footsteps_bob_cycle(&mut idle, 50, 0, 0, false, 0, SCALES, feel);
        assert_eq!(idle.sway_phase, 0, "no keys");

        let mut air = running();
        air.standing_on = NO_ENTITY_SLOT;
        footsteps_bob_cycle(&mut air, 50, 127, 0, false, 0, SCALES, feel);
        assert_eq!(air.sway_phase, 0, "airborne");
    }
}
