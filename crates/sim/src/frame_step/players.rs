//! The movement phase: every user command queued for the frame is turned into a move through
//! Black Ops II's movement code, and the weapon code is then advanced by the same command.
//!
//! One command goes through five steps, each its own function below: check that the player can
//! take it, set up the movement context from the gun in hand, let the scripts adjust the command,
//! move through the map, and record what the move did.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use bevy_ecs::prelude::World;
use movement_t6::bridge::{Bo2Feel, HullExtents, OneTickMotionArgs, footsteps_anim_move_type};
use playerstate_t6::game::{AvatarState, InputCommand, buttons, movement_state_bits};
use weapon_t6::combat_facts::GunBattleProfile;

use super::solids::{
    SweepCollisionProvider, PlayerModelTouches, alive_body_clips, oriented_blocker_brushes, stance_room_trace, unstick_feet,
    unstick_log,
};
use super::feel::{HandlingFacts, bo2_feel, pmove_context};
use super::actions::retag_cheat_move_aim;
use super::phases::{stage_build_entity_models, stage_unstick_players};
use super::{StallGuard, FrameTicket, step_view_of, is_alive};
use crate::shot_trace::AttachedSolidShape;
use crate::gunfire::{progress_weapon_order, emit_shot_events, trace_shots};
use crate::step_view::StepWorldView;
use crate::ent_slots::{EntityPassStage, MoverSlot};
use crate::match_ids::{GameRoundStage, ScriptedModel};
use crate::sim_parts::{SeatNo, SimContent, StepNo};
use crate::ServerModelSource;

/// A command is never run for more time than this, however late it arrives.
const LONGEST_COMMAND_MS: i32 = 200;

/// With no weapon facts the sights move at this rate (a fifth of a second to fully raise).
const DEFAULT_ADS_RATE: f32 = 1.0 / 200.0;

/// The part of the map that is the same for every command of the frame.
struct MapSnapshot {
    content: Arc<SimContent>,
    movers: Vec<MoverSlot>,
    /// Entity number of each script mover, for the platforms players ride.
    mover_numbers: BTreeMap<ScriptedModel, u16>,
    /// Black Ops II's own collision is used when the map carries its brushes' corner points.
    own_collision: bool,
}

impl MapSnapshot {
    fn capture(world: &mut StepWorldView) -> Self {
        let content = world.content();
        let own_collision = content.collision_solids().iter().any(|b| !b.vertex_points.is_empty());
        let movers = crate::step_view::gather_movers(world.ecs());
        let mover_numbers = movers
            .iter()
            .map(|mover| (mover.id, mover.state.slot_index as u16))
            .collect();
        Self {
            content,
            movers,
            mover_numbers,
            own_collision,
        }
    }

    /// Every brush model with the number of the mover it belongs to (the world's number when it
    /// has none), so a hit on it names the platform a player stands on, plus whose it is.
    fn solid_models(
        &self,
        world: &StepWorldView,
        id: SeatNo,
    ) -> (Vec<(u16, AttachedSolidShape)>, Vec<ServerModelSource>) {
        world
            .entity_hit_support()
            .iter()
            .flat_map(|caps| {
                let entnum = caps
                    .owner
                    .scripted_model()
                    .and_then(|id| self.mover_numbers.get(&id).copied())
                    .unwrap_or(trace_t6::WORLD_ENTITY_SLOT);
                // bo2zm: a brush hidden from this player does not stop him (BO2's
                // query_should_pass_gentity); it stops the others.
                caps.solid_brushes_for(id.0)
                    .iter()
                    .map(move |brush| ((entnum, brush.clone()), caps.owner))
            })
            .unzip()
    }
}

/// The three tables of what each client held down last frame. Commands are run against the
/// previous frame's tables and the tables are put back afterwards; only the finalize phase
/// moves them on.
struct InputMemory {
    buttons: Vec<(SeatNo, u32)>,
    angles: Vec<(SeatNo, [i32; 3])>,
    moves: Vec<(SeatNo, [i8; 2])>,
}

impl InputMemory {
    fn save(world: &mut StepWorldView) -> Self {
        Self {
            buttons: world.prev_button_state_mut().clone(),
            angles: world.prev_input_angles_mut().clone(),
            moves: world.old_cmd_moves_mut().clone(),
        }
    }

    fn restore(self, world: &mut StepWorldView) {
        *world.prev_button_state_mut() = self.buttons;
        *world.prev_input_angles_mut() = self.angles;
        *world.old_cmd_moves_mut() = self.moves;
    }
}

/// Which player is being moved, and when.
#[derive(Clone, Copy)]
struct Moment {
    tick: StepNo,
    match_clock_ms: i32,
    id: SeatNo,
}

pub(super) fn run_players_system(ecs: &mut World) {
    let _stall = StallGuard::new("run_players");
    let (tick, given) = {
        let request = ecs.resource::<FrameTicket>();
        (request.tick, request.input.clone())
    };
    let match_clock_ms = crate::level_time_ms(tick);
    let mut world = step_view_of(ecs);
    world.begin_pass_stage(EntityPassStage::RunAvatars);

    let moving = matches!(world.phase(), GameRoundStage::Playing | GameRoundStage::Warmup);
    world.prune_mantle_blockers(match_clock_ms);
    let map = MapSnapshot::capture(&mut world);

    let memory = InputMemory::save(&mut world);
    let mut used_up = Vec::new();
    if moving {
        stage_build_entity_models(&mut world);
        for &(id, sent) in &given.cmds {
            let at = Moment {
                tick,
                match_clock_ms,
                id,
            };
            if let Some(ran) = run_command(&mut world, &map, at, sent) {
                used_up.push((id, ran));
            }
        }
    }
    memory.restore(&mut world);

    retag_cheat_move_aim(&mut world, &given.actions);
    if moving && world.emits_state_snapshot() {
        stage_unstick_players(&mut world);
    }
    drop(world);
    if moving {
        ecs.resource_mut::<FrameTicket>().input.cmds = used_up;
    }
}

/// Runs one command for one player. Returns the command as it was finally run, or `None` when
/// the player could not take it (not alive, or the command is not newer than the last one).
fn run_command(
    world: &mut StepWorldView,
    map: &MapSnapshot,
    at: Moment,
    sent: InputCommand,
) -> Option<InputCommand> {
    let id = at.id;
    world.pick_rewind_order(id, sent.authority_clock_ms);
    let (ps, elapsed, last_tick_buttons) = ready_to_move(world, id, sent.authority_clock_ms)?;

    let (mut context, feel) = movement_setup(world, at.tick, id, &ps, last_tick_buttons);
    let mut seq = adjust_command(world, id, sent, last_tick_buttons, elapsed, feel.zombies);
    let commanded_move = seq.thrust_ahead != 0 || seq.thrust_lateral != 0;
    let keys_as_sent = [seq.thrust_ahead, seq.thrust_lateral];
    context.old_move = world.old_cmd_move(id).unwrap_or([0, 0]);

    let outcome = move_through_map(world, map, at, context, feel, &mut seq);
    record_move(world, at, commanded_move, outcome);

    run_weapon(world, at, seq, elapsed);
    super::close::settle_player_events(world, at.tick, id);
    world.store_prior_input(id, seq.buttons, seq.angles);
    world.set_old_cmd_move(id, keys_as_sent);
    Some(seq)
}

/// The player row, the time this command covers and what was held last command, when the
/// player can take a command at all.
fn ready_to_move(
    world: &StepWorldView,
    id: SeatNo,
    authority_clock_ms: i32,
) -> Option<(AvatarState, i32, u32)> {
    if !is_alive(world, id) {
        return None;
    }
    let ps = *world.avatar(id)?;
    let elapsed = authority_clock_ms.wrapping_sub(ps.last_cmd_ms);
    if elapsed <= 0 {
        return None;
    }
    Some((ps, elapsed, world.last_tick_buttons(id).unwrap_or(0)))
}

/// Whether the gun in hand lets its holder aim down the sights right now. Black Ops II's own
/// rule comes first; the weapon table's rule answers only when that one has no opinion.
fn aiming_allowed(
    world: &StepWorldView,
    id: SeatNo,
    pstate: &AvatarState,
    facts: Option<&GunBattleProfile>,
) -> bool {
    if let Some(verdict) = crate::gun_brain::sights_permitted(world, id, pstate) {
        return verdict;
    }
    let Some(facts) = facts.filter(|_| pstate.weapon != 0) else {
        return false;
    };
    let mut allow = facts.ads_allow_facts();
    allow.motion_clip_slot = weapon_t6::ammo_table::clip_table_key(allow.motion_clip_slot, pstate.weapon);
    let holder = weapon_t6::ads_allow::AdsAllowPlayer {
        previous_gun_hand: pstate.previous_gun_hand,
        ent_bits: pstate.e_flags,
        movement_mode: pstate.movement_mode,
        secondary_state_bits: pstate.secondary_state_bits,
        movement_state_bits: pstate.movement_state_bits,
        weapon_state_bits: pstate.weapon_state_bits,
        weapon_state_t6: pstate.weapon_state_t6,
    };
    weapon_t6::ads_allow::is_ads_allowed_bo2(
        &holder,
        &allow,
        &pstate.magazine_rows,
        pstate.previous_gun_hand.min(1) as u8,
    )
}

/// Builds the movement context for this player from the gun in hand and the rules in force.
fn movement_setup(
    world: &mut StepWorldView,
    tick: StepNo,
    seat: SeatNo,
    pstate: &AvatarState,
    last_tick_buttons: u32,
) -> (OneTickMotionArgs, Bo2Feel) {
    let gun = pstate.weapon;
    let facts = world.damage_profile_of(gun);

    let predicting = !world.emits_state_snapshot();
    let rules_from_host = world.feel();
    let mut feel = bo2_feel(world.ecs(), predicting, rules_from_host);
    if !predicting {
        // Every snapshot carries the rules on to the other PCs.
        world.set_feel(feel);
    }
    if let Some(f) = &facts {
        // The gun in hand sets its own footstep rhythm.
        feel.sprint_cycle_scale = f.sprint_cycle_scale;
        feel.ducked_sprint_cycle_scale = f.ducked_sprint_cycle_scale;
        feel.dtp_cycle_scale = f.dtp_cycle_scale;
    }

    let (ads_in, ads_out, rack_bolt_when_scoped, fire_only_when_scoped) = facts
        .map(|f| f.scope_blend_bundle())
        .unwrap_or((DEFAULT_ADS_RATE, DEFAULT_ADS_RATE, true, false));
    // Quickdraw skips the sniper anim sets, which is a property of the anim type and not of
    // the sniper class.
    let (scope_raise_speed, scope_lower_speed) = movement_t6::quickdraw_ads_rates(
        pstate,
        ads_in,
        ads_out,
        facts.map_or(0, |f| f.player_motion_kind),
        feel,
    );
    let shellshock_scale = world
        .client_meta(seat)
        .and_then(|m| m.shellshock.as_ref())
        .map_or(1.0, |shock| {
            // Black Ops II shocks carry no walk switch; the scale goes through the move speed
            // multiplier instead.
            movement_t6::shellshock_walk_scale(false, shock.movement_scale)
        });
    let handling = HandlingFacts {
        weapon_scales: world.damage_scales_of(gun),
        sights_permitted: aiming_allowed(world, seat, pstate, facts.as_ref()),
        scoped_aiming: facts.is_some_and(|f| f.scoped_aiming),
        scope_raise_speed,
        scope_lower_speed,
        rack_bolt_when_scoped,
        ads_reload: facts.map_or((false, 0, false), |f| f.ads_reload_rule()),
        fire_only_when_scoped,
        strike_wait_millis: facts.map_or(0, |f| f.strike_wait_millis),
        lunge_windup_wait_millis: facts.map_or(0, |f| f.lunge_windup_wait_millis),
        hud_crosshair_layer: facts.map_or(0, |f| f.hud_crosshair_layer),
        shellshock_movement_scale: shellshock_scale,
    };
    let mut context = pmove_context(last_tick_buttons, &handling, feel);

    // The Hybrid Optic swaps sights on the sprint key; tapping it while aimed only swaps the
    // sight and must not drop the aim.
    context.ads_intent.dash_keeps_scope = facts.is_some_and(|f| f.optic_switch);
    // The T6 weapon code lerps the sights, and owns the knife lunge.
    let weapon_code_owns = !world.gun_facts().is_empty();
    context.ads_frac.weapon_brain_owns = weapon_code_owns;
    context.gun_forbids_lying = world.gun_facts().get(gun).moves.blocks_prone;
    if weapon_code_owns {
        context.walk.lunge = Some(crate::knife_lunge::move_lunge(world, seat, tick, feel.zombies));
    }
    // Inside a scorestreak vehicle the view keeps to the vehicle's limits.
    if let Some([up, down]) = world.client_meta(seat).and_then(|m| m.view_pitch_clamp) {
        context.look_rotation.tilt_upper_limit = up;
        context.look_rotation.tilt_lower_limit = down;
    }
    (context, feel)
}

/// Lets the scripts and the player's current state change what the keys say. A player steering
/// a remote missile does not move: only the stance keys get through.
fn adjust_command(
    world: &mut StepWorldView,
    seat: SeatNo,
    sent: InputCommand,
    last_tick_buttons: u32,
    elapsed: i32,
    zombies: bool,
) -> InputCommand {
    let mut cmd = sent;
    crate::client_script::constrain_cmd(world, seat, &mut cmd, zombies);
    crate::gsc_host::select_location(world.ecs(), seat.0, &mut cmd, last_tick_buttons);
    if world
        .client_meta(seat)
        .is_some_and(|m| m.remote_missile.is_some())
    {
        crate::rider_missile::steer(world, seat, &cmd, elapsed.min(LONGEST_COMMAND_MS));
        cmd.thrust_ahead = 0;
        cmd.thrust_lateral = 0;
        cmd.buttons &= buttons::DUCK_BUTTON_BIT | buttons::LYING_FLAT;
    }
    cmd
}

/// Everything one move reports back for the bookkeeping that follows it.
struct MoveOutcome {
    walking: i32,
    bounds: HullExtents,
    footstep_move_class: Option<u8>,
    from: [f32; 3],
    to: [f32; 3],
    view_change: f32,
    /// Where a climb that started this move ends, and how long it takes.
    mantle_landing: Option<([f32; 3], i32)>,
    body_anims: movement_t6::BodyAnims,
    /// The script models the move touched.
    touched: Vec<ServerModelSource>,
    /// The standing windows the body ran into (pane, where, which way): each shatters.
    panes_run_into: Vec<(u32, [f32; 3], [f32; 3])>,
}

/// Slower than this (units a second, flat) a body leaning on a window leaves it whole.
const BODY_BREAKS_GLASS_SPEED: f32 = 50.0;

/// The standing windows a body moving at `velocity` from `origin` runs into within the
/// next tick: Black Ops II shatters a pane a player walks, jumps or dives through.
fn panes_run_into(
    backend: &SweepCollisionProvider<'_>,
    origin: [f32; 3],
    velocity: [f32; 3],
    bounds: HullExtents,
) -> Vec<(u32, [f32; 3], [f32; 3])> {
    let speed = velocity[0].hypot(velocity[1]);
    if speed < BODY_BREAKS_GLASS_SPEED {
        return Vec::new();
    }
    let reach = speed * crate::SIM_STEP_MILLIS as f32 / 1000.0 + 2.0;
    let way = [velocity[0] / speed, velocity[1] / speed, 0.0];
    let end = [origin[0] + way[0] * reach, origin[1] + way[1] * reach, origin[2]];
    backend
        .brushes
        .iter()
        .filter(|pane| pane.glass_hit_code != 0 && backend.glass_stands(pane.glass_hit_code - 1))
        .filter_map(|pane| {
            let hit = crate::sim_parts::t6_brushes_trace(
                &[pane],
                origin,
                end,
                bounds.lo_corner,
                bounds.hi_corner,
                bounds.collide_filter,
            );
            (hit.fraction < 1.0 || hit.starts_inside).then(|| {
                let at = [0, 1, 2].map(|k| origin[k] + (end[k] - origin[k]) * hit.fraction);
                (u32::from(pane.glass_hit_code - 1), at, way)
            })
        })
        .collect()
}

/// Runs the player's move through the map: builds the collision the move sees, applies the
/// per-frame player upkeep and calls Black Ops II's movement.
fn move_through_map(
    level: &mut StepWorldView,
    map: &MapSnapshot,
    at: Moment,
    context: OneTickMotionArgs,
    feel: Bo2Feel,
    cmd: &mut InputCommand,
) -> MoveOutcome {
    let id = at.id;
    let serving = level.emits_state_snapshot();
    let frozen = level.client_meta(id).is_some_and(|m| m.controls.stuck);

    let (attached_solids, linked_owners) = map.solid_models(level, id);
    let struck = RefCell::new(Vec::new());
    let touched = RefCell::new(Vec::new());
    let masses = alive_body_clips(level);
    let blockers = oriented_blocker_brushes(level);
    let slick = level.client_meta(id).is_some_and(|m| m.controls.slick);
    let pane_harm = level.world_objects().pane_harm_entries();
    let content = &map.content;
    let backend = SweepCollisionProvider {
        brushes: content.collision_solids(),
        bsp: content.collision_tree(),
        mesh: content.collision_triangles(),
        pane_harm: &pane_harm,
        player_boxes: &masses,
        own_entity_slot: id.0 as u16,
        cmodels: &content.collision_submodel_set().models,
        attached_solids: &attached_solids,
        linked_owners: &linked_owners,
        struck: Some(&struck),
        touched: Some(&touched),
        blockers: &blockers,
        movers: &map.movers,
        vehicles: &[],
        gjk: map.own_collision,
        slick,
    };

    let animation_refs = level.animation_refs();
    // bo2zm: a bus rider is carried through the map's clip brushes (BO2
    // moves him with its mover, no trace: G_PushEntity); only the
    // movement code's own 1-unit fix runs then, not the wider unstick
    // (runs/sl3: the bus's front sweeps through the road's player clip
    // on its turns and shoved him 1-3 units each tick). The client's
    // predicted step has no riders list: it starts from the server's
    // place, so it skips it too while he stands on a mover or is off
    // the ground (runs/sl4: 9 predicted shoves, none on the server).
    let authority = level
        .ecs()
        .resource::<FrameTicket>()
        .reason
        .moves_server_world_forward();
    let carried = level
        .ecs()
        .get_resource::<crate::zombie_rt::Zm>()
        .is_some_and(|zm| zm.riders.carried.contains(&id.0))
        || (!authority
            && level.avatar(id).is_some_and(|pstate| {
                pstate.standing_on != i32::from(trace_t6::WORLD_ENTITY_SLOT)
            }));
    let pstate = level
        .occupant_mut(id)
        .expect("a living seat always owns a body row");
    let was_mantling = pstate.movement_state_bits & movement_state_bits::MANTLE != 0;
    if pstate.stun_start.wrapping_add(pstate.stun_length) < at.match_clock_ms {
        pstate.movement_state_bits &= !movement_state_bits::SHELLSHOCKED;
    }
    // bo2zm: a spot a hair inside a brush (Ascension's lift top is 0.1 above its
    // spawn spots) leaves him unable to move; lift him the smallest step that clears.
    // Not mid-climb: a BO2 climb starts with the body below the ledge's foot and
    // follows its animation from there; lifting it then ends the climb short and high.
    let stuck_at = pstate.origin;
    if !carried && !was_mantling {
        pstate.origin = unstick_feet(&backend, pstate.origin);
    }
    if pstate.origin != stuck_at {
        unstick_log(&backend, stuck_at, pstate.origin);
    }
    // Black Ops II sets the walk speed afresh every frame (move-faster adds 7%).
    pstate.speed = movement_t6::bridge::player_speed(pstate, feel);
    // bg_gravity is everyone's gravity; the mod menu's Low Gravity sets it.
    if !feel.zombies {
        pstate.gravity = feel.gravity;
    }
    // PmoveSingle stops a frozen player dead every frame, in the air too.
    if frozen {
        pstate.velocity = [0.0; 3];
    }
    let from = pstate.origin;
    let velocity_before = pstate.velocity;
    let (result, body_anims) = movement_t6::pmove_with_body(
        pstate,
        cmd,
        context,
        &backend,
        animation_refs.as_ref(),
        animation_refs.as_ref(),
        serving,
    );
    let footstep_move_class = footsteps_anim_move_type(
        pstate,
        cmd.thrust_ahead,
        cmd.thrust_lateral,
        result.pml.near_floor_contact != 0,
    );
    stance_room_trace(&backend, pstate);
    let mantle_landing = (!was_mantling && pstate.movement_state_bits & movement_state_bits::MANTLE != 0)
        .then(|| mantle_landing_spot(pstate, animation_refs.as_ref()));
    let panes_run_into = panes_run_into(&backend, pstate.origin, velocity_before, result.bounds);
    MoveOutcome {
        walking: result.pml.walking as i32,
        bounds: result.bounds,
        footstep_move_class,
        from,
        to: pstate.origin,
        view_change: result.view_change,
        mantle_landing,
        body_anims,
        touched: touched.take(),
        panes_run_into,
    }
}

/// Where the climb that has just started ends, and how many milliseconds it takes: the player's
/// position moved on by the root motion still to come.
fn mantle_landing_spot(ps: &AvatarState, tree: &crate::LedgeClimbAnimSet) -> ([f32; 3], i32) {
    use movement_t6::bridge::mantle::{duration, sample_root_track};
    let total = duration(ps, tree);
    let now = sample_root_track(ps, ps.mantle_state.timer, tree, tree);
    let end = sample_root_track(ps, total, tree, tree);
    ([0, 1, 2].map(|axis| ps.origin[axis] + end[axis] - now[axis]), total)
}

/// What the world must hear about a finished move.
fn record_move(world: &mut StepWorldView, at: Moment, commanded_move: bool, outcome: MoveOutcome) {
    let id = at.id;
    // The jump and land of the move, for the legs.
    crate::zombie_rt::note_body_anims(world.ecs(), id.0, outcome.body_anims);
    world
        .client_meta_mut(id)
        .input_receipt
        .record(commanded_move, outcome.from, outcome.to);
    // What the move touched, for the server's "touch" notifies (the touch pass after Pmove).
    if !outcome.touched.is_empty() && world.emits_state_snapshot() {
        world
            .ecs()
            .get_resource_or_insert_with(PlayerModelTouches::default)
            .0
            .extend(outcome.touched.into_iter().map(|owner| (id.0, owner)));
    }
    // A body moving into a standing window shatters it (the server's glass; the client hears).
    if world.emits_state_snapshot() {
        for (pane, at_spot, way) in outcome.panes_run_into {
            let _ = world
                .world_objects_mut()
                .shatter_glass(pane, at.match_clock_ms, crate::glass_and_props::Strike { at: at_spot, way });
        }
    }
    if let Some(movetype) = outcome.footstep_move_class {
        world.note_stride_kind(id, movetype);
    }
    world.set_pmove_walking(id, outcome.walking);
    world.set_pmove_view_change(id, outcome.view_change);
    world.link_player_area(id, outcome.bounds);
    // After a climb starts the spot it ends on is the climber's until the block-time buffer
    // after the climb is over.
    if let Some((centre, duration)) = outcome.mantle_landing {
        let ends = at.match_clock_ms + duration;
        world.add_mantle_blocker(crate::sim_parts::MantleBlocker {
            owner: id.0 as u16,
            spot: centre,
            ends,
            until: ends + crate::sim_parts::MANTLE_BLOCK_TIME_BUFFER_MS,
        });
    }
}

/// Advances the weapon by the command and traces every shot it let off.
fn run_weapon(level: &mut StepWorldView, at: Moment, cmd: InputCommand, elapsed: i32) {
    let Moment { tick: step, id, .. } = at;
    for blow in progress_weapon_order(level, step, id, cmd, elapsed.min(LONGEST_COMMAND_MS)) {
        crate::rocketry::fire_accepted_shot(level, step, &blow);

        let shooter = [id];
        let serving = level.emits_state_snapshot();
        level.record_collision_history(step, 0, (!serving).then_some(shooter.as_slice()));
        if serving {
            level.log_entity_hitboxes(step);
        }
        let shots_out = emit_shot_events(level, core::slice::from_ref(&blow));
        trace_shots(level, step, &shots_out);
    }
}
