//! Footsteps: which surface the foot is on, when a step lands, which move
//! animation the legs play, and how fast the head bob should run.

use playerstate_t6::consts::pm_type;
use playerstate_t6::game::{ENTITYNUM_NONE, PlayerState, pm_flags};

use crate::bridge::cmdscale::CmdScaleWalkContext;
use crate::bridge::dmgtimer::{
    ANIM_MT_FLINCH_FORWARD, PLAYER_DMGTIMER_FLINCH_TIME_MS, PLAYER_DMGTIMER_STUMBLE_TIME_MS,
    damage_window_open,
};
use crate::bridge::events::add_predictable_event;
use crate::bridge::feel::{self, Bo2Feel};
use crate::bridge::stance::{StanceSurface, stance_speed_scale, stance_surface_type};

/// The surface types, in the order the sound tables number them.
pub const SURFACE_TYPE_NAMES: [&str; 31] = [
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
    "riotshield",
    "slush",
];

/// A ladder rung sounds like metal.
pub const LADDER_SURFACE_TYPE: u32 = 13;

/// The surface flag that silences footsteps.
const SURF_NOSTEPS: u32 = 0x2000;

/// The footstep events, by how the player is moving.
const EV_FOOTSTEP_SPRINT: i32 = 128;
const EV_FOOTSTEP_RUN: i32 = 129;
const EV_FOOTSTEP_WALK: i32 = 130;
const EV_FOOTSTEP_PRONE: i32 = 133;

/// Entity flags that mean the player is on a turret.
const EFLAGS_TURRET: u32 = 0xc00;

/// Below this ground speed the legs idle.
const IDLE_SPEED: f32 = 10.0;

/// Leg animations the move type picks between.
const ANIM_MT_IDLE: u8 = 1;
const ANIM_MT_IDLECR: u8 = 2;
const ANIM_MT_IDLEPRONE: u8 = 3;
const ANIM_MT_SPRINT: u8 = 20;
const ANIM_MT_STUMBLE_SPRINT_FORWARD: u8 = 42;
const ANIM_MT_IDLELASTSTAND: u8 = 51;

/// The move animation, by `[running backward][stance * 4 + walking * 2 +
/// stumbling]`, with the stances in `StanceSurface` order.
const MOVE_ANIM: [[u8; 16]; 2] = [
    [10, 36, 4, 38, 8, 8, 8, 8, 12, 40, 6, 40, 52, 52, 52, 52],
    [11, 37, 5, 39, 9, 9, 9, 9, 13, 41, 7, 41, 53, 53, 53, 53],
];

/// The surface type a surface flag word names (bits 20 to 24).
#[must_use]
pub fn surface_type_index(surface_flags: u32) -> usize {
    ((surface_flags >> 20) & 0x1f) as usize
}

/// Whether the 8-bit bob counter crossed the half-way mark going from `old`
/// to `new`: that is when a foot lands.
#[must_use]
pub fn bob_cycle_wrapped(old: u8, new: u8) -> bool {
    let half = |cycle: u8| cycle.wrapping_add(0x40);
    ((half(old) ^ half(new)) as i8) < 0
}

fn ground_speed(ps: &PlayerState) -> f32 {
    libm::sqrtf(ps.velocity[0] * ps.velocity[0] + ps.velocity[1] * ps.velocity[1])
}

/// In the air, with nothing underfoot close enough to count as ground.
fn off_the_ground(ps: &PlayerState, almost_ground_plane: bool) -> bool {
    ps.ground_entity_num == ENTITYNUM_NONE
        && ps.pm_type != pm_type::NORMAL_LINKED
        && ps.pm_type != pm_type::LASTSTAND
        && !almost_ground_plane
}

/// The fastest the player can go this frame, for scaling the head bob: the
/// base speed shaped by the keys held, lean, sprint, the weapon and the stance.
#[must_use]
pub fn get_bob_max_speed(
    ps: &PlayerState,
    forwardmove: i8,
    rightmove: i8,
    walking: bool,
    sprinting: bool,
    server_time: i32,
    scales: CmdScaleWalkContext,
) -> f32 {
    const STRAFE_SHARE: f32 = 0.75;
    const HALF: f32 = 0.5;
    const WALK_SCALE: f32 = 0.4;

    let strafe = scales.player_strafe_speed_scale;
    let back = scales.player_back_speed_scale;
    let strafe_factor = (strafe - 1.0) * STRAFE_SHARE + 1.0;

    let mut max_speed = ps.speed as f32;
    if forwardmove == 0 {
        if rightmove != 0 {
            max_speed *= strafe_factor;
        }
    } else if rightmove == 0 {
        if forwardmove < 0 {
            max_speed *= back;
        }
    } else {
        max_speed *= (strafe_factor + 1.0) * HALF;
        if forwardmove < 0 {
            max_speed = HALF * (back + 1.0) * max_speed;
        }
    }

    if walking {
        max_speed *= WALK_SCALE;
    } else if sprinting {
        max_speed *= scales.player_sprint_speed_scale;
    }

    if ps.weapon != 0 {
        let slow_walk = ps.pm_flags & pm_flags::WALKING != 0;
        if scales.weapon_move_speed_scale > 0.0 && !slow_walk {
            max_speed *= scales.weapon_move_speed_scale;
        } else if scales.weapon_ads_move_speed_scale > 0.0 {
            max_speed *= scales.weapon_ads_move_speed_scale;
        }
    }

    max_speed
        * stance_speed_scale(
            ps,
            server_time,
            scales.player_last_stand_crawl_speed_scale,
            scales.prone_lerp_ms,
        )
}

/// Advance the head-bob counter one frame. It stands still when dead, on a
/// turret, in the air, when barely moving, linked, or with no keys held; a dive
/// runs its own rhythm.
pub fn footsteps_bob_cycle(
    ps: &mut PlayerState,
    msec: i32,
    forwardmove: i8,
    rightmove: i8,
    almost_ground_plane: bool,
    server_time: i32,
    scales: CmdScaleWalkContext,
    bo2_feel: Bo2Feel,
) {
    if pm_type::is_dead(ps.pm_type) {
        return;
    }
    if feel::dive_bob_cycle(ps, msec, bo2_feel) {
        return;
    }
    if ps.e_flags & EFLAGS_TURRET != 0 || off_the_ground(ps, almost_ground_plane) {
        return;
    }
    let speed = ground_speed(ps);
    if speed < feel::PLAYER_MOVE_THRESHHOLD || ps.pm_type == pm_type::NORMAL_LINKED {
        return;
    }
    if forwardmove == 0 && rightmove == 0 {
        return;
    }
    feel::bob_cycle(
        ps,
        msec,
        forwardmove,
        rightmove,
        speed,
        server_time,
        scales,
        bo2_feel,
    );
}

/// The leg animation for what the player is doing; `None` when nothing
/// should change it (dead, or in the air).
#[must_use]
pub fn footsteps_anim_move_type(
    ps: &PlayerState,
    forwardmove: i8,
    rightmove: i8,
    almost_ground_plane: bool,
) -> Option<u8> {
    if pm_type::is_dead(ps.pm_type) {
        return None;
    }
    let stance = stance_surface_type(ps);
    if ps.e_flags & EFLAGS_TURRET != 0 {
        return Some(idle_anim(stance));
    }
    if off_the_ground(ps, almost_ground_plane) {
        return None;
    }

    let stumbling = damage_window_open(
        ps.damage_timer,
        ps.damage_duration,
        PLAYER_DMGTIMER_STUMBLE_TIME_MS,
    );
    if ground_speed(ps) <= IDLE_SPEED || ps.pm_type == pm_type::NORMAL_LINKED {
        let flinching = damage_window_open(
            ps.damage_timer,
            ps.damage_duration,
            PLAYER_DMGTIMER_FLINCH_TIME_MS,
        );
        return Some(if stance == StanceSurface::Stand && flinching {
            ANIM_MT_FLINCH_FORWARD.saturating_add(ps.flinch_yaw_anim as u8)
        } else {
            idle_anim(stance)
        });
    }
    if forwardmove == 0 && rightmove == 0 {
        return Some(idle_anim(stance));
    }

    let walking = ps.pm_flags & pm_flags::WALKING != 0 || ps.leanf != 0.0;
    let sprinting = ps.pm_flags & pm_flags::SPRINTING != 0;
    let backward = ps.pm_flags & pm_flags::BACKWARDS_RUN != 0;
    if stance == StanceSurface::Stand && !backward && sprinting {
        return Some(if stumbling {
            ANIM_MT_STUMBLE_SPRINT_FORWARD
        } else {
            ANIM_MT_SPRINT
        });
    }

    let column = 4 * stance as usize + 2 * usize::from(walking) + usize::from(stumbling);
    Some(MOVE_ANIM[usize::from(backward)][column])
}

/// The standing-still animation of a stance.
fn idle_anim(stance: StanceSurface) -> u8 {
    match stance {
        StanceSurface::Prone => ANIM_MT_IDLEPRONE,
        StanceSurface::Crouch => ANIM_MT_IDLECR,
        StanceSurface::LastStand => ANIM_MT_IDLELASTSTAND,
        StanceSurface::Stand => ANIM_MT_IDLE,
    }
}

/// Only an upright, unhurried player makes footstep noise.
#[must_use]
pub fn should_make_footsteps(ps: &PlayerState) -> bool {
    stance_surface_type(ps) == StanceSurface::Stand && ps.pm_flags & pm_flags::WALKING == 0
}

/// The sprint timestamps say a sprint is under way: one has started and not
/// yet ended.
fn mid_sprint(ps: &PlayerState) -> bool {
    let state = &ps.sprint_state;
    state.last_sprint_start != 0 && state.last_sprint_end < state.last_sprint_start
}

/// Which footstep event this step raises; 0 for a silent surface.
fn footstep_event_type(ps: &PlayerState, surface_flags: u32) -> i32 {
    if surface_flags & SURF_NOSTEPS != 0 || surface_type_index(surface_flags) == 0 {
        return 0;
    }
    if ps.pm_flags & pm_flags::PRONE != 0 {
        return EV_FOOTSTEP_PRONE;
    }
    let quiet = ps.pm_flags & (pm_flags::WALKING | pm_flags::CROUCH) != 0 || ps.leanf != 0.0;
    match (quiet, mid_sprint(ps)) {
        (true, _) => EV_FOOTSTEP_WALK,
        (false, true) => EV_FOOTSTEP_SPRINT,
        (false, false) => EV_FOOTSTEP_RUN,
    }
}

/// Raise a footstep event if the bob counter went from `old` to `new` across
/// a landing, the player is on the ground, and `make_noise` is set. Returns
/// whether it did.
pub fn footstep_event(
    ps: &mut PlayerState,
    old: u8,
    new: u8,
    surface_flags: u32,
    make_noise: bool,
) -> bool {
    if !bob_cycle_wrapped(old, new) || ps.ground_entity_num == ENTITYNUM_NONE || !make_noise {
        return false;
    }
    let event = footstep_event_type(ps, surface_flags);
    if event == 0 {
        return false;
    }
    add_predictable_event(ps, event, surface_type_index(surface_flags) as i32);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::stance::view_height;

    const SCALES: CmdScaleWalkContext = CmdScaleWalkContext {
        player_back_speed_scale: 0.7,
        player_strafe_speed_scale: 0.8,
        player_sprint_speed_scale: 1.5,
        player_last_stand_crawl_speed_scale: 0.3,
        weapon_move_speed_scale: 0.0,
        weapon_ads_move_speed_scale: 0.0,
        shellshock_movement_scale: 1.0,
        prone_lerp_ms: 600,
    };

    /// On the ground at a full run.
    fn running() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 5;
        ps.velocity = [190.0, 0.0, 0.0];
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.view_height_target = view_height::STAND;
        ps
    }

    const STONE: u32 = 5 << 20;

    #[test]
    fn the_surface_type_is_in_bits_twenty_to_twenty_four() {
        assert_eq!(surface_type_index(13 << 20), 13);
        assert_eq!(surface_type_index((31 << 20) | 0xfffff), 31);
        assert_eq!(SURFACE_TYPE_NAMES[LADDER_SURFACE_TYPE as usize], "metal");
    }

    #[test]
    fn the_bob_counter_wraps_at_the_half_way_mark_and_at_the_top() {
        assert!(bob_cycle_wrapped(0x3f, 0x40));
        assert!(bob_cycle_wrapped(0xbf, 0xc0));
        assert!(!bob_cycle_wrapped(0x40, 0x41));
        assert!(!bob_cycle_wrapped(0x00, 0x3f));
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
        let ps = running();
        let back = get_bob_max_speed(&ps, -127, 0, false, false, 0, SCALES);
        assert!((back - 190.0 * 0.7).abs() < 1e-3, "{back}");
        let strafe = get_bob_max_speed(&ps, 0, 127, false, false, 0, SCALES);
        assert!(
            (strafe - 190.0 * ((0.8 - 1.0) * 0.75 + 1.0)).abs() < 1e-3,
            "{strafe}"
        );
    }

    #[test]
    fn walking_beats_sprinting_in_the_bob_target() {
        let ps = running();
        assert!((get_bob_max_speed(&ps, 127, 0, true, true, 0, SCALES) - 190.0 * 0.4).abs() < 1e-3);
        assert!(
            (get_bob_max_speed(&ps, 127, 0, false, true, 0, SCALES) - 190.0 * 1.5).abs() < 1e-3
        );
    }

    #[test]
    fn a_held_weapon_scales_the_bob_target_by_its_move_scale() {
        let mut ps = running();
        ps.weapon = 7;
        let scales = CmdScaleWalkContext {
            weapon_move_speed_scale: 0.9,
            ..SCALES
        };
        assert!((get_bob_max_speed(&ps, 127, 0, false, false, 0, scales) - 171.0).abs() < 1e-3);
    }

    #[test]
    fn a_standing_run_makes_the_legs_run_and_a_sprint_makes_them_sprint() {
        let mut ps = running();
        assert_eq!(footsteps_anim_move_type(&ps, 127, 0, false), Some(10));
        ps.pm_flags |= pm_flags::SPRINTING;
        assert_eq!(
            footsteps_anim_move_type(&ps, 127, 0, false),
            Some(ANIM_MT_SPRINT)
        );
    }

    #[test]
    fn crouched_backward_and_walking_pick_their_own_animations() {
        let mut ps = running();
        ps.view_height_target = view_height::CROUCH;
        assert_eq!(footsteps_anim_move_type(&ps, 127, 0, false), Some(12));
        ps.pm_flags |= pm_flags::BACKWARDS_RUN | pm_flags::WALKING;
        assert_eq!(footsteps_anim_move_type(&ps, -127, 0, false), Some(7));
        ps.view_height_target = view_height::STAND;
        assert_eq!(footsteps_anim_move_type(&ps, -127, 0, false), Some(5));
    }

    #[test]
    fn standing_still_idles_in_the_stance() {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 5;
        for (height, anim) in [
            (view_height::STAND, ANIM_MT_IDLE),
            (view_height::CROUCH, ANIM_MT_IDLECR),
            (view_height::PRONE, ANIM_MT_IDLEPRONE),
            (view_height::LAST_STAND, ANIM_MT_IDLELASTSTAND),
        ] {
            ps.view_height_target = height;
            assert_eq!(
                footsteps_anim_move_type(&ps, 0, 0, false),
                Some(anim),
                "height {height}"
            );
        }
    }

    #[test]
    fn the_air_and_death_leave_the_legs_alone() {
        let mut ps = running();
        ps.ground_entity_num = ENTITYNUM_NONE;
        assert_eq!(footsteps_anim_move_type(&ps, 127, 0, false), None);
        assert_eq!(footsteps_anim_move_type(&ps, 127, 0, true), Some(10));
        ps.pm_type = pm_type::DEAD;
        assert_eq!(footsteps_anim_move_type(&ps, 127, 0, true), None);
    }

    #[test]
    fn a_standing_hit_flinches_while_the_window_is_open() {
        let mut ps = PlayerState::ZERO;
        ps.ground_entity_num = 5;
        ps.view_height_target = view_height::STAND;
        ps.damage_duration = 1000;
        ps.damage_timer = 900;
        ps.flinch_yaw_anim = 2;
        assert_eq!(
            footsteps_anim_move_type(&ps, 0, 0, false),
            Some(ANIM_MT_FLINCH_FORWARD + 2)
        );
    }

    #[test]
    fn only_an_upright_unhurried_player_makes_footsteps() {
        let mut ps = running();
        assert!(should_make_footsteps(&ps));
        ps.pm_flags |= pm_flags::WALKING;
        assert!(!should_make_footsteps(&ps));
        ps.pm_flags = 0;
        ps.view_height_target = view_height::CROUCH;
        assert!(!should_make_footsteps(&ps));
    }

    #[test]
    fn a_landing_foot_raises_the_event_for_how_the_player_moves() {
        let mut ps = running();
        assert!(footstep_event(&mut ps, 0x3f, 0x40, STONE, true));
        assert_eq!(ps.predictable_events[0], EV_FOOTSTEP_RUN);
        assert_eq!(ps.predictable_event_parms[0], 5);

        ps.sprint_state.last_sprint_start = 100;
        assert!(footstep_event(&mut ps, 0x3f, 0x40, STONE, true));
        assert_eq!(ps.predictable_events[1], EV_FOOTSTEP_SPRINT);

        ps.sprint_state.last_sprint_start = 0;
        ps.pm_flags |= pm_flags::CROUCH;
        assert!(footstep_event(&mut ps, 0x3f, 0x40, STONE, true));
        assert_eq!(ps.predictable_events[2], EV_FOOTSTEP_WALK);

        ps.pm_flags = pm_flags::PRONE;
        assert!(footstep_event(&mut ps, 0x3f, 0x40, STONE, true));
        assert_eq!(ps.predictable_events[3], EV_FOOTSTEP_PRONE);
    }

    #[test]
    fn no_event_without_a_landing_ground_noise_or_a_loud_surface() {
        let mut ps = running();
        assert!(!footstep_event(&mut ps, 0x40, 0x41, STONE, true), "no wrap");
        assert!(
            !footstep_event(&mut ps, 0x3f, 0x40, STONE, false),
            "noise off"
        );
        assert!(
            !footstep_event(&mut ps, 0x3f, 0x40, 0, true),
            "default surface is silent"
        );
        assert!(
            !footstep_event(&mut ps, 0x3f, 0x40, STONE | SURF_NOSTEPS, true),
            "no-steps surface"
        );
        ps.ground_entity_num = ENTITYNUM_NONE;
        assert!(
            !footstep_event(&mut ps, 0x3f, 0x40, STONE, true),
            "in the air"
        );
    }

    #[test]
    fn the_bob_counter_moves_only_while_walking_on_the_ground_with_keys_held() {
        let feel = Bo2Feel::BO2_MP;
        let mut ps = running();
        footsteps_bob_cycle(&mut ps, 50, 127, 0, false, 0, SCALES, feel);
        assert_ne!(ps.bob_cycle, 0, "a run advances the bob");

        let mut idle = running();
        footsteps_bob_cycle(&mut idle, 50, 0, 0, false, 0, SCALES, feel);
        assert_eq!(idle.bob_cycle, 0, "no keys");

        let mut air = running();
        air.ground_entity_num = ENTITYNUM_NONE;
        footsteps_bob_cycle(&mut air, 50, 127, 0, false, 0, SCALES, feel);
        assert_eq!(air.bob_cycle, 0, "airborne");
    }
}
