//! Black Ops II's PmoveSingle: one frame of a player's movement in BO2's own
//! order. The keys are settled first (a knife charge runs forward, the dead
//! and downed do not walk, lying down to aim holds still), then the view,
//! aim, sprint and stance. Then the player's state picks the move: flying,
//! the end-of-match screen, linked to something, on a turret, climbing a
//! ledge, or the ordinary dive, ladder, knife charge, walk or air move.

use crate::bridge::{
    AdsIntentContext, CheckLadderContext, CollisionBackend, GroundTraceInput, LadderAttachBackend,
    LadderTraceHit, MantleCapsuleTrace,
    MantleRootDelta,
    MantleXAnimLength, MoveBounds, PRONE_LERP_MS, Pml, PmoveResult, PmoveSingleContext,
    SprintResult, StanceChange, calc_melee_charge_time,
    drop_timers, mantle, melee_charge_move, ufo, update_ads_frac,
    update_ads_intent,
};
use playerstate_t6::game::{
    ENTITYNUM_NONE, PM_TYPE_DEAD, PM_TYPE_DEAD_LINKED, PM_TYPE_INTERMISSION, PM_TYPE_LAST_STAND,
    PM_TYPE_NORMAL_LINKED, PlayerState, UserCmd, pm_flags,
};

use crate::dive;
use crate::feel::EF_TURRET;
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
    fn add(&mut self, other: BodyAnims) {
        self.jump |= other.jump;
        self.land |= other.land;
    }
}

/// Black Ops II's Pmove: a late command catches up at most 150 ms, in moves
/// of at most 66 ms. It takes the player, command and context and gives
/// back the result (the last move's). It moves as the server does.
pub fn pmove<C: Bo2Collision, L: MantleXAnimLength, R: MantleRootDelta>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    context: PmoveSingleContext,
    collision: &C,
    lengths: &L,
    root: &R,
) -> PmoveResult {
    pmove_with_body(ps, cmd, context, collision, lengths, root, true).0
}

/// `pmove`, also giving the legs' jump and land animations any of its moves
/// started, and the slope pitch they averaged. `server` is BO2's server move
/// (not a client's prediction): only it sends the slide sound events.
pub fn pmove_with_body<C: Bo2Collision, L: MantleXAnimLength, R: MantleRootDelta>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    mut context: PmoveSingleContext,
    collision: &C,
    lengths: &L,
    root: &R,
    server: bool,
) -> (PmoveResult, BodyAnims) {
    let mut body = BodyAnims::default();
    // BO2 clears the pitch history with the rest of pmove_t for each command.
    let mut pitch = crate::slope::PitchHistory::default();
    let server_time = cmd.server_time;
    // The platform the player rides carries them first, over the whole
    // command (before the catch-up below).
    crate::mover::pre_step(ps, cmd, collision);
    if server_time.wrapping_sub(ps.command_time) > CATCH_UP_MSEC {
        ps.command_time = server_time.wrapping_sub(CATCH_UP_MSEC);
    }
    loop {
        let step = server_time.wrapping_sub(ps.command_time).min(STEP_MSEC);
        cmd.server_time = ps.command_time.wrapping_add(step);
        let result = pmove_single(ps, cmd, context, collision, lengths, root, &mut body, &mut pitch, server);
        if cmd.server_time == server_time {
            body.average_pitch = pitch.average();
            return (result, body);
        }
        // The next move's last command is this one.
        context.old_buttons = cmd.buttons;
        context.walk.old_buttons = cmd.buttons;
        context.old_move = [cmd.forwardmove, cmd.rightmove];
    }
}

/// One move of Black Ops II movement (PmoveSingle).
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn pmove_single<C: Bo2Collision, L: MantleXAnimLength, R: MantleRootDelta>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    context: PmoveSingleContext,
    collision: &C,
    lengths: &L,
    root: &R,
    body: &mut BodyAnims,
    pitch: &mut crate::slope::PitchHistory,
    server: bool,
) -> PmoveResult {
    let mut context = bo2_context(context);
    let feel = context.walk.feel;
    // The crouch and stand-up checks (BO2's is_not_penetrating).
    let penetration = PenetrationChecks(collision);

    // A knife charge runs straight ahead whatever the keys say.
    if ps.pm_flags & pm_flags::MELEE_CHARGE != 0 {
        cmd.forwardmove = 127;
        cmd.rightmove = 0;
    }
    // A dead body passes through players unless its link says otherwise.
    if ps.pm_type >= PM_TYPE_DEAD && ps.link_flags & LINK_KEEPS_BODY_CONTENTS == 0 {
        context.bounds.tracemask &= !crate::stance::BO2_GETUP_IGNORES;
    }

    let msec = cmd.server_time.wrapping_sub(ps.command_time).clamp(MIN_MSEC, MAX_MSEC);
    // The water level is worked out again after the view; until then it is
    // dry (BO2 reads the lying-down aim before it).
    ps.waterlevel = 0;
    // Lying down and aiming holds still (decided before anything moves).
    crate::feel::prone_aim(
        ps,
        cmd,
        context.old_move,
        crate::stance::sniper_scope_raised(ps, context.ads_intent),
    );
    // The dead do not walk.
    if ps.pm_type >= PM_TYPE_DEAD {
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
    }
    ps.command_time = cmd.server_time;

    let mut pml = Pml {
        forward: [0.0; 3],
        right: [0.0; 3],
        up: [0.0; 3],
        frametime: (msec as f32) * 0.001_f32,
        msec,
        walking: 0,
        ground_plane: 0,
        almost_ground_plane: 0,
        ground_trace: [0; 11],
        previous_origin: ps.origin,
        previous_velocity: ps.velocity,
        holdrand: holdrand(ps.viewangles[1], cmd.server_time),
    };

    let old_yaw = ps.viewangles[1];
    crate::view::update_view_angles(ps, cmd, context.view_angles);
    // Lying down, the view is held to the body's turn.
    crate::prone_turn::bo2_prone_view(ps, cmd, msec, collision, feel.zombies, old_yaw);
    crate::water::set_water_level(ps, collision);
    // A dive into water is over once it stops rising (BO2's Dtp_CanMove,
    // dtp_new_trajectory on).
    if dive::active(ps) && ps.waterlevel > 0 && ps.velocity[2] <= 0.0 {
        dive::interrupt(ps, cmd.server_time);
    }
    let (forward, right, up) = game_math::angle_vectors(ps.viewangles);
    pml.forward = forward;
    pml.right = right;
    pml.up = up;

    // Noclip and UFO (the mod menu's fly mode): no ground, no ladder, no
    // stance, mantle or dive.
    if ufo::active(ps) {
        ps.ground_entity_num = ENTITYNUM_NONE;
        ladder_to_fall(ps);
        drop_timers(ps, &pml);
        ufo::fly(ps, cmd, &pml);
        return result(pml, context.bounds, None, false, 0.0);
    }

    mantle::clear_hint(ps);

    // A dive to prone starts from a sprint, so it is decided before the
    // sprint and stance updates see the prone press; while it lasts it owns
    // sprint, stance and movement. After its slide the player lies still a
    // moment, and a slide that has stopped is over before the stance update.
    if feel.dive {
        if dive::pausing(ps, cmd.server_time) {
            cmd.forwardmove = 0;
            cmd.rightmove = 0;
        }
        dive::end_stopped_slide(ps, cmd.server_time);
    }
    let dive_stance = dive::owns_stance(ps, cmd.server_time);
    let dive_start =
        feel.dive && !context.weapon_blocks_prone && dive::wants(ps, cmd, context.old_buttons);
    let diving = dive_start || dive::active(ps);

    // The gun can aim through a dive (asked for, 10-09).
    let ads_cmd = dive::ads_cmd(ps, cmd, diving);
    let _ads = update_ads_intent(ps, &ads_cmd, context.old_buttons, context.ads_intent);
    let mut sprint = context.sprint;
    sprint.weapon_max_sprint_time =
        crate::feel::stamin_up_sprint_time(ps, sprint.weapon_max_sprint_time, feel);
    if ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH) != 0 {
        sprint.stand_up_clear = !penetration
            .trace(GroundTraceInput {
                start: ps.origin,
                end: ps.origin,
                mins: context.bounds.mins,
                maxs: context.bounds.maxs,
                tracemask: context.bounds.tracemask & !0x0200_0000,
            })
            .allsolid;
    }
    let sprint_change = if diving {
        SprintResult::Unchanged
    } else {
        // Sprint in any direction (feel.omni); a held prone key does not end
        // the sprint while getting up from a dive.
        let sprint_cmd = crate::feel::omni_sprint_cmd(&dive::stance_cmd(ps, cmd), feel);
        crate::sprint::update(ps, &sprint_cmd, sprint, context.weapon_blocks_prone)
    };

    // The end-of-match screen: aim and sprint settle, nothing moves.
    if ps.pm_type == PM_TYPE_INTERMISSION {
        ladder_to_fall(ps);
        update_ads_frac(ps, pml.msec, context.ads_frac);
        return result(pml, context.bounds, None, false, 0.0);
    }

    crate::feel::update_walking_flag(ps, cmd);

    let duck = duck_pass(ps, cmd, &ads_cmd, collision, &context, dive_stance, diving, pml.frametime);
    let mut stance_event = duck.anim;
    let mut reset_torso =
        duck.change != StanceChange::Unchanged || sprint_change == SprintResult::Started;
    let mut bounds = duck.bounds;
    update_ads_frac(ps, pml.msec, context.ads_frac);

    // Linked to something (a vehicle, a script's link, a linked body): the
    // link owns the origin, so the player is held still.
    if ps.pm_type == PM_TYPE_NORMAL_LINKED || ps.pm_type == PM_TYPE_DEAD_LINKED {
        hold_still(ps, &mut pml);
        drop_timers(ps, &pml);
        footsteps(ps, &pml, cmd, &context, pitch);
        // A dive is cut short by a link.
        if ps.pm_type == PM_TYPE_NORMAL_LINKED && dive::active(ps) {
            dive::interrupt(ps, cmd.server_time);
        }
        return result(pml, bounds, stance_event, reset_torso, 0.0);
    }

    // Going down leaves any ladder and any turret.
    if ps.pm_type == PM_TYPE_LAST_STAND {
        ladder_to_fall(ps);
        ps.e_flags &= !EF_TURRET;
    }

    // On a turret the gun does the moving.
    if ps.e_flags & EF_TURRET != 0 {
        hold_still(ps, &mut pml);
        drop_timers(ps, &pml);
        footsteps(ps, &pml, cmd, &context, pitch);
        return result(pml, bounds, stance_event, reset_torso, 0.0);
    }

    let mut climb_started = false;
    if ps.pm_flags & pm_flags::MANTLE == 0 {
        // A climb's own path owns the ledge; the ground's solid fix-up during
        // it would push the player back off.
        body.add(crate::ground::ground_trace(ps, &mut pml, bounds, collision, feel, cmd.server_time, server));
        let mut tracer = CollisionMantleTrace { collision };
        let check = crate::mantle::CheckContext {
            buttons: cmd.buttons,
            facing_xy: [pml.forward[0], pml.forward[1]],
            tracemask: bounds.tracemask,
            mp: !feel.zombies,
        };
        climb_started =
            !diving && crate::mantle::check(ps, &check, &mut tracer, lengths, root);
        if climb_started {
            mantle::mark_bo2_weapon(ps, feel, lengths);
        }
    }

    if ps.pm_flags & pm_flags::MANTLE != 0 {
        ladder_to_fall(ps);
        ps.ground_entity_num = ENTITYNUM_NONE;
        pml.walking = 0;
        pml.ground_plane = 0;
        if climb_started {
            // BO2 runs the aim, sprint, walking and stance updates again on
            // the frame a climb starts, now off the ground: the aim lets go
            // (no aiming while climbing) and the view's height steps again.
            update_ads_intent(
                ps,
                &ads_cmd,
                context.old_buttons,
                AdsIntentContext { ads_allowed: false, ..context.ads_intent },
            );
            let sprint_cmd = crate::feel::omni_sprint_cmd(&dive::stance_cmd(ps, cmd), feel);
            crate::sprint::update(ps, &sprint_cmd, sprint, context.weapon_blocks_prone);
            crate::feel::update_walking_flag(ps, cmd);
            let again =
                duck_pass(ps, cmd, &ads_cmd, collision, &context, dive_stance, diving, pml.frametime);
            bounds = again.bounds;
            stance_event = again.anim.or(stance_event);
            reset_torso |= again.change != StanceChange::Unchanged;
        }
        body.jump |= crate::mantle::advance(ps, pml.msec, cmd.server_time, !feel.zombies, lengths, root);
        return result(pml, bounds, stance_event, reset_torso, 0.0);
    }

    // The body's slope pitches follow this frame's ground.
    crate::prone_turn::bo2_prone_pitch(ps, &pml, collision, feel.zombies);
    drop_timers(ps, &pml);
    if ps.pm_type >= PM_TYPE_LAST_STAND {
        crate::dead::dead_move(ps, &pml);
    }

    if !diving {
        let mut ladder = CollisionLadderBackend {
            collision,
            bounds,
            last_entity: None,
        };
        let check = CheckLadderContext {
            server_time: cmd.server_time,
            walking: pml.walking != 0,
            forward_xy: [pml.forward[0], pml.forward[1]],
            forwardmove: cmd.forwardmove,
        };
        body.jump |= crate::ladder::check_ladder_move(ps, &mut pml, check, &mut ladder);
    }

    let view_change = if diving {
        if dive_start {
            dive::start(ps, &mut pml, cmd, context.walk.jump.jump_height);
        }
        dive::advance(ps, &pml, cmd, bounds, collision, context.walk.jump.jump_height, context.air)
    } else if ps.pm_flags & pm_flags::LADDER != 0 {
        crate::ladder::ladder_move(ps, &mut pml, cmd, context.walk, bounds, collision, body)
    } else if let Some(lunge) = context.walk.lunge {
        // bo2mp's knife lunge (BO2's COD4-style charge) launches here, then
        // the walk or air move runs it with the lunge's own friction.
        crate::bridge::bo2_lunge_update(ps, &pml, lunge);
        if ps.waterlevel >= crate::water::SWIM_LEVEL {
            crate::water::swim_move(ps, &mut pml, cmd, context.air, bounds, collision, feel.zombies, Some(lunge))
        } else if pml.walking == 0 {
            crate::air::air_move(ps, &pml, cmd, context.air, bounds, collision, feel.zombies, Some(lunge))
        } else {
            crate::walk::walk_move(ps, &mut pml, cmd, context.walk, bounds, collision, body)
        }
    } else {
        calc_melee_charge_time(ps, context.melee_charge, context.player_melee_range);
        if ps.pm_flags & pm_flags::MELEE_CHARGE != 0 {
            melee_charge_move(
                ps,
                &pml,
                bounds.mins,
                bounds.maxs,
                bounds.tracemask,
                collision,
                feel.zombies,
            )
        } else if ps.waterlevel >= crate::water::SWIM_LEVEL {
            crate::water::swim_move(ps, &mut pml, cmd, context.air, bounds, collision, feel.zombies, None)
        } else if pml.walking == 0 {
            crate::air::air_move(ps, &pml, cmd, context.air, bounds, collision, feel.zombies, None)
        } else {
            crate::walk::walk_move(ps, &mut pml, cmd, context.walk, bounds, collision, body)
        }
    };

    body.add(crate::ground::ground_trace(ps, &mut pml, bounds, collision, feel, cmd.server_time, server));
    footsteps(ps, &pml, cmd, &context, pitch);
    // BO2 runs the gun between the two; ours runs after the move.
    crate::foliage::foliage_sounds(ps, cmd.server_time, bounds, collision);
    crate::snap::end_tick_velocity(ps, &pml);
    result(pml, bounds, stance_event, reset_torso, view_change)
}

/// BO2's random seed for the frame: the view's yaw in whole degrees plus the
/// frame's time, stepped four times.
pub fn holdrand(view_yaw: f32, server_time: i32) -> i32 {
    let mut value = (view_yaw as i32).wrapping_add(server_time) as u32;
    for _ in 0..4 {
        value = value.wrapping_mul(HOLDRAND_MUL).wrapping_add(HOLDRAND_ADD);
    }
    value as i32
}

/// This crate is always Black Ops II, whatever rules it is handed.
fn bo2_context(mut context: PmoveSingleContext) -> PmoveSingleContext {
    if context.walk.cmd_scale.prone_lerp_ms == 0 {
        context.walk.cmd_scale.prone_lerp_ms = PRONE_LERP_MS;
    }
    context
}

/// Leaving a ladder any way but the ground lets the player fall off it.
fn ladder_to_fall(ps: &mut PlayerState) {
    if ps.pm_flags & pm_flags::LADDER != 0 {
        ps.pm_flags = (ps.pm_flags & !pm_flags::LADDER) | pm_flags::LADDER_FALL;
    }
}

/// Linked or on a turret: off any ladder, off the ground, not moving.
fn hold_still(ps: &mut PlayerState, pml: &mut Pml) {
    ladder_to_fall(ps);
    ps.ground_entity_num = ENTITYNUM_NONE;
    pml.walking = 0;
    pml.ground_plane = 0;
    pml.almost_ground_plane = 0;
    ps.velocity = [0.0; 3];
}

/// PM_Footsteps (footstep.rs): the walk rhythm and its footstep.
fn footsteps(
    ps: &mut PlayerState,
    pml: &Pml,
    cmd: &UserCmd,
    context: &PmoveSingleContext,
    pitch: &mut crate::slope::PitchHistory,
) {
    crate::footstep::footsteps(ps, pml, cmd, context.walk.cmd_scale, context.walk.feel, pitch);
}

/// What one stance pass leaves.
struct DuckPass {
    bounds: MoveBounds,
    anim: Option<u8>,
    change: StanceChange,
}

/// BO2's PM_CheckDuck: the stance keys, the 1.8 s prone lock, the view's
/// height and the box, and lying down drops the aim. It runs again on the
/// frame a climb starts.
#[allow(clippy::too_many_arguments)]
fn duck_pass<C: Bo2Collision>(
    ps: &mut PlayerState,
    cmd: &mut UserCmd,
    ads_cmd: &UserCmd,
    collision: &C,
    context: &PmoveSingleContext,
    dive_stance: bool,
    diving: bool,
    frametime: f32,
) -> DuckPass {
    let feel = context.walk.feel;
    let previous_stance = ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH);
    // From the frame after a dive's take-off to the end of its still moment
    // the view drops to prone; a get-up pressed meanwhile is kept.
    let dive_prone = if dive_stance {
        dive::queue_getup(ps, cmd, context.old_buttons);
        dive::force_prone(ps);
        true
    } else {
        !diving && dive::holds_prone(ps, cmd, context.old_buttons)
    };
    // A held prone key does not drop the player back down while they get up
    // from a dive. Buttons the stance update clears stay cleared in the real
    // command.
    let mut stance_cmd = dive::stance_cmd(ps, cmd);
    let held_back = cmd.buttons & !stance_cmd.buttons;
    let duck = crate::stance::check_duck(
        ps,
        &mut stance_cmd,
        collision,
        DuckContext {
            bounds: context.bounds,
            weapon_blocks_prone: context.weapon_blocks_prone,
            zombies: feel.zombies,
            dive_owns: diving || dive_prone,
        },
    );
    cmd.buttons &= stance_cmd.buttons | held_back;
    let stance_event = duck.anim;
    let stance_change = duck.change;
    // Dropping to prone locks the stance for 1.8 s (a dive's landing and last
    // stand do not).
    if stance_change == StanceChange::EnteredProne
        && ps.pm_type != PM_TYPE_LAST_STAND
        && ps.pm_time == 0
        && !dive::any(ps)
    {
        ps.pm_flags |= pm_flags::JUMPING;
        ps.pm_time = 1800;
    }
    dive::finish_getup(ps, cmd);
    let mut bounds =
        crate::stance::finish_duck(ps, duck.tail, context.bounds, cmd.server_time, frametime);
    if dive::active(ps) {
        // Through a dive the body shrinks as the view drops.
        bounds.maxs[2] = dive::hull_height(ps);
    }
    // A dive's push does not drop the aim on lying down.
    crate::stance::bo2_lie_down(
        ps,
        ads_cmd,
        collision,
        bounds,
        previous_stance & pm_flags::PRONE != 0,
    );
    DuckPass { bounds, anim: stance_event, change: stance_change }
}

fn result(
    pml: Pml,
    bounds: MoveBounds,
    stance_event: Option<u8>,
    reset_torso: bool,
    view_change: f32,
) -> PmoveResult {
    PmoveResult {
        pml,
        bounds,
        stance_event,
        reset_torso,
        view_change,
    }
}

/// The mantle's capsule sweeps run through the world's collision.
struct CollisionMantleTrace<'a, C: CollisionBackend> {
    collision: &'a C,
}

impl<C: CollisionBackend> MantleCapsuleTrace for CollisionMantleTrace<'_, C> {
    fn trace(
        &mut self,
        start: [f32; 3],
        end: [f32; 3],
        mins: [f32; 3],
        maxs: [f32; 3],
        contentmask: u32,
    ) -> trace_t6::Trace {
        self.collision.trace(GroundTraceInput {
            start,
            end,
            mins,
            maxs,
            tracemask: contentmask,
        })
    }
}

/// The ladder probe: a slimmer box from knee height, swept toward the
/// ladder. It remembers which entity it last hit, for a ladder on a moving
/// thing.
struct CollisionLadderBackend<'a, C: CollisionBackend> {
    collision: &'a C,
    bounds: MoveBounds,
    last_entity: Option<i32>,
}

/// The ladder probe is this much slimmer on each side, and starts this high.
const LADDER_PROBE_INSET: f32 = 6.0;
const LADDER_PROBE_FLOOR: f32 = 8.0;

impl<C: CollisionBackend> LadderAttachBackend for CollisionLadderBackend<'_, C> {
    fn ladder_trace(&mut self, origin: [f32; 3], dir: [f32; 3], dist: f32) -> Option<LadderTraceHit> {
        let end = [
            origin[0] + dir[0] * dist,
            origin[1] + dir[1] * dist,
            origin[2] + dir[2] * dist,
        ];
        let mut mins = self.bounds.mins;
        let mut maxs = self.bounds.maxs;
        mins[0] += LADDER_PROBE_INSET;
        mins[1] += LADDER_PROBE_INSET;
        mins[2] = LADDER_PROBE_FLOOR;
        maxs[0] -= LADDER_PROBE_INSET;
        maxs[1] -= LADDER_PROBE_INSET;
        if maxs[2] < LADDER_PROBE_FLOOR {
            maxs[2] = mins[2];
        }
        let hit = self.collision.trace(GroundTraceInput {
            start: origin,
            end,
            mins,
            maxs,
            tracemask: self.bounds.tracemask,
        });
        self.last_entity = (hit.hit_type == trace_t6::TraceHitType::Entity)
            .then(|| i32::from(trace_t6::trace_get_entity_hit_id(&hit)));
        if hit.fraction >= 1.0 {
            return None;
        }
        Some(LadderTraceHit {
            fraction: hit.fraction,
            normal: hit.normal,
            surface_flags: hit.sflags,
        })
    }
}

impl<C: CollisionBackend> crate::ladder::LadderHitEntity for CollisionLadderBackend<'_, C> {
    fn last_hit_entity(&self) -> Option<i32> {
        self.last_entity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{
        AdsFracContext, AdsIntentContext, AirMoveContext, Bo2Feel, CmdScaleWalkContext,
        FlatMantleAnimLength, JumpLaunchContext, MeleeChargeWeaponDelays, SprintContext,
        ViewAngleClamp, WalkMoveContext, ZeroMantleRootDelta, view_height,
    };
    use playerstate_t6::game::buttons;
    use trace_t6::Trace;

    /// A floor at z = 0 and nothing else.
    struct Floor;

    impl Bo2Collision for Floor {}

    impl CollisionBackend for Floor {
        fn trace(&self, input: GroundTraceInput) -> Trace {
            let start = input.start[2] + input.mins[2];
            let end = input.end[2] + input.mins[2];
            let mut hit = Trace {
                fraction: 1.0,
                ..Trace::default()
            };
            if start < 0.0 {
                hit.startsolid = true;
                hit.allsolid = end < 0.0;
                hit.fraction = 0.0;
                hit.normal = [0.0, 0.0, 1.0];
                return hit;
            }
            if end < 0.0 {
                let fraction = start / (start - end);
                hit.fraction = fraction;
                hit.normal = [0.0, 0.0, 1.0];
                hit.walkable = true;
                hit.hit_type = trace_t6::TraceHitType::Entity;
                hit.hit_id = trace_t6::ENTITYNUM_WORLD;
            }
            hit
        }
    }

    /// The sim's move settings, handed MW2's rules (the frame turns BO2's on).
    fn context() -> PmoveSingleContext {
        let feel = Bo2Feel::BO2_MP;
        let air = AirMoveContext {
            player_spectate_speed_scale: 1.0,
            shellshock_gravity_scale: 1.0,
            shellshock_gravity_bias: 0.0,
        };
        PmoveSingleContext {
            walk: WalkMoveContext {
                cmd_scale: CmdScaleWalkContext {
                    player_back_speed_scale: 0.7,
                    player_strafe_speed_scale: 0.8,
                    player_sprint_speed_scale: 1.5,
                    player_last_stand_crawl_speed_scale: 0.15,
                    weapon_move_speed_scale: 1.0,
                    weapon_ads_move_speed_scale: 1.0,
                    shellshock_movement_scale: 1.0,
                    prone_lerp_ms: 0,
                },
                weapon_move_scale: 1.0,
                old_buttons: 0,
                jump: JumpLaunchContext {
                    jump_height: 39.0,
                    dive: false,
                    crouch_jump_scale: 1.0,
                    jump_ladder_push_vel: 128.0,
                },
                air,
                feel,
                lunge: None,
            },
            air,
            bounds: MoveBounds {
                mins: [-15.0, -15.0, 0.0],
                maxs: [15.0, 15.0, 70.0],
                tracemask: 0x0281_0011,
            },
            view_angles: ViewAngleClamp {
                pitch_up: 85.0,
                pitch_down: 85.0,
                unclamped_pitch_bit: false,
            },
            sprint: SprintContext {
                weapon_max_sprint_time: 4000,
                sprint_forever: false,
                min_sprint_time_seconds: 1.0,
                sprint_delay_seconds: 0.0,
                sprint_forward_minimum: 105,
                stand_up_clear: true,
                sprint_recharge_pause_seconds: 0.0,
            },
            ads_intent: AdsIntentContext {
                ads_allowed: true,
                weapon_def_scope: false,
                sprint_hold_ads: false,
            },
            ads_frac: AdsFracContext {
                aim_down_sight: false,
                ads_in_rate: 1.0 / 200.0,
                ads_out_rate: 1.0 / 200.0,
                rechamber_while_ads: true,
                ads_fire_only: false,
                ..AdsFracContext::default()
            },
            melee_charge: MeleeChargeWeaponDelays {
                melee_delay_ms: 0,
                melee_charge_delay_ms: 0,
            },
            player_melee_range: crate::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
            old_buttons: 0,
            old_move: [0, 0],
            weapon_blocks_prone: false,
        }
    }

    /// A player standing on the floor at time 1000.
    fn player() -> PlayerState {
        let mut ps = PlayerState::ZERO;
        ps.gravity = 800;
        ps.speed = 190;
        ps.move_speed_scale_multiplier = 1.0;
        ps.ground_entity_num = 1022;
        ps.view_height_target = 60;
        ps.view_height_current = 60.0;
        ps.command_time = 1000;
        ps
    }

    /// Runs one 50 ms frame with these keys; returns the command as the
    /// frame left it.
    fn frame(ps: &mut PlayerState, forward: i8, right: i8) -> UserCmd {
        let mut cmd = UserCmd {
            server_time: ps.command_time + 50,
            forwardmove: forward,
            rightmove: right,
            ..UserCmd::default()
        };
        pmove(
            ps,
            &mut cmd,
            context(),
            &Floor,
            &FlatMantleAnimLength::default(),
            &ZeroMantleRootDelta,
        );
        cmd
    }

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
        let mut ps = player();
        for _ in 0..20 {
            frame(&mut ps, 127, 0);
        }
        assert!(ps.origin[0] > 100.0, "{:?}", ps.origin);
        assert!(ps.origin[2].abs() < 0.5, "{:?}", ps.origin);
        assert_ne!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert!((ps.velocity[0] - 190.0).abs() < 2.0, "{:?}", ps.velocity);
    }

    #[test]
    fn a_late_command_catches_up_only_150_ms() {
        let mut ps = player();
        for _ in 0..20 {
            frame(&mut ps, 127, 0);
        }
        let speed = ps.velocity[0];
        let from = ps.origin[0];
        let mut cmd = UserCmd {
            server_time: ps.command_time + 500,
            forwardmove: 127,
            ..UserCmd::default()
        };
        let late = cmd.server_time;
        pmove(
            &mut ps,
            &mut cmd,
            context(),
            &Floor,
            &FlatMantleAnimLength::default(),
            &ZeroMantleRootDelta,
        );
        assert_eq!((ps.command_time, cmd.server_time), (late, late));
        let moved = ps.origin[0] - from;
        assert!((moved - speed * 0.150).abs() < 1.0, "{moved} at {speed}");
    }

    #[test]
    fn a_knife_charge_runs_straight_ahead() {
        let mut ps = player();
        ps.pm_flags = pm_flags::MELEE_CHARGE;
        let cmd = frame(&mut ps, 0, -50);
        assert_eq!((cmd.forwardmove, cmd.rightmove), (127, 0));
    }

    #[test]
    fn the_dead_do_not_walk() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_DEAD;
        let cmd = frame(&mut ps, 127, 127);
        assert_eq!((cmd.forwardmove, cmd.rightmove), (0, 0));
        assert!(ps.velocity[0].abs() < 1e-3, "{:?}", ps.velocity);
    }

    #[test]
    fn a_dead_body_slides_to_a_stop() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_DEAD;
        ps.velocity = [100.0, 0.0, 0.0];
        frame(&mut ps, 0, 0);
        // Dead drag 100 -> 80. BO2's ladder check then takes the dead off
        // the ground, so the frame runs the air move: no ground grip.
        assert_eq!(ps.velocity, [80.0, 0.0, 0.0]);
        frame(&mut ps, 0, 0);
        // The frame's ground trace put the body back down, so the drag
        // runs again next frame.
        assert_eq!(ps.velocity, [60.0, 0.0, 0.0]);
    }

    #[test]
    fn a_linked_player_is_held_still_off_any_ladder() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_NORMAL_LINKED;
        ps.pm_flags = pm_flags::LADDER;
        ps.velocity = [100.0, 0.0, 50.0];
        frame(&mut ps, 127, 0);
        assert_eq!(ps.velocity, [0.0; 3]);
        assert_eq!(ps.origin, [0.0; 3]);
        assert_eq!(ps.ground_entity_num, ENTITYNUM_NONE);
        assert_eq!(ps.pm_flags & (pm_flags::LADDER | pm_flags::LADDER_FALL), pm_flags::LADDER_FALL);
    }

    #[test]
    fn a_turret_holds_the_player_still() {
        let mut ps = player();
        ps.e_flags = 0x400;
        ps.velocity = [100.0, 0.0, 0.0];
        frame(&mut ps, 127, 0);
        assert_eq!(ps.velocity, [0.0; 3]);
        assert_eq!(ps.origin, [0.0; 3]);
    }

    #[test]
    fn going_down_leaves_the_turret() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_LAST_STAND;
        ps.e_flags = 0x400;
        frame(&mut ps, 0, 0);
        assert_eq!(ps.e_flags & EF_TURRET, 0);
    }

    #[test]
    fn the_end_of_match_screen_moves_nobody() {
        let mut ps = player();
        ps.pm_type = PM_TYPE_INTERMISSION;
        ps.velocity = [100.0, 0.0, 0.0];
        frame(&mut ps, 127, 0);
        assert_eq!(ps.origin, [0.0; 3]);
    }

    #[test]
    fn the_rules_are_always_bo2() {
        let context = bo2_context(context());
        assert_eq!(context.walk.cmd_scale.prone_lerp_ms, PRONE_LERP_MS);
    }

    /// Runs one 50 ms frame holding these keys.
    fn hold(ps: &mut PlayerState, keys: u32) -> PmoveResult {
        let mut cmd = UserCmd {
            server_time: ps.command_time + 50,
            buttons: keys,
            ..UserCmd::default()
        };
        pmove(
            ps,
            &mut cmd,
            context(),
            &Floor,
            &FlatMantleAnimLength::default(),
            &ZeroMantleRootDelta,
        )
    }

    #[test]
    fn lying_down_and_getting_up_go_by_crouch() {
        let mut ps = player();
        let mut locked = false;
        let mut last = hold(&mut ps, buttons::PRONE);
        // Down to crouch height first (200 ms), then prone (600 ms).
        assert_eq!(ps.view_height_target, view_height::CROUCH);
        assert_eq!(last.bounds.maxs[2], 50.0);
        for _ in 0..20 {
            let was = ps.view_height_target;
            last = hold(&mut ps, buttons::PRONE);
            if was != view_height::PRONE && ps.view_height_target == view_height::PRONE {
                // Lying down holds the stance for 1.8 s.
                locked = ps.pm_flags & pm_flags::JUMPING != 0 && ps.pm_time > 1700;
            }
        }
        assert!(locked);
        assert_eq!(ps.view_height_current, 11.0);
        assert_eq!(ps.pm_flags & pm_flags::PRONE, pm_flags::PRONE);
        assert_eq!(last.bounds.maxs[2], 30.0);

        // Letting go: the get-up animation once, then up by way of crouch.
        let first = hold(&mut ps, 0);
        assert_eq!(first.stance_event, Some(16));
        assert!(first.reset_torso);
        assert_eq!(ps.view_height_target, view_height::CROUCH);
        for _ in 0..20 {
            last = hold(&mut ps, 0);
            assert_eq!(last.stance_event, None);
        }
        assert_eq!(ps.view_height_current, 60.0);
        assert_eq!(ps.pm_flags & (pm_flags::PRONE | pm_flags::CROUCH), 0);
        assert_eq!(last.bounds.maxs[2], 70.0);
    }
}
