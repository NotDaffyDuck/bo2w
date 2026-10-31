//! Black Ops II's PmoveSingle: one frame of a player's movement in BO2's own
//! order. The keys are settled first (a knife charge runs forward, the dead
//! and downed do not walk, lying down to aim holds still), then the view,
//! aim, sprint and stance. Then the player's state picks the move: flying,
//! the end-of-match screen, linked to something, on a turret, climbing a
//! ledge, or the ordinary dive, ladder, knife charge, walk or air move.

use crate::bridge::{
    ScopeWishArgs, RungGrabArgs, SolidProbe, SweepAsk, RungGripSource,
    RungProbeContact, ClimbPillSweep,
    ClimbBoneOffsetSource,
    ClimbClipDuration, HullExtents, PRONE_LERP_MS, TickLocals, MotionTickOutcome, OneTickMotionArgs,
    DashVerdict, PostureShift, calc_melee_charge_time,
    drop_timers, mantle, melee_charge_move, ufo, update_ads_frac,
    update_ads_intent,
};
use playerstate_t6::game::{
    MOVE_MODE_WALK_ATTACHED, AvatarState, InputCommand, movement_state_bits,
};
use playerstate_t6::game::NO_ENTITY_SLOT;
use playerstate_t6::game::MOVE_KIND_DEAD;
use playerstate_t6::game::MOVE_KIND_DEAD_TETHERED;
use playerstate_t6::game::MOVE_MODE_INTERLUDE;
use playerstate_t6::game::MOVE_MODE_DOWNED;

use crate::dive;
use playerstate_t6::game::entity_flag_bits::TURRET_MASK;
use crate::gjk::{Bo2Collision, PenetrationChecks};
use crate::stance::DuckContext;

/// A frame runs at least 1 ms and at most 200 ms of time.
const MIN_MSEC: i32 = 1;
const MAX_MSEC: i32 = 200;

/// A late command catches up at most this much time (Pmove's 0x96).
const CATCH_UP_MSEC: i32 = 150;

/// Each catch-up move runs at most this long (Pmove's 0x42).
const STEP_MSEC: i32 = 66;

/// A dead body linked to something keeps bumping into players (linkFlags 4).
const LINK_KEEPS_BODY_CONTENTS: u32 = 4;

/// The BO2 random seed (holdrand) steps four times from its start.
const HOLDRAND_MUL: u32 = 0x343fd;
const HOLDRAND_ADD: u32 = 0x0026_9ec3;

/// The legs' animations BO2's movement starts (BG_AnimScriptEvent's jump and
/// land), and the slope they are picked on; the sim plays them (sim
/// `t6/playeranim.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyAnims {
    /// ANIM_ET_JUMP: a jump, walking off an edge, losing a ladder in the
    /// air, the end of a vault.
    pub jump: bool,
    /// ANIM_ET_LAND: a landing from over 12 units, not out of a dive.
    pub land: bool,
    /// averagePitch: the ground's pitch along his facing, averaged over the
    /// command's moves (`slope.rs`), for the legs' `slope` condition.
    pub average_pitch: f32,
}

impl BodyAnims {
    fn merge(&mut self, other: BodyAnims) {
        self.jump |= other.jump;
        self.land |= other.land;
    }
}

/// Black Ops II's Pmove: a late command catches up at most 150 ms, in moves
/// of at most 66 ms. It takes the player, command and context and gives
/// back the result (the last move's). It moves as the server does.
pub fn pmove<C: Bo2Collision, L: ClimbClipDuration, R: ClimbBoneOffsetSource>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    setup: OneTickMotionArgs,
    solids: &C,
    anim_len: &L,
    delta_src: &R,
) -> MotionTickOutcome {
    pmove_with_body(avatar, ucmd, setup, solids, anim_len, delta_src, true).0
}

/// `pmove`, also giving the legs' jump and land animations any of its moves
/// started, and the slope pitch they averaged. `server` is BO2's server move
/// (not a client's prediction): only it sends the slide sound events.
pub fn pmove_with_body<C: Bo2Collision, L: ClimbClipDuration, R: ClimbBoneOffsetSource>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    mut setup: OneTickMotionArgs,
    solids: &C,
    anim_len: &L,
    delta_src: &R,
    server: bool,
) -> (MotionTickOutcome, BodyAnims) {
    let mut body = BodyAnims::default();
    // BO2 clears the pitch history with the rest of pmove_t for each command.
    let mut pitch = crate::slope::PitchHistory::default();
    let authority_clock_ms = ucmd.authority_clock_ms;
    // The platform the player rides carries them first, over the whole
    // command (before the catch-up below).
    crate::mover::pre_step(avatar, ucmd, solids);
    if authority_clock_ms.wrapping_sub(avatar.last_cmd_ms) > CATCH_UP_MSEC {
        avatar.last_cmd_ms = authority_clock_ms.wrapping_sub(CATCH_UP_MSEC);
    }
    loop {
        let step = authority_clock_ms.wrapping_sub(avatar.last_cmd_ms).min(STEP_MSEC);
        ucmd.authority_clock_ms = avatar.last_cmd_ms.wrapping_add(step);
        let outcome = pmove_single(avatar, ucmd, setup, solids, anim_len, delta_src, &mut body, &mut pitch, server);
        if ucmd.authority_clock_ms == authority_clock_ms {
            body.average_pitch = pitch.average();
            return (outcome, body);
        }
        // The next move's last command is this one.
        setup.last_tick_buttons = ucmd.buttons;
        setup.walk.last_tick_buttons = ucmd.buttons;
        setup.old_move = [ucmd.thrust_ahead, ucmd.thrust_lateral];
    }
}

/// One move of Black Ops II movement (PmoveSingle).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn pmove_single<C: Bo2Collision, L: ClimbClipDuration, R: ClimbBoneOffsetSource>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    setup: OneTickMotionArgs,
    solids: &C,
    anim_len: &L,
    delta_src: &R,
    body: &mut BodyAnims,
    pitch: &mut crate::slope::PitchHistory,
    server: bool,
) -> MotionTickOutcome {
    let mut frame_ctx = bo2_context(setup);
    let feel = frame_ctx.walk.feel;
    // The crouch and stand-up checks (BO2's is_not_penetrating).
    let penetration = PenetrationChecks(solids);

    // A knife charge runs straight ahead whatever the keys say.
    if avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE != 0 {
        set_keys(ucmd, 127, 0);
    }
    // A dead body passes through players unless its link says otherwise.
    if avatar.movement_mode >= MOVE_KIND_DEAD && avatar.attach_bits & LINK_KEEPS_BODY_CONTENTS == 0 {
        frame_ctx.bounds.collide_filter &= !crate::stance::BO2_GETUP_IGNORES;
    }

    let msec = ucmd.authority_clock_ms.wrapping_sub(avatar.last_cmd_ms).clamp(MIN_MSEC, MAX_MSEC);
    // The water level is worked out again after the view; until then it is
    // dry (BO2 reads the lying-down aim before it).
    avatar.waterlevel = 0;
    // Lying down and aiming holds still (decided before anything moves).
    crate::feel::prone_aim(
        avatar,
        ucmd,
        frame_ctx.old_move,
        crate::stance::sniper_scope_raised(avatar, frame_ctx.ads_intent),
    );
    // The dead do not walk.
    if avatar.movement_mode >= MOVE_KIND_DEAD {
        set_keys(ucmd, 0, 0);
    }
    avatar.last_cmd_ms = ucmd.authority_clock_ms;

    let mut tick_vars = TickLocals {
        fwd_axis: ORIGIN_VEC,
        right_axis: ORIGIN_VEC,
        up_axis: ORIGIN_VEC,
        step_millis_elapsed: (msec as f32) * 0.001_f32,
        frame_ms: msec,
        walking: 0,
        standing_on_floor: 0, near_floor_contact: 0,
        floor_probe: [0; 11],
        prior_tick_position: avatar.origin, prior_tick_speed_vec: avatar.velocity,
        frame_seed: holdrand(avatar.look_angles[1], ucmd.authority_clock_ms),
    };

    let old_yaw = avatar.look_angles[1];
    crate::view::update_view_angles(avatar, ucmd, frame_ctx.look_rotation);
    // Lying down, the view is held to the body's turn.
    crate::prone_turn::bo2_prone_view(avatar, ucmd, msec, solids, feel.zombies, old_yaw);
    crate::water::set_water_level(avatar, solids);
    // A dive into water is over once it stops rising (BO2's Dtp_CanMove,
    // dtp_new_trajectory on).
    if dive::active(avatar) && avatar.waterlevel > 0 && avatar.velocity[2] <= 0.0 {
        dive::interrupt(avatar, ucmd.authority_clock_ms);
    }
    (tick_vars.fwd_axis, tick_vars.right_axis, tick_vars.up_axis) = game_math::euler_basis(avatar.look_angles);

    // Noclip and UFO (the mod menu's fly mode): no ground, no ladder, no
    // stance, mantle or dive.
    if ufo::active(avatar) {
        avatar.standing_on = NO_ENTITY_SLOT;
        ladder_to_fall(avatar);
        drop_timers(avatar, &tick_vars);
        ufo::fly(avatar, ucmd, &tick_vars);
        return result(tick_vars, frame_ctx.bounds, None, false, 0.0);
    }

    mantle::clear_hint(avatar);

    // A dive to prone starts from a sprint, so it is decided before the
    // sprint and stance updates see the prone press; while it lasts it owns
    // sprint, stance and movement. After its slide the player lies still a
    // moment, and a slide that has stopped is over before the stance update.
    if feel.dive {
        if dive::pausing(avatar, ucmd.authority_clock_ms) {
            set_keys(ucmd, 0, 0);
        }
        dive::end_stopped_slide(avatar, ucmd.authority_clock_ms);
    }
    let dive_stance = dive::owns_stance(avatar, ucmd.authority_clock_ms);
    let dive_start =
        feel.dive && !frame_ctx.gun_forbids_lying && dive::wants(avatar, ucmd, frame_ctx.last_tick_buttons);
    let diving = dive_start || dive::active(avatar);

    // The gun can aim through a dive (asked for, 10-09).
    let ads_cmd = dive::ads_cmd(avatar, ucmd, diving);
    let _ads = update_ads_intent(avatar, &ads_cmd, frame_ctx.last_tick_buttons, frame_ctx.ads_intent);
    let mut sprint = frame_ctx.sprint;
    sprint.gun_dash_cap_secs =
        crate::feel::stamin_up_sprint_time(avatar, sprint.gun_dash_cap_secs, feel);
    if avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT) != 0 {
        sprint.rise_space_free = !penetration
            .trace(SweepAsk {
                start: avatar.origin,
                end: avatar.origin,
                lo_corner: frame_ctx.bounds.lo_corner,
                hi_corner: frame_ctx.bounds.hi_corner,
                collide_filter: frame_ctx.bounds.collide_filter & !crate::stance::BO2_GETUP_IGNORES,
            })
            .fully_embedded;
    }
    let sprint_change = if diving {
        DashVerdict::Unchanged
    } else {
        // Sprint in any direction (feel.omni); a held prone key does not end
        // the sprint while getting up from a dive.
        let sprint_cmd = crate::feel::omni_sprint_cmd(&dive::stance_cmd(avatar, ucmd), feel);
        crate::sprint::update(avatar, &sprint_cmd, sprint, frame_ctx.gun_forbids_lying)
    };

    // The end-of-match screen: aim and sprint settle, nothing moves.
    if avatar.movement_mode == MOVE_MODE_INTERLUDE {
        ladder_to_fall(avatar);
        update_ads_frac(avatar, tick_vars.frame_ms, frame_ctx.ads_frac);
        return result(tick_vars, frame_ctx.bounds, None, false, 0.0);
    }

    crate::feel::update_walking_flag(avatar, ucmd);

    let duck = duck_pass(avatar, ucmd, &ads_cmd, solids, &frame_ctx, dive_stance, diving, tick_vars.step_millis_elapsed);
    let mut stance_event = duck.anim;
    let mut reset_torso =
        duck.change != PostureShift::Unchanged || sprint_change == DashVerdict::Started;
    let mut bounds = duck.bounds;
    update_ads_frac(avatar, tick_vars.frame_ms, frame_ctx.ads_frac);

    // Linked to something (a vehicle, a script's link, a linked body): the
    // link owns the origin, so the player is held still.
    if avatar.movement_mode == MOVE_MODE_WALK_ATTACHED || avatar.movement_mode == MOVE_KIND_DEAD_TETHERED {
        hold_still(avatar, &mut tick_vars);
        drop_timers(avatar, &tick_vars);
        footsteps(avatar, &tick_vars, ucmd, &frame_ctx, pitch);
        // A dive is cut short by a link.
        if avatar.movement_mode == MOVE_MODE_WALK_ATTACHED && dive::active(avatar) {
            dive::interrupt(avatar, ucmd.authority_clock_ms);
        }
        return result(tick_vars, bounds, stance_event, reset_torso, 0.0);
    }

    // Going down leaves any ladder and any turret.
    if avatar.movement_mode == MOVE_MODE_DOWNED {
        ladder_to_fall(avatar);
        avatar.e_flags &= !TURRET_MASK;
    }

    // On a turret the gun does the moving.
    if avatar.e_flags & TURRET_MASK != 0 {
        hold_still(avatar, &mut tick_vars);
        drop_timers(avatar, &tick_vars);
        footsteps(avatar, &tick_vars, ucmd, &frame_ctx, pitch);
        return result(tick_vars, bounds, stance_event, reset_torso, 0.0);
    }

    let mut climb_started = false;
    if avatar.movement_state_bits & movement_state_bits::MANTLE == 0 {
        // A climb's own path owns the ledge; the ground's solid fix-up during
        // it would push the player back off.
        body.merge(crate::ground::floor_probe(avatar, &mut tick_vars, bounds, solids, feel, ucmd.authority_clock_ms, server));
        let mut tracer = WorldMantleSweep { solids };
        let check = crate::mantle::CheckContext {
            held_buttons: ucmd.buttons,
            heading_planar: level_heading(&tick_vars),
            collide_filter: bounds.collide_filter,
            mp: !feel.zombies,
        };
        // bo2zm: a roof hatch over the player comes before any ledge.
        let hatch = !diving
            && feel.zombies
            && crate::mantle::hatch_check(avatar, ucmd.buttons, tick_vars.fwd_axis, tick_vars.up_axis, &mut tracer, anim_len, delta_src);
        climb_started = match hatch {
            true => avatar.movement_state_bits & movement_state_bits::MANTLE != 0,
            false => !diving && crate::mantle::check(avatar, &check, &mut tracer, anim_len, delta_src),
        };
        if climb_started {
            mantle::mark_bo2_weapon(avatar, feel, anim_len);
        }
    }

    if avatar.movement_state_bits & movement_state_bits::MANTLE != 0 {
        ladder_to_fall(avatar);
        lift_off(avatar, &mut tick_vars);
        if climb_started {
            // BO2 runs the aim, sprint, walking and stance updates again on
            // the frame a climb starts, now off the ground: the aim lets go
            // (no aiming while climbing) and the view's height steps again.
            update_ads_intent(
                avatar,
                &ads_cmd,
                frame_ctx.last_tick_buttons,
                ScopeWishArgs { sights_permitted: false, ..frame_ctx.ads_intent },
            );
            let sprint_cmd = crate::feel::omni_sprint_cmd(&dive::stance_cmd(avatar, ucmd), feel);
            crate::sprint::update(avatar, &sprint_cmd, sprint, frame_ctx.gun_forbids_lying);
            crate::feel::update_walking_flag(avatar, ucmd);
            let again =
                duck_pass(avatar, ucmd, &ads_cmd, solids, &frame_ctx, dive_stance, diving, tick_vars.step_millis_elapsed);
            bounds = again.bounds;
            stance_event = again.anim.or(stance_event);
            reset_torso |= again.change != PostureShift::Unchanged;
        }
        body.jump |= crate::mantle::tick_climb(avatar, tick_vars.frame_ms, ucmd.authority_clock_ms, !feel.zombies, anim_len, delta_src);
        return result(tick_vars, bounds, stance_event, reset_torso, 0.0);
    }

    // The body's slope pitches follow this frame's ground.
    crate::prone_turn::bo2_prone_pitch(avatar, &tick_vars, solids, feel.zombies);
    drop_timers(avatar, &tick_vars);
    if avatar.movement_mode >= MOVE_MODE_DOWNED {
        crate::dead::dead_move(avatar, &tick_vars);
    }

    if !diving {
        let mut ladder = WorldLadderProbe {
            solids,
            bounds,
            last_entity: None,
        };
        let check = RungGrabArgs {
            authority_clock_ms: ucmd.authority_clock_ms,
            on_foot: tick_vars.walking != 0,
            ahead_planar: level_heading(&tick_vars),
            thrust_ahead: ucmd.thrust_ahead,
        };
        body.jump |= crate::ladder::check_ladder_move(avatar, &mut tick_vars, check, &mut ladder);
    }

    let view_change = if diving {
        if dive_start {
            dive::start(avatar, &mut tick_vars, ucmd, frame_ctx.walk.jump.leap_height);
        }
        dive::advance(avatar, &tick_vars, ucmd, bounds, solids, frame_ctx.walk.jump.leap_height, frame_ctx.air)
    } else if avatar.movement_state_bits & movement_state_bits::LADDER != 0 {
        crate::ladder::ladder_move(avatar, &mut tick_vars, ucmd, frame_ctx.walk, bounds, solids, body)
    } else if let Some(lunge) = frame_ctx.walk.lunge {
        // bo2mp's knife lunge (BO2's COD4-style charge) launches here, then
        // the walk or air move runs it with the lunge's own friction.
        crate::bridge::bo2_lunge_update(avatar, &tick_vars, lunge);
        let swimming = avatar.waterlevel >= crate::water::SWIM_LEVEL;
        let airborne = tick_vars.walking == 0;
        if swimming {
            crate::water::swim_move(avatar, &mut tick_vars, ucmd, frame_ctx.air, bounds, solids, feel.zombies, Some(lunge))
        } else if airborne {
            crate::air::air_move(avatar, &tick_vars, ucmd, frame_ctx.air, bounds, solids, feel.zombies, Some(lunge))
        } else {
            crate::walk::walk_move(avatar, &mut tick_vars, ucmd, frame_ctx.walk, bounds, solids, body)
        }
    } else {
        calc_melee_charge_time(avatar, frame_ctx.melee_charge, frame_ctx.knife_reach);
        if avatar.movement_state_bits & movement_state_bits::MELEE_CHARGE != 0 {
            let HullExtents { lo_corner: lo, hi_corner: hi, collide_filter: mask } = bounds;
            melee_charge_move(avatar, &tick_vars, lo, hi, mask, solids, feel.zombies)
        } else if avatar.waterlevel >= crate::water::SWIM_LEVEL {
            crate::water::swim_move(avatar, &mut tick_vars, ucmd, frame_ctx.air, bounds, solids, feel.zombies, None)
        } else if 0 == tick_vars.walking {
            crate::air::air_move(avatar, &tick_vars, ucmd, frame_ctx.air, bounds, solids, feel.zombies, None)
        } else {
            crate::walk::walk_move(avatar, &mut tick_vars, ucmd, frame_ctx.walk, bounds, solids, body)
        }
    };

    body.merge(crate::ground::floor_probe(avatar, &mut tick_vars, bounds, solids, feel, ucmd.authority_clock_ms, server));
    footsteps(avatar, &tick_vars, ucmd, &frame_ctx, pitch);
    // BO2 runs the gun between the two; ours runs after the move.
    crate::foliage::foliage_sounds(avatar, ucmd.authority_clock_ms, bounds, solids);
    crate::snap::end_tick_velocity(avatar, &tick_vars);
    result(tick_vars, bounds, stance_event, reset_torso, view_change)
}

/// BO2's random seed for the frame: the view's yaw in whole degrees plus the
/// frame's time, stepped four times.
pub fn holdrand(look_heading: f32, authority_clock_ms: i32) -> i32 {
    let seed = (look_heading as i32).wrapping_add(authority_clock_ms) as u32;
    let stepped = (0..4).fold(seed, |acc, _| {
        acc.wrapping_mul(HOLDRAND_MUL).wrapping_add(HOLDRAND_ADD)
    });
    stepped as i32
}

/// This crate is always Black Ops II, whatever rules it is handed.
fn bo2_context(mut ctx: OneTickMotionArgs) -> OneTickMotionArgs {
    let scale = &mut ctx.walk.cmd_scale;
    if scale.prone_lerp_ms == 0 {
        scale.prone_lerp_ms = PRONE_LERP_MS;
    }
    ctx
}

/// Leaving a ladder any way but the ground lets the player fall off it.
fn ladder_to_fall(avatar: &mut AvatarState) {
    let on_ladder = avatar.movement_state_bits & movement_state_bits::LADDER != 0;
    if on_ladder {
        avatar.movement_state_bits = (avatar.movement_state_bits & !movement_state_bits::LADDER) | movement_state_bits::LADDER_FALL;
    }
}

/// Linked or on a turret: off any ladder, off the ground, not moving.
fn hold_still(avatar: &mut AvatarState, tick_vars: &mut TickLocals) {
    ladder_to_fall(avatar);
    lift_off(avatar, tick_vars);
    tick_vars.near_floor_contact = 0;
    avatar.velocity = ORIGIN_VEC;
}

/// Detaches the player from the ground for this frame.
fn lift_off(avatar: &mut AvatarState, tick_vars: &mut TickLocals) {
    tick_vars.standing_on_floor = 0;
    tick_vars.walking = 0;
    avatar.standing_on = NO_ENTITY_SLOT;
}

fn set_keys(ucmd: &mut InputCommand, ahead: i8, side: i8) {
    ucmd.thrust_lateral = side;
    ucmd.thrust_ahead = ahead;
}

fn level_heading(tick_vars: &TickLocals) -> [f32; 2] {
    let [x, y, _] = tick_vars.fwd_axis;
    [x, y]
}

const ORIGIN_VEC: [f32; 3] = [0.0; 3];

/// PM_Footsteps (footstep.rs): the walk rhythm and its footstep.
fn footsteps(
    avatar: &mut AvatarState,
    tick_vars: &TickLocals,
    ucmd: &InputCommand,
    frame_ctx: &OneTickMotionArgs,
    pitch: &mut crate::slope::PitchHistory,
) {
    crate::gait_cycle::footsteps(avatar, tick_vars, ucmd, frame_ctx.walk.cmd_scale, frame_ctx.walk.feel, pitch);
}

/// What one stance pass leaves.
struct DuckPass {
    bounds: HullExtents,
    anim: Option<u8>,
    change: PostureShift,
}

/// BO2's PM_CheckDuck: the stance keys, the 1.8 s prone lock, the view's
/// height and the box, and lying down drops the aim. It runs again on the
/// frame a climb starts.
#[allow(clippy::too_many_arguments)]
fn duck_pass<C: Bo2Collision>(
    avatar: &mut AvatarState,
    ucmd: &mut InputCommand,
    ads_cmd: &InputCommand,
    solids: &C,
    frame_ctx: &OneTickMotionArgs,
    dive_stance: bool,
    diving: bool,
    step_millis_elapsed: f32,
) -> DuckPass {
    let feel = frame_ctx.walk.feel;
    let previous_stance = avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT);
    // From the frame after a dive's take-off to the end of its still moment
    // the view drops to prone; a get-up pressed meanwhile is kept.
    let dive_prone = if dive_stance {
        dive::queue_getup(avatar, ucmd, frame_ctx.last_tick_buttons);
        dive::force_prone(avatar);
        true
    } else {
        !diving && dive::holds_prone(avatar, ucmd, frame_ctx.last_tick_buttons)
    };
    // A held prone key does not drop the player back down while they get up
    // from a dive. Buttons the stance update clears stay cleared in the real
    // command.
    let mut stance_cmd = dive::stance_cmd(avatar, ucmd);
    let held_back = ucmd.buttons & !stance_cmd.buttons;
    let duck = crate::stance::check_duck(
        avatar,
        &mut stance_cmd,
        solids,
        DuckContext {
            body: frame_ctx.bounds,
            prone_barred: frame_ctx.gun_forbids_lying,
            zombies: feel.zombies,
            dive_owns: diving || dive_prone,
        },
    );
    ucmd.buttons &= stance_cmd.buttons | held_back;
    let stance_event = duck.anim;
    let stance_change = duck.change;
    // Dropping to prone locks the stance for 1.8 s (a dive's landing and last
    // stand do not).
    if stance_change == PostureShift::WentFlat
        && avatar.movement_mode != MOVE_MODE_DOWNED
        && avatar.move_timer == 0
        && !dive::any(avatar)
    {
        avatar.movement_state_bits |= movement_state_bits::JUMPING;
        avatar.move_timer = 1800;
    }
    dive::finish_getup(avatar, ucmd);
    let mut bounds =
        crate::stance::finish_duck(avatar, duck.tail, frame_ctx.bounds, ucmd.authority_clock_ms, step_millis_elapsed);
    if dive::active(avatar) {
        // Through a dive the body shrinks as the view drops.
        bounds.hi_corner[2] = dive::hull_height(avatar);
    }
    // A dive's push does not drop the aim on lying down.
    crate::stance::bo2_lie_down(
        avatar,
        ads_cmd,
        solids,
        bounds,
        previous_stance & movement_state_bits::LYING_FLAT != 0,
    );
    DuckPass { bounds, anim: stance_event, change: stance_change }
}

fn result(
    tick_vars: TickLocals,
    bounds: HullExtents,
    stance_event: Option<u8>,
    reset_torso: bool,
    view_change: f32,
) -> MotionTickOutcome {
    MotionTickOutcome { pml: tick_vars, bounds, stance_event, reset_torso, view_change }
}

/// The mantle's capsule sweeps run through the world's collision.
struct WorldMantleSweep<'a, C: SolidProbe> {
    solids: &'a C,
}

impl<C: SolidProbe> ClimbPillSweep for WorldMantleSweep<'_, C> {
    fn trace(
        &mut self,
        from_pt: [f32; 3],
        end: [f32; 3],
        box_lo: [f32; 3],
        box_hi: [f32; 3],
        solid_filter: u32,
    ) -> trace_t6::Trace {
        let query = SweepAsk { start: from_pt, end, lo_corner: box_lo, hi_corner: box_hi, collide_filter: solid_filter };
        self.solids.trace(query)
    }
}

/// The ladder probe: a slimmer box from knee height, swept toward the
/// ladder. It remembers which entity it last hit, for a ladder on a moving
/// thing.
struct WorldLadderProbe<'a, C: SolidProbe> {
    solids: &'a C,
    bounds: HullExtents,
    last_entity: Option<i32>,
}

/// The ladder probe is this much slimmer on each side, and starts this high.
const LADDER_PROBE_INSET: f32 = 6.0;
const LADDER_PROBE_FLOOR: f32 = 8.0;

impl<C: SolidProbe> RungGripSource for WorldLadderProbe<'_, C> {
    fn rung_probe(&mut self, at: [f32; 3], heading: [f32; 3], dist: f32) -> Option<RungProbeContact> {
        let reach_to = core::array::from_fn(|i| at[i] + heading[i] * dist);
        let [lo_x, lo_y, _] = self.bounds.lo_corner;
        let [hi_x, hi_y, hi_z] = self.bounds.hi_corner;
        let probe_lo = [lo_x + LADDER_PROBE_INSET, lo_y + LADDER_PROBE_INSET, LADDER_PROBE_FLOOR];
        let probe_hi_z = if hi_z < LADDER_PROBE_FLOOR { probe_lo[2] } else { hi_z };
        let probe_hi = [hi_x - LADDER_PROBE_INSET, hi_y - LADDER_PROBE_INSET, probe_hi_z];
        let strike = self.solids.trace(SweepAsk {
            start: at,
            end: reach_to,
            lo_corner: probe_lo,
            hi_corner: probe_hi,
            collide_filter: self.bounds.collide_filter,
        });
        self.last_entity = (strike.impact_category == trace_t6::TraceHitType::Entity)
            .then(|| i32::from(trace_t6::trace_get_entity_hit_id(&strike)));
        if strike.fraction >= 1.0 {
            return None;
        }
        Some(RungProbeContact {
            travelled: strike.fraction,
            facing: strike.normal,
            surf_bits: strike.sflags,
        })
    }
}

impl<C: SolidProbe> crate::ladder::LadderHitEntity for WorldLadderProbe<'_, C> {
    fn last_hit_entity(&self) -> Option<i32> {
        self.last_entity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{
        ScopeBlendArgs, ScopeWishArgs, AirborneStepArgs, Bo2Feel, StrideScaleArgs,
        FixedClimbClipDuration, LeapKickoffArgs, LungeGunTimings, DashArgs,
    };
    use crate::bridge::LookRotationLimit;
    use crate::bridge::FootedStepArgs;
    use crate::bridge::StillClimbOffset;
    use crate::bridge::eye_level;
    use playerstate_t6::game::buttons;
    use trace_t6::Trace;

    /// A floor at z = 0 and nothing else.
    struct Floor;

    impl Bo2Collision for Floor {}

    impl SolidProbe for Floor {
        fn trace(&self, input: SweepAsk) -> Trace {
            let start = input.start[2] + input.lo_corner[2];
            let end = input.end[2] + input.lo_corner[2];
            let mut strike = Trace::new();
            if start < 0.0 {
                strike.starts_inside = true;
                strike.fully_embedded = end < 0.0;
                strike.fraction = 0.0_f32;
                strike.normal = [0.0, 0.0, 1.0];
                return strike;
            }
            if end < 0.0 {
                let fraction = start / (start - end);
                strike.fraction = fraction;
                strike.normal = [0.0, 0.0, 1.0];
                strike.walkable = true;
                strike.impact_category = trace_t6::TraceHitType::Entity;
                strike.struck_entity_num = trace_t6::ENTITYNUM_WORLD;
            }
            strike
        }
    }

    /// Move settings as the sim hands them over, with MW2's rules (the frame
    /// switches BO2's on).
    fn frame_ctx() -> OneTickMotionArgs {
        let feel = Bo2Feel::BO2_MP;
        let (off, on) = (false, true);
        let air = AirborneStepArgs {
            observer_pace_ratio: 1.0,
            concussed_fall_ratio: 1.0,
            concussed_fall_offset: 0.0,
        };
        let cmd_scale = StrideScaleArgs {
            retreat_pace_ratio: 0.7, sidestep_pace_ratio: 0.8,
            dash_pace_ratio: 1.5, downed_crawl_pace_ratio: 0.15,
            gun_stride_ratio: 1.0, scoped_carry_pace: 1.0,
            shellshock_movement_scale: 1.0, prone_lerp_ms: 0,
        };
        let jump = LeapKickoffArgs {
            leap_height: 39.0_f32, dive: off, squat_leap_ratio: 1.0, rung_kickoff_speed: 128.0,
        };
        let walk = FootedStepArgs {
            cmd_scale, gun_carry_pace: 1.0, last_tick_buttons: 0, jump, air, feel, lunge: None,
        };
        let sprint = DashArgs {
            gun_dash_cap_secs: 4000, endless_dash: off, dash_least_secs: 1.0,
            dash_warmup_secs: 0.0, dash_thrust_floor: 105, rise_space_free: on,
            dash_refill_hold_secs: 0.0,
        };
        let rate = 1.0 / 200.0;
        OneTickMotionArgs {
            walk,
            air,
            bounds: HullExtents {
                collide_filter: 0x0281_0011, hi_corner: [15.0_f32, 15.0, 70.0], lo_corner: [-15.0_f32, -15.0, 0.0],
            },
            look_rotation: LookRotationLimit { tilt_upper_limit: 85.0, tilt_lower_limit: 85.0, free_tilt_flag: off },
            sprint,
            ads_intent: ScopeWishArgs { sights_permitted: on, scoped_weapon: off, dash_keeps_scope: off },
            ads_frac: ScopeBlendArgs {
                scoped_aiming: off,
                scope_raise_speed: rate,
                scope_lower_speed: rate,
                rack_bolt_when_scoped: on,
                fire_only_when_scoped: off,
                ..ScopeBlendArgs::default()
            },
            melee_charge: LungeGunTimings { strike_wait_millis: 0, lunge_windup_wait_millis: 0 },
            knife_reach: crate::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
            last_tick_buttons: 0,
            old_move: [0, 0],
            gun_forbids_lying: off,
        }
    }

    /// A player standing on the floor at time 1000.
    fn player() -> AvatarState {
        let mut avatar = AvatarState::ZERO;
        avatar.gravity = 800_i32;
        avatar.speed = 190;
        avatar.run_speed_modifier = 1.0;
        avatar.standing_on = 1022;
        (avatar.eye_height_goal, avatar.eye_height_now) = (60, 60.0);
        avatar.last_cmd_ms = 1000;
        avatar
    }

    /// Runs one 50 ms frame with these keys; returns the command as the
    /// frame left it.
    fn frame(avatar: &mut AvatarState, ahead: i8, side: i8) -> InputCommand {
        let mut ucmd = InputCommand {
            authority_clock_ms: avatar.last_cmd_ms + 50,
            thrust_ahead: ahead,
            thrust_lateral: side,
            ..InputCommand::default()
        };
        pmove(
            avatar,
            &mut ucmd,
            frame_ctx(),
            &Floor,
            &FixedClimbClipDuration::default(),
            &StillClimbOffset,
        );
        ucmd
    }

    /// Frames run in the longer tests.
    const FRAMES: usize = 20;

    #[test]
    fn holdrand_uses_whole_degrees_of_yaw() {
        assert_eq!(holdrand(0.0, 0), 159_719_620);
        assert_eq!(holdrand(90.7, 1000), 1_117_329_830);
        assert_eq!(holdrand(90.2, 1000), holdrand(90.9, 1000));
        // Truncated toward zero, as BO2's float-to-int does.
        assert_eq!(holdrand(-0.5, 7), holdrand(0.0, 7));
    }

    #[test]
    fn walking_forward_reaches_the_walk_speed_on_the_floor() {
        let mut avatar = player();
        for _ in 0..FRAMES {
            frame(&mut avatar, 127, 0);
        }
        assert!(avatar.origin[0] > 100.0, "{:?}", avatar.origin);
        assert!(avatar.origin[2].abs() < 0.5, "{:?}", avatar.origin);
        assert_ne!(avatar.standing_on, NO_ENTITY_SLOT);
        assert!((avatar.velocity[0] - 190.0).abs() < 2.0, "{:?}", avatar.velocity);
    }

    #[test]
    fn a_late_command_catches_up_only_150_ms() {
        let mut avatar = player();
        for _ in 0..FRAMES {
            frame(&mut avatar, 127, 0);
        }
        let speed = avatar.velocity[0];
        let from = avatar.origin[0];
        let mut ucmd = InputCommand {
            authority_clock_ms: avatar.last_cmd_ms + 500,
            thrust_ahead: 127,
            ..InputCommand::default()
        };
        let late = ucmd.authority_clock_ms;
        pmove(
            &mut avatar,
            &mut ucmd,
            frame_ctx(),
            &Floor,
            &FixedClimbClipDuration::default(),
            &StillClimbOffset,
        );
        assert_eq!((avatar.last_cmd_ms, ucmd.authority_clock_ms), (late, late));
        let moved = avatar.origin[0] - from;
        assert!((moved - speed * 0.150).abs() < 1.0, "{moved} at {speed}");
    }

    #[test]
    fn a_knife_charge_runs_straight_ahead() {
        let mut avatar = player();
        avatar.movement_state_bits = movement_state_bits::MELEE_CHARGE;
        let ucmd = frame(&mut avatar, 0, -50);
        assert_eq!((ucmd.thrust_ahead, ucmd.thrust_lateral), (127, 0));
    }

    #[test]
    fn the_dead_do_not_walk() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_KIND_DEAD;
        let ucmd = frame(&mut avatar, 127, 127);
        assert_eq!((ucmd.thrust_ahead, ucmd.thrust_lateral), (0, 0));
        assert!(avatar.velocity[0].abs() < 1e-3, "{:?}", avatar.velocity);
    }

    #[test]
    fn a_dead_body_slides_to_a_stop() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_KIND_DEAD;
        avatar.velocity = [100.0, 0.0, 0.0];
        frame(&mut avatar, 0, 0);
        // Dead drag 100 -> 80. BO2's ladder check then takes the dead off
        // the ground, so the frame runs the air move: no ground grip.
        assert_eq!(avatar.velocity, [80.0, 0.0, 0.0]);
        frame(&mut avatar, 0, 0);
        // The frame's ground trace put the body back down, so the drag
        // runs again next frame.
        assert_eq!(avatar.velocity, [60.0, 0.0, 0.0]);
    }

    #[test]
    fn a_linked_player_is_held_still_off_any_ladder() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_WALK_ATTACHED;
        avatar.movement_state_bits = movement_state_bits::LADDER;
        avatar.velocity = [100.0, 0.0, 50.0];
        frame(&mut avatar, 127, 0);
        assert_eq!(avatar.velocity, [0.0; 3]);
        assert_eq!(avatar.origin, [0.0; 3]);
        assert_eq!(avatar.standing_on, NO_ENTITY_SLOT);
        assert_eq!(avatar.movement_state_bits & (movement_state_bits::LADDER | movement_state_bits::LADDER_FALL), movement_state_bits::LADDER_FALL);
    }

    #[test]
    fn a_turret_holds_the_player_still() {
        let mut avatar = player();
        avatar.e_flags = playerstate_t6::game::entity_flag_bits::TURRET_STAND;
        avatar.velocity = [100.0, 0.0, 0.0];
        frame(&mut avatar, 127, 0);
        assert_eq!(avatar.velocity, [0.0; 3]);
        assert_eq!(avatar.origin, [0.0; 3]);
    }

    #[test]
    fn going_down_leaves_the_turret() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_DOWNED;
        avatar.e_flags = playerstate_t6::game::entity_flag_bits::TURRET_STAND;
        frame(&mut avatar, 0, 0);
        assert_eq!(avatar.e_flags & TURRET_MASK, 0);
    }

    #[test]
    fn the_end_of_match_screen_moves_nobody() {
        let mut avatar = player();
        avatar.movement_mode = MOVE_MODE_INTERLUDE;
        avatar.velocity = [100.0, 0.0, 0.0];
        frame(&mut avatar, 127, 0);
        assert_eq!(avatar.origin, [0.0; 3]);
    }

    #[test]
    fn the_rules_are_always_bo2() {
        let frame_ctx = bo2_context(frame_ctx());
        assert_eq!(frame_ctx.walk.cmd_scale.prone_lerp_ms, PRONE_LERP_MS);
    }

    /// Runs one 50 ms frame holding these keys.
    fn hold(avatar: &mut AvatarState, keys: u32) -> MotionTickOutcome {
        let mut ucmd = InputCommand {
            authority_clock_ms: avatar.last_cmd_ms + 50,
            buttons: keys,
            ..InputCommand::default()
        };
        pmove(
            avatar,
            &mut ucmd,
            frame_ctx(),
            &Floor,
            &FixedClimbClipDuration::default(),
            &StillClimbOffset,
        )
    }

    #[test]
    fn lying_down_and_getting_up_go_by_crouch() {
        let mut avatar = player();
        let mut locked = false;
        let mut last = hold(&mut avatar, buttons::LYING_FLAT);
        // Down to crouch height first (200 ms), then prone (600 ms).
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
        assert_eq!(last.bounds.hi_corner[2], 50.0);
        for _ in 0..FRAMES {
            let was = avatar.eye_height_goal;
            last = hold(&mut avatar, buttons::LYING_FLAT);
            if was != eye_level::LYING_FLAT && avatar.eye_height_goal == eye_level::LYING_FLAT {
                // Lying down holds the stance for 1.8 s.
                locked = avatar.movement_state_bits & movement_state_bits::JUMPING != 0 && avatar.move_timer > 1700;
            }
        }
        assert!(locked);
        assert_eq!(avatar.eye_height_now, 11.0);
        assert_eq!(avatar.movement_state_bits & movement_state_bits::LYING_FLAT, movement_state_bits::LYING_FLAT);
        assert_eq!(last.bounds.hi_corner[2], 30.0);

        // Letting go: the get-up animation once, then up by way of crouch.
        let first = hold(&mut avatar, 0);
        assert_eq!(first.stance_event, Some(16));
        assert!(first.reset_torso);
        assert_eq!(avatar.eye_height_goal, eye_level::DUCK_BUTTON_BIT);
        for _ in 0..FRAMES {
            last = hold(&mut avatar, 0);
            assert_eq!(last.stance_event, None);
        }
        assert_eq!(avatar.eye_height_now, 60.0);
        assert_eq!(avatar.movement_state_bits & (movement_state_bits::LYING_FLAT | movement_state_bits::DUCK_BUTTON_BIT), 0);
        assert_eq!(last.bounds.hi_corner[2], 70.0);
    }
}
