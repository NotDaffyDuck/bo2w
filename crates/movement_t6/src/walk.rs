//! Black Ops II's PM_WalkMove: the landing slowdown, a jump, friction, the
//! keys' push along the ground, the sliding and slick ground's push, then
//! the step and slide. It steps every frame, standing still too, so a
//! player at rest stays down on the floor.

use crate::bridge::{
    JumpCheckContext, JumpCheckResult, MoveBounds, Pml, StanceSurface,
    WalkMoveContext, accelerate, cmd_scale_walk, damage_scale_walk, jump,
    stance_surface_type, step_slide_move, walk_move_drop_damage_timer,
};
use playerstate_t6::game::{PlayerState, UserCmd, pm_flags};

use crate::air::air_move;
use crate::feel::{JUMP_LAND_RAISE, SLICK_ACCEL, KNOCKBACK_ACCEL};
use crate::footstep::WATER_SURFACE_TYPE;
use crate::frame::BodyAnims;
use crate::friction::{SURF_SLIDE, friction};
use crate::gjk::Bo2Collision;
use crate::movement_dir::set_movement_dir;
use crate::project::{clip_to_ground_plane, wish_dir};
use crate::slide_adjust::slide_adjust;

/// A trace's surface type bits (surfaceFlags >> 20).
const SURFACE_TYPE_BITS: u32 = 0x3f << 20;

/// The forced slick flag (pm_flags).
const PMF_SLICK: u32 = 0x4000_0000;

/// The knockback timer (pm_flags 0x100): pushed, the ground's grip is off.
const PMF_TIME_KNOCKBACK: u32 = 0x100;

/// The hard-landing stun (pm_flags 0x80) cuts the push to a quarter.
const PMF_TIME_HARDLANDING: u32 = 0x80;
const HARD_LANDING_ACCEL_SCALE: f32 = 0.25;

const ACCEL_PRONE: f32 = 19.0;
const ACCEL_CROUCH: f32 = 12.0;
const ACCEL_STAND: f32 = 9.0;

/// How long a landing's slowdown lasts (ms): a high landing, a low one, and
/// past the low one the jump is over.
const HIGH_LANDING_MS: i32 = 0x4b0;
const LOW_LANDING_MS: i32 = 0x708;

/// The speed kept on landing: from high up, and otherwise.
const HIGH_LANDING_SCALE: f32 = 0.5;
const LOW_LANDING_SCALE: f32 = 0.65;

pub(crate) fn walk_move<C: Bo2Collision>(
    ps: &mut PlayerState,
    pml: &mut Pml,
    cmd: &mut UserCmd,
    context: WalkMoveContext,
    bounds: MoveBounds,
    collision: &C,
    body: &mut BodyAnims,
) -> f32 {
    let feel = context.feel;
    if ps.pm_flags & pm_flags::JUMPING != 0 {
        landing_slowdown(ps, feel.jump_slowdown);
    }
    // Sprinting (not with sprint in any direction), the sideways keys count
    // for less.
    if !feel.omni && ps.pm_flags & pm_flags::SPRINTING != 0 {
        cmd.rightmove = ((cmd.rightmove as f32) * feel.sprint_strafe_speed_scale) as i32 as i8;
    }

    if jump_check(ps, pml, cmd, &context) {
        body.jump = true;
        return air_move(ps, pml, cmd, context.air, bounds, collision, feel.zombies, context.lunge);
    }

    friction(ps, pml, feel.jump_slowdown, context.lunge);

    let scale = crate::feel::omni_cmd_scale(ps, context.cmd_scale, feel);
    let scale = crate::feel::stamin_up_cmd_scale(ps, scale, feel);
    let command_scale = cmd_scale_walk(ps, cmd, scale) * damage_scale_walk(ps.damage_timer);
    walk_move_drop_damage_timer(ps, pml.frametime);

    let (mut wishdir, wishspeed) = wish_dir(pml.forward, pml.right, cmd.forwardmove, cmd.rightmove);
    clip_to_ground_plane(&mut wishdir, &pml.ground_trace);
    accelerate(
        ps,
        pml,
        &wishdir,
        wishspeed * context.weapon_move_scale * command_scale,
        accel_scale(ps, pml),
    );

    slide_adjust(ps, pml);

    if pml.ground_trace[4] & SURF_SLIDE != 0 || ps.pm_flags & (PMF_SLICK | PMF_TIME_KNOCKBACK) != 0 {
        ps.velocity[2] -= ps.gravity as f32 * pml.frametime;
    }

    clip_to_ground_plane(&mut ps.velocity, &pml.ground_trace);
    let step = if collision.gjk_on() {
        crate::gjk::step::step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            false,
            feel.zombies,
        )
    } else {
        step_slide_move(
            ps,
            pml,
            collision,
            bounds.mins,
            bounds.maxs,
            bounds.tracemask,
            None,
            feel.zombies,
        )
    };
    set_movement_dir(ps, pml, cmd);
    step
}

/// Jump_Check (walking and on a ladder): true when the player jumped. A jump
/// taken soon after landing one goes lower.
pub(crate) fn jump_check(
    ps: &mut PlayerState,
    pml: &mut Pml,
    cmd: &mut UserCmd,
    context: &WalkMoveContext,
) -> bool {
    let gate = JumpCheckContext {
        time_since_jump: cmd.server_time.wrapping_sub(ps.jump_time),
        old_buttons: context.old_buttons,
        stance_surface_type: stance_surface_type(ps) as u8,
    };
    let mut launch = context.jump;
    launch.crouch_jump_scale = jump::bo2_land_factor(ps, context.feel.jump_slowdown);
    if on_a_player(ps) {
        // Another player is not ground to jump off. A held jump is still
        // cleared, so it must be pressed again once off them.
        jump::check_gate(ps, cmd, gate);
        return false;
    }
    let floor = pml.ground_trace[4];
    pml.ground_trace[4] = jump_surface_flags(ps, floor);
    let launched = matches!(
        jump::check(ps, pml, cmd, gate, launch),
        JumpCheckResult::Launched { .. }
    );
    pml.ground_trace[4] = floor;
    launched
}

/// Jump_AddSurfaceEvent names the ground as PM_GroundSurfaceType does, so a
/// jump out of any water splashes; the shared jump reads the floor's type.
fn jump_surface_flags(ps: &PlayerState, floor: u32) -> u32 {
    if ps.waterlevel == 0 {
        return floor;
    }
    (floor & !SURFACE_TYPE_BITS) | ((WATER_SURFACE_TYPE as u32) << 20)
}

/// Standing on another player (BO2's G_IsEntWalkable says no to them).
fn on_a_player(ps: &PlayerState) -> bool {
    crate::ground::is_player(ps.ground_entity_num)
}

/// Jump_ApplySlowdown: the first landed frame keeps half the speed when the
/// player came down at least 18 units above the take-off, 0.65 otherwise,
/// and starts the landing's timer; once that runs past the low landing's
/// time the jump is over.
fn landing_slowdown(ps: &mut PlayerState, jump_slowdown: bool) {
    let mut scale = 1.0_f32;
    if ps.pm_time <= LOW_LANDING_MS {
        if ps.pm_time == 0 {
            if ps.jump_origin_z + JUMP_LAND_RAISE <= ps.origin[2] {
                ps.pm_time = HIGH_LANDING_MS;
                scale = HIGH_LANDING_SCALE;
            } else {
                ps.pm_time = LOW_LANDING_MS;
                scale = LOW_LANDING_SCALE;
            }
        }
    } else {
        ps.pm_flags &= !pm_flags::JUMPING;
        ps.jump_origin_z = 0.0;
        scale = LOW_LANDING_SCALE;
    }
    if jump_slowdown {
        ps.velocity[0] *= scale;
        ps.velocity[1] *= scale;
        ps.velocity[2] *= scale;
    }
}

/// How hard the keys push on this ground and in this stance.
fn accel_scale(ps: &PlayerState, pml: &Pml) -> f32 {
    let mut accel = if pml.ground_trace[4] & SURF_SLIDE != 0 || ps.pm_flags & PMF_SLICK != 0 {
        SLICK_ACCEL
    } else if ps.pm_flags & PMF_TIME_KNOCKBACK != 0 {
        KNOCKBACK_ACCEL
    } else {
        match stance_surface_type(ps) {
            StanceSurface::Prone | StanceSurface::LastStand => ACCEL_PRONE,
            StanceSurface::Crouch => ACCEL_CROUCH,
            StanceSurface::Stand => ACCEL_STAND,
        }
    };
    if ps.pm_flags & PMF_TIME_HARDLANDING != 0 {
        accel *= HARD_LANDING_ACCEL_SCALE;
    }
    accel
}

#[cfg(test)]
mod tests {
    use super::*;
    use playerstate_t6::game::ENTITYNUM_NONE;

    fn landed(raise: f32, jump_slowdown: bool) -> f32 {
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.jump_origin_z = 100.0;
        ps.origin = [0.0, 0.0, 100.0 + raise];
        ps.velocity = [100.0, 0.0, 0.0];
        landing_slowdown(&mut ps, jump_slowdown);
        ps.velocity[0]
    }

    #[test]
    fn a_jump_out_of_water_splashes_unless_the_ground_is_silent() {
        const STONE: u32 = 18 << 20;
        const NOSTEPS: u32 = 0x2000;
        let mut ps = PlayerState::ZERO;
        assert_eq!(jump::ground_surface_type(jump_surface_flags(&ps, STONE)), 18);
        ps.waterlevel = 1;
        let wet = jump_surface_flags(&ps, STONE | 2);
        assert_eq!(jump::ground_surface_type(wet), WATER_SURFACE_TYPE);
        // The slick bit and the rest of the flags stay as they were.
        assert_eq!(wet & !SURFACE_TYPE_BITS, 2);
        assert_eq!(jump::ground_surface_type(jump_surface_flags(&ps, STONE | NOSTEPS)), 0);
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
        let mut ps = PlayerState::ZERO;
        ps.pm_flags = pm_flags::JUMPING;
        ps.pm_time = LOW_LANDING_MS + 1;
        ps.jump_origin_z = 50.0;
        landing_slowdown(&mut ps, true);
        assert_eq!(ps.pm_flags & pm_flags::JUMPING, 0);
        assert_eq!(ps.jump_origin_z, 0.0);
    }

    #[test]
    fn players_are_not_ground_to_jump_off() {
        let mut ps = PlayerState::ZERO;
        for (ground, on_player) in [(0, true), (17, true), (18, false), (ENTITYNUM_NONE - 1, false), (ENTITYNUM_NONE, false)] {
            ps.ground_entity_num = ground;
            assert_eq!(on_a_player(&ps), on_player, "{ground}");
        }
    }

    #[test]
    fn slick_and_sliding_ground_push_at_two() {
        let mut ps = PlayerState::ZERO;
        let mut pml = crate::testing::pml(true);
        assert_eq!(accel_scale(&ps, &pml), ACCEL_STAND);
        ps.pm_flags = PMF_SLICK;
        assert_eq!(accel_scale(&ps, &pml), SLICK_ACCEL);
        ps.pm_flags = PMF_TIME_KNOCKBACK;
        assert_eq!(accel_scale(&ps, &pml), KNOCKBACK_ACCEL);
        pml.ground_trace[4] = SURF_SLIDE;
        assert_eq!(accel_scale(&ps, &pml), SLICK_ACCEL);
        ps.pm_flags = PMF_TIME_HARDLANDING;
        assert_eq!(accel_scale(&ps, &pml), SLICK_ACCEL * HARD_LANDING_ACCEL_SCALE);
    }
}
