//! Black Ops II's PM_WalkMove: the landing slowdown, a jump, friction, the
//! keys' push along the ground, the sliding and slick ground's push, then
//! the step and slide. It steps every frame, standing still too, so a
//! player at rest stays down on the floor.

use crate::bridge::{
    LeapGateArgs, LeapGateVerdict, HullExtents, TickLocals, PostureFooting,
    FootedStepArgs, accelerate, cmd_scale_walk, damage_scale_walk, leap,
    posture_footing_kind, step_slide_move, walk_move_drop_damage_timer,
};
use playerstate_t6::game::{AvatarState, InputCommand, movement_state_bits};

use crate::air::air_move;
use crate::feel::{JUMP_LAND_RAISE, SLICK_ACCEL, KNOCKBACK_ACCEL};
use crate::gait_cycle::WATER_SURFACE_TYPE;
use crate::frame::BodyAnims;
use crate::ground_drag::{SURF_SLIDE, friction};
use crate::gjk::Bo2Collision;
use crate::movement_dir::set_movement_dir;
use crate::project::{clip_to_ground_plane, wish_dir};
use crate::slide_adjust::slide_adjust;

/// A trace's surface type bits (surfaceFlags >> 20).
const SURFACE_TYPE_BITS: u32 = 0x3f << 20;

/// The forced slick flag (movement_state_bits).
const PMF_SLICK: u32 = 0x4000_0000;

/// The knockback timer (movement_state_bits 0x100): pushed, the ground's grip is off.
const PMF_TIME_KNOCKBACK: u32 = 0x100;

/// The hard-landing stun (movement_state_bits 0x80) cuts the push to a quarter.
const PMF_TIME_HARDLANDING: u32 = 0x80;
const HARD_LANDING_ACCEL_SCALE: f32 = 0.25;

const BELLY_SPEEDUP: f32 = 19.0;
const SQUAT_SPEEDUP: f32 = 12.0;
const UPRIGHT_SPEEDUP: f32 = 9.0;

/// How long a landing's slowdown lasts (ms): a high landing, a low one, and
/// past the low one the jump is over.
const HIGH_LANDING_MS: i32 = 0x4b0;
const LOW_LANDING_MS: i32 = 0x708;

/// The speed kept on landing: from high up, and otherwise.
const HIGH_LANDING_SCALE: f32 = 0.5;
const LOW_LANDING_SCALE: f32 = 0.65;

pub(crate) fn walk_move<C: Bo2Collision>(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    ucmd: &mut InputCommand,
    frame_ctx: FootedStepArgs,
    bounds: HullExtents,
    solids: &C,
    body: &mut BodyAnims,
) -> f32 {
    let feel = frame_ctx.feel;
    if avatar.movement_state_bits & movement_state_bits::JUMPING != 0 {
        landing_slowdown(avatar, feel.jump_slowdown);
    }
    // Sprinting (not with sprint in any direction), the sideways keys count
    // for less.
    if !feel.omni && avatar.movement_state_bits & movement_state_bits::SPRINTING != 0 {
        ucmd.thrust_lateral = ((ucmd.thrust_lateral as f32) * feel.sprint_strafe_speed_scale) as i32 as i8;
    }

    if jump_check(avatar, tick_vars, ucmd, &frame_ctx) {
        body.jump = true;
        return air_move(avatar, tick_vars, ucmd, frame_ctx.air, bounds, solids, feel.zombies, frame_ctx.lunge);
    }

    friction(avatar, tick_vars, feel.jump_slowdown, frame_ctx.lunge);

    let scale = crate::feel::omni_cmd_scale(avatar, frame_ctx.cmd_scale, feel);
    let scale = crate::feel::stamin_up_cmd_scale(avatar, scale, feel);
    let command_scale = cmd_scale_walk(avatar, ucmd, scale) * damage_scale_walk(avatar.hurt_clock);
    walk_move_drop_damage_timer(avatar, tick_vars.step_millis_elapsed);

    let (mut intended_heading, intended_pace) = wish_dir(tick_vars.fwd_axis, tick_vars.right_axis, ucmd.thrust_ahead, ucmd.thrust_lateral);
    clip_to_ground_plane(&mut intended_heading, &tick_vars.floor_probe);
    accelerate(
        avatar,
        tick_vars,
        &intended_heading,
        intended_pace * frame_ctx.gun_carry_pace * command_scale,
        accel_scale(avatar, tick_vars),
    );

    slide_adjust(avatar, tick_vars);

    if tick_vars.floor_probe[4] & SURF_SLIDE != 0 || avatar.movement_state_bits & (PMF_SLICK | PMF_TIME_KNOCKBACK) != 0 {
        avatar.velocity[2] -= avatar.gravity as f32 * tick_vars.step_millis_elapsed;
    }

    clip_to_ground_plane(&mut avatar.velocity, &tick_vars.floor_probe);
    let HullExtents { lo_corner: hull_low, hi_corner: hull_high, collide_filter: filter } = bounds;
    let step = if solids.gjk_on() {
        crate::gjk::step::step_slide_move(avatar, tick_vars, solids, hull_low, hull_high, filter, false, feel.zombies)
    } else {
        step_slide_move(avatar, tick_vars, solids, hull_low, hull_high, filter, None, feel.zombies)
    };
    set_movement_dir(avatar, tick_vars, ucmd);
    step
}

/// Jump_Check (walking and on a ladder): true when the player jumped. A jump
/// taken soon after landing one goes lower.
pub(crate) fn jump_check(
    avatar: &mut AvatarState,
    tick_vars: &mut TickLocals,
    ucmd: &mut InputCommand,
    frame_ctx: &FootedStepArgs,
) -> bool {
    let gate = LeapGateArgs {
        since_leap_ms: ucmd.authority_clock_ms.wrapping_sub(avatar.hop_start_ms),
        last_tick_buttons: frame_ctx.last_tick_buttons,
        posture_footing_kind: posture_footing_kind(avatar) as u8,
    };
    let mut launch = frame_ctx.jump;
    launch.squat_leap_ratio = leap::bo2_land_factor(avatar, frame_ctx.feel.jump_slowdown);
    if on_a_player(avatar) {
        // Another player is not ground to jump off. A held jump is still
        // cleared, so it must be pressed again once off them.
        leap::check_gate(avatar, ucmd, gate);
        return false;
    }
    let floor = tick_vars.floor_probe[4];
    tick_vars.floor_probe[4] = jump_surface_flags(avatar, floor);
    let launched = matches!(
        leap::check(avatar, tick_vars, ucmd, gate, launch),
        LeapGateVerdict::Launched { .. }
    );
    tick_vars.floor_probe[4] = floor;
    launched
}

/// Jump_AddSurfaceEvent names the ground as PM_GroundSurfaceType does, so a
/// jump out of any water splashes; the shared jump reads the floor's type.
fn jump_surface_flags(avatar: &AvatarState, floor: u32) -> u32 {
    if avatar.waterlevel == 0 {
        return floor;
    }
    (floor & !SURFACE_TYPE_BITS) | ((WATER_SURFACE_TYPE as u32) << 20)
}

/// Standing on another player (BO2's G_IsEntWalkable says no to them).
fn on_a_player(avatar: &AvatarState) -> bool {
    crate::ground::is_player(avatar.standing_on)
}

/// Jump_ApplySlowdown: the first landed frame keeps half the speed when the
/// player came down at least 18 units above the take-off, 0.65 otherwise,
/// and starts the landing's timer; once that runs past the low landing's
/// time the jump is over.
fn landing_slowdown(avatar: &mut AvatarState, jump_slowdown: bool) {
    let landed_from_high = avatar.takeoff_height + JUMP_LAND_RAISE <= avatar.origin[2];
    let scale = match avatar.move_timer {
        // The landing's timer ran out: the jump is over.
        t if t > LOW_LANDING_MS => {
            avatar.movement_state_bits &= !movement_state_bits::JUMPING;
            avatar.takeoff_height = 0.0;
            LOW_LANDING_SCALE
        }
        // The first landed frame starts the timer, longer for a high drop.
        0 if landed_from_high => {
            avatar.move_timer = HIGH_LANDING_MS;
            HIGH_LANDING_SCALE
        }
        0 => {
            avatar.move_timer = LOW_LANDING_MS;
            LOW_LANDING_SCALE
        }
        // Mid-landing frames keep their speed.
        _ => 1.0,
    };
    if jump_slowdown {
        avatar.velocity.iter_mut().for_each(|axis| *axis *= scale);
    }
}

/// How hard the keys push on this ground and in this stance.
fn accel_scale(avatar: &AvatarState, tick_vars: &TickLocals) -> f32 {
    let mut speed_up_rate = if tick_vars.floor_probe[4] & SURF_SLIDE != 0 || avatar.movement_state_bits & PMF_SLICK != 0 {
        SLICK_ACCEL
    } else if avatar.movement_state_bits & PMF_TIME_KNOCKBACK != 0 {
        KNOCKBACK_ACCEL
    } else {
        match posture_footing_kind(avatar) {
            PostureFooting::Prone | PostureFooting::DownedGrace => BELLY_SPEEDUP,
            PostureFooting::Crouch => SQUAT_SPEEDUP,
            PostureFooting::Stand => UPRIGHT_SPEEDUP,
        }
    };
    if avatar.movement_state_bits & PMF_TIME_HARDLANDING != 0 {
        speed_up_rate *= HARD_LANDING_ACCEL_SCALE;
    }
    speed_up_rate
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::NO_ENTITY_SLOT;

    fn landed(raise: f32, jump_slowdown: bool) -> f32 {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.takeoff_height = 100.0;
        avatar.origin = [0.0, 0.0, 100.0 + raise];
        avatar.velocity = [100.0, 0.0, 0.0];
        landing_slowdown(&mut avatar, jump_slowdown);
        avatar.velocity[0]
    }

    #[test]
    fn a_jump_out_of_water_splashes_unless_the_ground_is_silent() {
        const STONE: u32 = 18 << 20;
        const NOSTEPS: u32 = 0x2000;
        let mut avatar = AvatarState::ZERO;
        assert_eq!(leap::ground_surface_type(jump_surface_flags(&avatar, STONE)), 18);
        avatar.waterlevel = 1;
        let wet = jump_surface_flags(&avatar, STONE | 2);
        assert_eq!(leap::ground_surface_type(wet), WATER_SURFACE_TYPE);
        // The slick bit and the rest of the flags stay as they were.
        assert_eq!(wet & !SURFACE_TYPE_BITS, 2);
        assert_eq!(leap::ground_surface_type(jump_surface_flags(&avatar, STONE | NOSTEPS)), 0);
    }

    #[test]
    fn flat_landing_keeps_065() {
        assert!((landed(0.0, true) - 65.0).abs() < 1e-3);
    }

    #[test]
    fn landing_18_up_slows_to_half() {
        assert!((landed(18.0, true) - 50.0).abs() < 1e-3);
        assert!((landed(17.0, true) - 65.0).abs() < 1e-3);
    }

    #[test]
    fn slowdown_off_keeps_speed() {
        assert!((landed(0.0, false) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn a_long_landing_ends_the_jump() {
        let mut avatar = AvatarState::ZERO;
        avatar.movement_state_bits = movement_state_bits::JUMPING;
        avatar.move_timer = LOW_LANDING_MS + 1;
        avatar.takeoff_height = 50.0;
        landing_slowdown(&mut avatar, true);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::JUMPING, 0);
        assert_eq!(avatar.takeoff_height, 0.0);
    }

    #[test]
    fn players_are_not_ground_to_jump_off() {
        let mut avatar = AvatarState::ZERO;
        for (ground, on_player) in [(0, true), (17, true), (18, false), (NO_ENTITY_SLOT - 1, false), (NO_ENTITY_SLOT, false)] {
            avatar.standing_on = ground;
            assert_eq!(on_a_player(&avatar), on_player, "{ground}");
        }
    }

    #[test]
    fn slick_and_sliding_ground_push_at_two() {
        let mut avatar = AvatarState::ZERO;
        let mut tick_vars = crate::testing::tick_vars(true);
        assert_eq!(accel_scale(&avatar, &tick_vars), UPRIGHT_SPEEDUP);
        avatar.movement_state_bits = PMF_SLICK;
        assert_eq!(accel_scale(&avatar, &tick_vars), SLICK_ACCEL);
        avatar.movement_state_bits = PMF_TIME_KNOCKBACK;
        assert_eq!(accel_scale(&avatar, &tick_vars), KNOCKBACK_ACCEL);
        tick_vars.floor_probe[4] = SURF_SLIDE;
        assert_eq!(accel_scale(&avatar, &tick_vars), SLICK_ACCEL);
        avatar.movement_state_bits = PMF_TIME_HARDLANDING;
        assert_eq!(accel_scale(&avatar, &tick_vars), SLICK_ACCEL * HARD_LANDING_ACCEL_SCALE);
    }
}
