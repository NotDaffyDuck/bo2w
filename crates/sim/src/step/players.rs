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
use movement_t6::bridge::{Bo2Feel, MoveBounds, PmoveSingleContext, footsteps_anim_move_type};
use playerstate_t6::game::{PlayerState, UserCmd, buttons, pm_flags};
use weapon_t6::combat_facts::WeaponCombatFacts;

use super::clip::{ClipBackend, PlayerModelTouches, alive_body_clips, oriented_blocker_brushes};
use super::feel::{HandlingFacts, bo2_feel, pmove_context};
use super::actions::restamp_debug_move_look;
use super::phases::{phase_materialize_entity_dobjs, phase_stuck_in_client};
use super::{StallGuard, StepRequest, frame_world, is_alive};
use crate::bullet_collision::LinkedBrushCollisionBrush;
use crate::combat::{advance_weapon_command, phase_emit, phase_trace};
use crate::frame::FrameWorld;
use crate::gentity::{KernelPhase, ScriptMoverGentity};
use crate::identities::{MatchPhase, ScriptModelId};
use crate::world::{ClientId, SimContent, Tick};
use crate::AuthorityModelOwner;

/// A command is never run for more time than this, however late it arrives.
const LONGEST_COMMAND_MS: i32 = 200;

/// With no weapon facts the sights move at this rate (a fifth of a second to fully raise).
const DEFAULT_ADS_RATE: f32 = 1.0 / 200.0;

/// The part of the map that is the same for every command of the frame.
struct MapSnapshot {
    content: Arc<SimContent>,
    movers: Vec<ScriptMoverGentity>,
    /// Entity number of each script mover, for the platforms players ride.
    mover_numbers: BTreeMap<ScriptModelId, u16>,
    /// Black Ops II's own collision is used when the map carries its brushes' corner points.
    own_collision: bool,
}

impl MapSnapshot {
    fn capture(world: &mut FrameWorld) -> Self {
        let content = world.content();
        let own_collision = content.clip_brushes().iter().any(|b| !b.verts.is_empty());
        let movers = crate::frame::collect_script_movers(world.ecs());
        let mover_numbers = movers
            .iter()
            .map(|mover| (mover.id, mover.state.number as u16))
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
    fn brush_models(
        &self,
        world: &FrameWorld,
    ) -> (Vec<(u16, LinkedBrushCollisionBrush)>, Vec<AuthorityModelOwner>) {
        world
            .entity_collision_capabilities()
            .iter()
            .flat_map(|caps| {
                let entnum = caps
                    .owner
                    .script_model()
                    .and_then(|id| self.mover_numbers.get(&id).copied())
                    .unwrap_or(trace_t6::ENTITYNUM_WORLD);
                caps.solid_brushes()
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
    buttons: Vec<(ClientId, u32)>,
    angles: Vec<(ClientId, [i32; 3])>,
    moves: Vec<(ClientId, [i8; 2])>,
}

impl InputMemory {
    fn save(world: &mut FrameWorld) -> Self {
        Self {
            buttons: world.old_buttons_mut().clone(),
            angles: world.old_cmd_angles_mut().clone(),
            moves: world.old_cmd_moves_mut().clone(),
        }
    }

    fn restore(self, world: &mut FrameWorld) {
        *world.old_buttons_mut() = self.buttons;
        *world.old_cmd_angles_mut() = self.angles;
        *world.old_cmd_moves_mut() = self.moves;
    }
}

/// Which player is being moved, and when.
#[derive(Clone, Copy)]
struct Moment {
    tick: Tick,
    level_time: i32,
    id: ClientId,
}

pub(super) fn run_players_system(ecs: &mut World) {
    let _stall = StallGuard::new("run_players");
    let (tick, input) = {
        let request = ecs.resource::<StepRequest>();
        (request.tick, request.input.clone())
    };
    let level_time = crate::level_time_ms(tick);
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::RunPlayers);

    let moving = matches!(world.phase(), MatchPhase::Playing | MatchPhase::Warmup);
    world.prune_mantle_blockers(level_time);
    let map = MapSnapshot::capture(&mut world);

    let memory = InputMemory::save(&mut world);
    let mut consumed = Vec::new();
    if moving {
        phase_materialize_entity_dobjs(&mut world);
        for &(id, sent) in &input.cmds {
            let at = Moment {
                tick,
                level_time,
                id,
            };
            if let Some(ran) = run_command(&mut world, &map, at, sent) {
                consumed.push((id, ran));
            }
        }
    }
    memory.restore(&mut world);

    restamp_debug_move_look(&mut world, &input.actions);
    if moving && world.publishes_snapshot() {
        phase_stuck_in_client(&mut world);
    }
    drop(world);
    if moving {
        ecs.resource_mut::<StepRequest>().input.cmds = consumed;
    }
}

/// Runs one command for one player. Returns the command as it was finally run, or `None` when
/// the player could not take it (not alive, or the command is not newer than the last one).
fn run_command(
    world: &mut FrameWorld,
    map: &MapSnapshot,
    at: Moment,
    sent: UserCmd,
) -> Option<UserCmd> {
    let id = at.id;
    world.select_lagcomp_command(id, sent.server_time);
    let (ps, elapsed, old_buttons) = ready_to_move(world, id, sent.server_time)?;

    let (mut context, feel) = movement_setup(world, at.tick, id, &ps, old_buttons);
    let mut cmd = adjust_command(world, id, sent, old_buttons, elapsed, feel.zombies);
    let commanded_move = cmd.forwardmove != 0 || cmd.rightmove != 0;
    let keys_as_sent = [cmd.forwardmove, cmd.rightmove];
    context.old_move = world.old_cmd_move(id).unwrap_or([0, 0]);

    let outcome = move_through_map(world, map, at, context, feel, &mut cmd);
    record_move(world, at, commanded_move, outcome);

    run_weapon(world, at, cmd, elapsed);
    world.set_old_cmd(id, cmd.buttons, cmd.angles);
    world.set_old_cmd_move(id, keys_as_sent);
    Some(cmd)
}

/// The player row, the time this command covers and what was held last command, when the
/// player can take a command at all.
fn ready_to_move(
    world: &FrameWorld,
    id: ClientId,
    server_time: i32,
) -> Option<(PlayerState, i32, u32)> {
    if !is_alive(world, id) {
        return None;
    }
    let ps = *world.player(id)?;
    let elapsed = server_time.wrapping_sub(ps.command_time);
    if elapsed <= 0 {
        return None;
    }
    Some((ps, elapsed, world.old_buttons(id).unwrap_or(0)))
}

/// Whether the gun in hand lets its holder aim down the sights right now. Black Ops II's own
/// rule comes first; the weapon table's rule answers only when that one has no opinion.
fn aiming_allowed(
    world: &FrameWorld,
    id: ClientId,
    ps: &PlayerState,
    facts: Option<&WeaponCombatFacts>,
) -> bool {
    if let Some(verdict) = crate::t6_gun::ads_allowed(world, id, ps) {
        return verdict;
    }
    let Some(facts) = facts.filter(|_| ps.weapon != 0) else {
        return false;
    };
    let mut allow = facts.ads_allow_facts();
    allow.clip_index = weapon_t6::ammo_table::clip_table_key(allow.clip_index, ps.weapon);
    let holder = weapon_t6::ads_allow::AdsAllowPlayer {
        last_weapon_hand: ps.last_weapon_hand,
        e_flags: ps.e_flags,
        pm_type: ps.pm_type,
        other_flags: ps.other_flags,
        pm_flags: ps.pm_flags,
        weap_flags: ps.weap_flags,
        weapon_state_t6: ps.weapon_state_t6,
    };
    weapon_t6::ads_allow::is_ads_allowed_bo2(
        &holder,
        &allow,
        &ps.ammoclip,
        ps.last_weapon_hand.min(1) as u8,
    )
}

/// Builds the movement context for this player from the gun in hand and the rules in force.
fn movement_setup(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    ps: &PlayerState,
    old_buttons: u32,
) -> (PmoveSingleContext, Bo2Feel) {
    let weapon = ps.weapon;
    let facts = world.combat_facts_for(weapon);

    let predicting = !world.publishes_snapshot();
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

    let (ads_in, ads_out, rechamber_while_ads, ads_fire_only) = facts
        .map(|f| f.ads_frac_context())
        .unwrap_or((DEFAULT_ADS_RATE, DEFAULT_ADS_RATE, true, false));
    // Quickdraw skips the sniper anim sets, which is a property of the anim type and not of
    // the sniper class.
    let (ads_in_rate, ads_out_rate) = movement_t6::quickdraw_ads_rates(
        ps,
        ads_in,
        ads_out,
        facts.map_or(0, |f| f.player_anim_type),
        feel,
    );
    let shellshock_scale = world
        .client_meta(id)
        .and_then(|m| m.shellshock.as_ref())
        .map_or(1.0, |shock| {
            // Black Ops II shocks carry no walk switch; the scale goes through the move speed
            // multiplier instead.
            movement_t6::shellshock_walk_scale(false, shock.movement_scale)
        });
    let handling = HandlingFacts {
        weapon_scales: world.scales_for(weapon),
        ads_allowed: aiming_allowed(world, id, ps, facts.as_ref()),
        aim_down_sight: facts.is_some_and(|f| f.aim_down_sight),
        ads_in_rate,
        ads_out_rate,
        rechamber_while_ads,
        ads_reload: facts.map_or((false, 0, false), |f| f.ads_reload_rule()),
        ads_fire_only,
        melee_delay_ms: facts.map_or(0, |f| f.melee_delay_ms),
        melee_charge_delay_ms: facts.map_or(0, |f| f.melee_charge_delay_ms),
        overlay_reticle: facts.map_or(0, |f| f.overlay_reticle),
        shellshock_movement_scale: shellshock_scale,
    };
    let mut context = pmove_context(old_buttons, &handling, feel);

    // The Hybrid Optic swaps sights on the sprint key; tapping it while aimed only swaps the
    // sight and must not drop the aim.
    context.ads_intent.sprint_hold_ads = facts.is_some_and(|f| f.optic_switch);
    // The T6 weapon code lerps the sights, and owns the knife lunge.
    let weapon_code_owns = !world.gun_facts().is_empty();
    context.ads_frac.weapon_brain_owns = weapon_code_owns;
    context.weapon_blocks_prone = world.gun_facts().get(weapon).moves.blocks_prone;
    if weapon_code_owns {
        context.walk.lunge = Some(crate::t6_lunge::move_lunge(world, id, tick, feel.zombies));
    }
    // Inside a scorestreak vehicle the view keeps to the vehicle's limits.
    if let Some([up, down]) = world.client_meta(id).and_then(|m| m.view_pitch_clamp) {
        context.view_angles.pitch_up = up;
        context.view_angles.pitch_down = down;
    }
    (context, feel)
}

/// Lets the scripts and the player's current state change what the keys say. A player steering
/// a remote missile does not move: only the stance keys get through.
fn adjust_command(
    world: &mut FrameWorld,
    id: ClientId,
    sent: UserCmd,
    old_buttons: u32,
    elapsed: i32,
    zombies: bool,
) -> UserCmd {
    let mut cmd = sent;
    crate::script::player_commands(world.ecs(), id.0, sent.buttons, old_buttons);
    crate::script_player::constrain_cmd(world, id, &mut cmd, zombies);
    crate::script::select_location(world.ecs(), id.0, &mut cmd, old_buttons);
    if world
        .client_meta(id)
        .is_some_and(|m| m.remote_missile.is_some())
    {
        crate::remote_missile::steer(world, id, &cmd, elapsed.min(LONGEST_COMMAND_MS));
        cmd.forwardmove = 0;
        cmd.rightmove = 0;
        cmd.buttons &= buttons::CROUCH | buttons::PRONE;
    }
    cmd
}

/// Everything one move reports back for the bookkeeping that follows it.
struct MoveOutcome {
    walking: i32,
    bounds: MoveBounds,
    anim_movetype: Option<u8>,
    from: [f32; 3],
    to: [f32; 3],
    view_change: f32,
    /// Where a climb that started this move ends, and how long it takes.
    mantle_landing: Option<([f32; 3], i32)>,
    body_anims: movement_t6::BodyAnims,
    /// The script models the move touched.
    touched: Vec<AuthorityModelOwner>,
}

/// Runs the player's move through the map: builds the collision the move sees, applies the
/// per-frame player upkeep and calls Black Ops II's movement.
fn move_through_map(
    world: &mut FrameWorld,
    map: &MapSnapshot,
    at: Moment,
    context: PmoveSingleContext,
    feel: Bo2Feel,
    cmd: &mut UserCmd,
) -> MoveOutcome {
    let id = at.id;
    let serving = world.publishes_snapshot();
    let frozen = world.client_meta(id).is_some_and(|m| m.controls.frozen);

    let (linked_brushes, linked_owners) = map.brush_models(world);
    let struck = RefCell::new(Vec::new());
    let touched = RefCell::new(Vec::new());
    let bodies = alive_body_clips(world);
    let blockers = oriented_blocker_brushes(world);
    let glass_damage = world.world_objects().glass_damage_pairs();
    let content = &map.content;
    let backend = ClipBackend {
        brushes: content.clip_brushes(),
        bsp: content.clip_bsp(),
        mesh: content.clip_mesh(),
        glass_damage: &glass_damage,
        bodies: &bodies,
        self_entnum: id.0 as u16,
        cmodels: &content.clip_cmodels().models,
        linked_brushes: &linked_brushes,
        linked_owners: &linked_owners,
        struck: Some(&struck),
        touched: Some(&touched),
        blockers: &blockers,
        movers: &map.movers,
        vehicles: &[],
        gjk: map.own_collision,
    };

    let xanims = world.xanims();
    let ps = world
        .player_mut(id)
        .expect("Alive client has a player row");
    let was_mantling = ps.pm_flags & pm_flags::MANTLE != 0;
    if ps.shellshock_time.wrapping_add(ps.shellshock_duration) < at.level_time {
        ps.pm_flags &= !pm_flags::SHELLSHOCKED;
    }
    // Black Ops II sets the walk speed afresh every frame (move-faster adds 7%).
    ps.speed = movement_t6::bridge::player_speed(ps, feel);
    // bg_gravity is everyone's gravity; the mod menu's Low Gravity sets it.
    if !feel.zombies {
        ps.gravity = feel.gravity;
    }
    // PmoveSingle stops a frozen player dead every frame, in the air too.
    if frozen {
        ps.velocity = [0.0; 3];
    }
    let from = ps.origin;
    let (result, body_anims) = movement_t6::pmove_with_body(
        ps,
        cmd,
        context,
        &backend,
        xanims.as_ref(),
        xanims.as_ref(),
        serving,
    );
    let anim_movetype = footsteps_anim_move_type(
        ps,
        cmd.forwardmove,
        cmd.rightmove,
        result.pml.almost_ground_plane != 0,
    );
    let mantle_landing = (!was_mantling && ps.pm_flags & pm_flags::MANTLE != 0)
        .then(|| mantle_landing_spot(ps, xanims.as_ref()));
    MoveOutcome {
        walking: result.pml.walking as i32,
        bounds: result.bounds,
        anim_movetype,
        from,
        to: ps.origin,
        view_change: result.view_change,
        mantle_landing,
        body_anims,
        touched: touched.take(),
    }
}

/// Where the climb that has just started ends, and how many milliseconds it takes: the player's
/// position moved on by the root motion still to come.
fn mantle_landing_spot(ps: &PlayerState, tree: &crate::MantleXAnimBind) -> ([f32; 3], i32) {
    use movement_t6::bridge::mantle::{duration, sample_root_track};
    let total = duration(ps, tree);
    let now = sample_root_track(ps, ps.mantle_state.timer, tree, tree);
    let end = sample_root_track(ps, total, tree, tree);
    ([0, 1, 2].map(|axis| ps.origin[axis] + end[axis] - now[axis]), total)
}

/// What the world must hear about a finished move.
fn record_move(world: &mut FrameWorld, at: Moment, commanded_move: bool, outcome: MoveOutcome) {
    let id = at.id;
    // The jump and land of the move, for the legs.
    crate::t6::note_body_anims(world.ecs(), id.0, outcome.body_anims);
    world
        .client_meta_mut(id)
        .input_receipt
        .record(commanded_move, outcome.from, outcome.to);
    // What the move touched, for the server's "touch" notifies (the touch pass after Pmove).
    if !outcome.touched.is_empty() && world.publishes_snapshot() {
        world
            .ecs()
            .get_resource_or_insert_with(PlayerModelTouches::default)
            .0
            .extend(outcome.touched.into_iter().map(|owner| (id.0, owner)));
    }
    if let Some(movetype) = outcome.anim_movetype {
        world.set_last_anim_movetype(id, movetype);
    }
    world.set_pmove_walking(id, outcome.walking);
    world.set_pmove_view_change(id, outcome.view_change);
    world.link_player_area(id, outcome.bounds);
    // After a climb starts the spot it ends on is the climber's until the block-time buffer
    // after the climb is over.
    if let Some((origin, duration)) = outcome.mantle_landing {
        let ends = at.level_time + duration;
        world.add_mantle_blocker(crate::world::MantleBlocker {
            owner: id.0 as u16,
            origin,
            ends,
            until: ends + crate::world::MANTLE_BLOCK_TIME_BUFFER_MS,
        });
    }
}

/// Advances the weapon by the command and traces every shot it let off.
fn run_weapon(world: &mut FrameWorld, at: Moment, cmd: UserCmd, elapsed: i32) {
    let Moment { tick, id, .. } = at;
    for shot in advance_weapon_command(world, tick, id, cmd, elapsed.min(LONGEST_COMMAND_MS)) {
        crate::missile::fire_accepted_shot(world, tick, &shot);

        let shooter = [id];
        let serving = world.publishes_snapshot();
        world.record_collision_history(tick, 0, (!serving).then_some(shooter.as_slice()));
        if serving {
            world.record_entity_collision_history(tick);
        }
        let emissions = phase_emit(world, core::slice::from_ref(&shot));
        phase_trace(world, tick, &emissions);
    }
}
