use crate::frame::FrameWorld;
use std::cell::RefCell;

use bevy_ecs::prelude::{Resource, World};
use bevy_ecs::schedule::{IntoScheduleConfigs, Schedule};
use movement_t6::bridge::{
    ANGLE2SHORT, AirMoveContext, CmdScaleWalkContext, CollisionBackend, GroundTraceInput,
    JumpLaunchContext, MoveBounds, PmoveSingleContext, SHORT2ANGLE, SprintContext, ViewAngleClamp,
    WalkMoveContext, consume_player_events, footsteps_anim_move_type, get_max_sprint_time,
};
use trace_t6::{Trace, TraceHitType};

use crate::bullet_collision::{LinkedBrushCollisionBrush, PLAYER_MAXS, PLAYER_MINS};
use crate::gentity::{ScriptMoverGentity, apos_from_entity_state, pos_from_entity_state};
use crate::combat::{advance_weapon_command, phase_emit, phase_trace};
use crate::identities::MatchPhase;
use crate::input::{ClientAction, TickInput};
use crate::match_state::{ClientLifecycle, EventAudience, SimEvent};
use crate::snapshot::Snapshot;
use crate::world::{
    ClientId, SimBrush, SimClipBsp, SimClipMesh, SimState, Tick, clip_move_to_bmodels_struck, clip_trace,
    give_weapon_to_ps_akimbo, gsc_give_weapon_is_akimbo, inventory_add_weapon,
};
use playerstate_t6::game::PlayerState;
use playerstate_t6::game::buttons;

#[derive(Resource)]
pub(crate) struct StepRequest {
    pub(crate) tick: Tick,
    pub(crate) input: TickInput,
    msec: i32,
    pub(crate) reason: crate::StepReason,
    output: Option<Snapshot>,
}

/// bo2mp: a part of the authority step slower than this names itself in
/// the log (ms), profiling or not: a freeze he would feel says where it was.
const STALL_MS: f64 = 250.0;

/// Times one part of the step; a slow one logs when it ends.
pub(crate) struct StallGuard(std::time::Instant, &'static str, Option<f64>);

impl StallGuard {
    pub(crate) fn new(part: &'static str) -> Self {
        Self(std::time::Instant::now(), part, diag::thread_cpu_ms())
    }
}

impl Drop for StallGuard {
    fn drop(&mut self) {
        let ms = self.0.elapsed().as_secs_f64() * 1000.0;
        if ms > STALL_MS {
            diag::warn!(Sim, "bo2mp stall: {} took {ms:.0} ms{}", self.1, diag::busy_note(self.2));
        }
    }
}

pub(crate) fn schedule() -> Schedule {
    let mut schedule = Schedule::default();
    schedule.add_systems(
        (
            advance_time_system,
            expire_transient_events_system,
            crate::script::apply_disconnects,
            crate::script::advance_mechanics,
            crate::script::advance_scheduler,
            crate::t6::advance,
            (
                apply_script_signals_system,
                apply_actions_system,
                crate::script::sync_players,
                crate::script::sync_presence,
                run_players_system,
                record_collision_state_system,
                run_entity_types_system,
                dispatch_touches_system,
                crate::t6::collect_item_pickups,
                crate::script::sync_engine_events,
                finalize_system,
                publish_snapshot_system,
            )
                .chain()
                .run_if(crate::script::healthy),
        )
            .chain(),
    );
    schedule
}

pub(crate) fn run_schedule(
    ecs: &mut World,
    schedule: &mut Schedule,
    tick: Tick,
    input: &TickInput,
    msec: i32,
    reason: crate::StepReason,
) -> Result<Snapshot, crate::script::Fault> {
    crate::script::preflight(ecs, tick, reason)?;
    assert!(
        !ecs.contains_resource::<StepRequest>(),
        "simulation schedule already has a pending frame"
    );
    ecs.insert_resource(StepRequest {
        tick,
        input: input.clone(),
        msec: msec.clamp(1, 200),
        reason,
        output: None,
    });
    schedule.run(ecs);
    let request = ecs
        .remove_resource::<StepRequest>()
        .expect("simulation request missing");
    if let Some(fault) = ecs.resource::<crate::script::Runtime>().fault.as_ref() {
        return Err(fault.clone());
    }
    Ok(request
        .output
        .expect("simulation schedule did not publish a snapshot"))
}

fn frame_world(world: &mut World) -> FrameWorld<'_> {
    crate::frame::FrameWorld::from_world(world)
}

fn apply_script_signals_system(world: &mut World) {
    let signals = crate::script::take_signals(world);
    if signals.is_empty() {
        return;
    }
    let mut frame = frame_world(world);
    for signal in signals {
        match &*signal {
            crate::script::EXIT_LEVEL
                if !matches!(
                    frame.phase(),
                    MatchPhase::Intermission | MatchPhase::PostGame
                ) =>
            {
                frame.set_phase(MatchPhase::Intermission);
            }
            crate::script::MAP_RESTART => {
                let tick = frame.ecs().resource::<StepRequest>().tick;
                crate::script::restart_level(frame.ecs(), tick);
            }
            name => crate::script::host::match_start::apply_level_notify(&mut frame, name),
        }
    }
}

pub fn phase_materialize_entity_dobjs(world: &mut SimState) {
    for capabilities in world.entity_collision_capabilities_mut() {
        if let Some(dobj) = &mut capabilities.dobj {
            dobj.materialize();
        }
    }
}

fn phase_animated_map_models(world: &mut FrameWorld, tick: Tick, msec: i32) {
    let dt = msec as f32 / 1000.0;

    let at_time = i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
    let mut mover_apos = Vec::new();
    world.visit_script_movers(|mover| {
        mover_apos.push((
            mover.id,
            crate::gentity::apos_from_entity_state(&mover.state),
        ));
    });
    for capabilities in world.entity_collision_capabilities_mut() {
        let Some(dobj) = capabilities.dobj.as_mut() else {
            continue;
        };
        if dobj.play_anim.is_some() {
            dobj.advance_script_model_play_anim(dt);
        }
        if let Some(id) = capabilities.owner.script_model() {
            if let Some((_, apos)) = mover_apos.iter().find(|(mover_id, _)| *mover_id == id) {
                dobj.apply_trajectory_apos_at(*apos, at_time);
                continue;
            }
        }
        if dobj.apos.is_some() {
            dobj.apply_apos_at(at_time);
        }
    }
}

fn phase_stuck_in_client(world: &mut FrameWorld) {
    let mut ids: Vec<ClientId> = world.client_ids_sorted();
    ids.retain(|id| {
        world
            .client_meta(*id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
            && world.player(*id).is_some()
    });
    let mut rows: Vec<gamemode_t6::StuckClient> = ids
        .iter()
        .filter_map(|id| world.player(*id).map(stuck_row))
        .collect();
    let mut pairs = Vec::new();
    let eject_speed = gamemode_t6::G_PLAYER_COLLISION_EJECT_SPEED_DEFAULT;
    let mut holdrand = *world.stuck_holdrand_mut();
    for i in 0..rows.len() {
        if let Some(pair) = gamemode_t6::stuck_in_client(i, &mut rows, eject_speed, &mut holdrand)
        {
            pairs.push((ids[pair.self_idx], ids[pair.other_idx]));
        }
    }
    *world.stuck_holdrand_mut() = holdrand;
    for (id, row) in ids.iter().zip(rows.iter()) {
        if let Some(ps) = world.player_mut(*id) {
            ps.velocity[0] = row.velocity[0];
            ps.velocity[1] = row.velocity[1];
            ps.pm_time = row.pm_time;
            ps.pm_flags = row.pm_flags;
        }
    }
    world.set_stuck_ejects(pairs);
}

fn stuck_row(ps: &PlayerState) -> gamemode_t6::StuckClient {
    gamemode_t6::StuckClient {
        origin: ps.origin,
        velocity: ps.velocity,
        speed: ps.speed,
        // QUESTIONS(Engine): sessionState and the entity contents are not on PlayerState;
        // only alive rows reach here, and a live player's body is CONTENTS_BODY.
        player: ps.other_flags & playerstate_t6::game::other_flags::PLAYER != 0,
        playing: true,
        contents: crate::world::CONTENTS_BODY,
        health: ps.health,
        pm_time: ps.pm_time,
        pm_flags: ps.pm_flags,
        maxs_x: PLAYER_MAXS[0],
        bounds_mid: [
            ps.origin[0] + (PLAYER_MINS[0] + PLAYER_MAXS[0]) * 0.5,
            ps.origin[1] + (PLAYER_MINS[1] + PLAYER_MAXS[1]) * 0.5,
            ps.origin[2] + (PLAYER_MINS[2] + PLAYER_MAXS[2]) * 0.5,
        ],
        bounds_half: [
            (PLAYER_MAXS[0] - PLAYER_MINS[0]) * 0.5,
            (PLAYER_MAXS[1] - PLAYER_MINS[1]) * 0.5,
            (PLAYER_MAXS[2] - PLAYER_MINS[2]) * 0.5,
        ],
    }
}

fn advance_time_system(world: &mut World) {
    let tick;
    {
        let mut request = world.resource_mut::<StepRequest>();
        request.input.canonicalize();
        tick = request.tick;
    }
    let mut frame = frame_world(world);
    frame.mark_running();
    frame.begin_entity_frame(tick);
}

fn expire_transient_events_system(world: &mut World) {
    let tick = world.resource::<StepRequest>().tick;
    let mut frame = frame_world(world);
    frame.enter_kernel_phase(crate::gentity::KernelPhase::ExpireTransientEvents);
    frame.expire_dying_missiles(tick);
    frame.clear_tick_events(tick);
}

fn apply_actions_system(world: &mut World) {
    let tick = world.resource::<StepRequest>().tick;
    let actions = world.resource::<StepRequest>().input.actions.clone();
    let mut frame = frame_world(world);
    for (id, action) in &actions {
        if action_needs_player_row(action) {
            let _ = frame.ensure_player(*id);
        } else {
            let _ = frame.client_meta_mut(*id);
        }
    }

    frame.enter_kernel_phase(crate::gentity::KernelPhase::ApplyActions);
    apply_actions(&mut frame, tick, &actions);
}

fn run_players_system(ecs: &mut World) {
    let _stall = StallGuard::new("run_players");
    let tick = ecs.resource::<StepRequest>().tick;
    let level_time = crate::level_time_ms(tick);
    let input = ecs.resource::<StepRequest>().input.clone();
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(crate::gentity::KernelPhase::RunPlayers);

    let allow_move = matches!(world.phase(), MatchPhase::Playing | MatchPhase::Warmup);
    world.prune_mantle_blockers(level_time);
    let content = world.content();
    let cmodel_models = &content.clip_cmodels().models;
    let (brushes, bsp, mesh) = (
        content.clip_brushes(),
        content.clip_bsp(),
        content.clip_mesh(),
    );
    // bo2mp: Black Ops II's movement runs on its own collision when the map
    // carries its brushes' corner points (a Black Ops II map).
    let gjk = brushes.iter().any(|b| !b.verts.is_empty());
    // bo2mp: the script movers by number, for the platforms players ride.
    let movers = crate::frame::collect_script_movers(world.ecs());
    let mover_numbers: std::collections::BTreeMap<_, u16> = movers
        .iter()
        .map(|mover| (mover.id, mover.state.number as u16))
        .collect();
    let original_buttons = world.old_buttons_mut().clone();
    let original_angles = world.old_cmd_angles_mut().clone();
    let original_moves = world.old_cmd_moves_mut().clone();
    let mut consumed = Vec::new();
    if allow_move {
        phase_materialize_entity_dobjs(&mut world);
        for (id, cmd) in &input.cmds {
            world.select_lagcomp_command(*id, cmd.server_time);
            if !world
                .client_meta(*id)
                .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
            {
                continue;
            }
            let Some(ps) = world.player(*id) else {
                continue;
            };

            let delta = cmd.server_time.wrapping_sub(ps.command_time);
            if delta <= 0 {
                continue;
            }

            let old_buttons = world
                .old_buttons_mut()
                .iter()
                .find(|(c, _)| c == id)
                .map_or(0, |(_, buttons)| *buttons);
            let Some(ps) = world.player(*id).copied() else {
                continue;
            };
            let weapon = ps.weapon;
            let scales = world.scales_for(weapon);
            let facts = world.combat_facts_for(weapon);
            let aim_down_sight = facts.is_some_and(|f| f.aim_down_sight);
            let overlay_reticle = facts.map(|f| f.overlay_reticle).unwrap_or(0);
            let (ads_in_rate, ads_out_rate, rechamber_while_ads, ads_fire_only) = facts
                .map(|f| f.ads_frac_context())
                .unwrap_or((1.0 / 200.0, 1.0 / 200.0, true, false));
            let ads_reload = facts
                .map(|f| f.ads_reload_rule())
                .unwrap_or((false, 0, false));
            let predicting = !world.publishes_snapshot();
            let sent = world.feel();
            let mut feel = bo2_feel(world.ecs(), predicting, sent);
            if !predicting {
                // Every snapshot carries the rules to the other PCs.
                world.set_feel(feel);
            }
            if let Some(f) = facts {
                // The gun in hand sets its own footstep rhythm scales.
                feel.sprint_cycle_scale = f.sprint_cycle_scale;
                feel.ducked_sprint_cycle_scale = f.ducked_sprint_cycle_scale;
                feel.dtp_cycle_scale = f.dtp_cycle_scale;
            }
            // bo2mp: Black Ops II's own PM_IsAdsAllowed, from the gun (no MW2 rules).
            let bo2_ads = crate::t6_gun::ads_allowed(&world, *id, &ps);
            let ads_allowed = bo2_ads.unwrap_or_else(|| {
                facts.is_some_and(|f| {
                    weapon != 0 && {
                        let mut allow = f.ads_allow_facts();

                        allow.clip_index = weapon_t6::ammo_table::clip_table_key(allow.clip_index, weapon);
                        let player = weapon_t6::ads_allow::AdsAllowPlayer {
                            last_weapon_hand: ps.last_weapon_hand,
                            e_flags: ps.e_flags,
                            pm_type: ps.pm_type,
                            other_flags: ps.other_flags,
                            pm_flags: ps.pm_flags,
                            weap_flags: ps.weap_flags,
                            weapon_state_t6: ps.weapon_state_t6,
                        };
                        weapon_t6::ads_allow::is_ads_allowed_bo2(&player, &allow, &ps.ammoclip, ps.last_weapon_hand.min(1) as u8)
                    }
                })
            });
            let (ads_in_rate, ads_out_rate) = movement_t6::quickdraw_ads_rates(
                &ps,
                ads_in_rate,
                ads_out_rate,
                // bo2mp: BO2's Quickdraw skips the sniper anim sets, not the sniper class.
                facts.map_or(0, |f| f.player_anim_type),
                feel,
            );
            let (melee_delay_ms, melee_charge_delay_ms) = facts
                .map(|f| (f.melee_delay_ms, f.melee_charge_delay_ms))
                .unwrap_or((0, 0));
            let mut context = pmove_context(
                old_buttons,
                scales,
                ads_allowed,
                aim_down_sight,
                ads_in_rate,
                ads_out_rate,
                rechamber_while_ads,
                ads_reload,
                ads_fire_only,
                melee_delay_ms,
                melee_charge_delay_ms,
                overlay_reticle,
                world
                    .client_meta(*id)
                    .and_then(|m| m.shellshock.as_ref())
                    .map_or(1.0, |shock| {
                        // BO2 shocks carry no walk switch: their scale goes
                        // through the move speed multiplier (t6 natives_player).
                        movement_t6::shellshock_walk_scale(false, shock.movement_scale)
                    }),
                feel,
            );
            // bo2mp: the Hybrid Optic's swap key is the sprint key; tapping it while aimed
            // must not drop ADS (it only swaps the sight).
            context.ads_intent.sprint_hold_ads = facts.is_some_and(|f| f.optic_switch);
            // bo2mp: the T6 brain lerps the sights (t6_gun::advance).
            context.ads_frac.weapon_brain_owns = !world.gun_facts().is_empty();
            // bo2mp: Black Ops II's BG_WeaponBlocksProne: the gun in hand can forbid
            // going prone, the dive and sprint-to-prone (its WeaponDef blocksProne).
            context.weapon_blocks_prone = world.gun_facts().get(weapon).moves.blocks_prone;
            // bo2mp: the knife lunge is Black Ops II's (t6_lunge).
            if context.ads_frac.weapon_brain_owns {
                context.walk.lunge = Some(crate::t6_lunge::move_lunge(&world, *id, tick, feel.zombies));
            }
            // bo2mp: in a scorestreak vehicle his view keeps to its limits.
            if let Some([up, down]) = world.client_meta(*id).and_then(|m| m.view_pitch_clamp) {
                context.view_angles.pitch_up = up;
                context.view_angles.pitch_down = down;
            }
            crate::script::player_commands(world.ecs(), id.0, cmd.buttons, old_buttons);
            let mut cmd = *cmd;
            crate::script_player::constrain_cmd(&mut world, *id, &mut cmd, feel.zombies);
            crate::script::select_location(world.ecs(), id.0, &mut cmd, old_buttons);
            if world
                .client_meta(*id)
                .is_some_and(|m| m.remote_missile.is_some())
            {
                crate::remote_missile::steer(&mut world, *id, &cmd, delta.min(200));
                cmd.forwardmove = 0;
                cmd.rightmove = 0;
                cmd.buttons &= playerstate_t6::game::buttons::CROUCH | playerstate_t6::game::buttons::PRONE;
            }
            // bo2mp: Black Ops II's PmoveSingle stops a frozen player dead every frame
            // (freezecontrols; PM_ClearVelocityForPmType), in the air too.
            let frozen = world.client_meta(*id).is_some_and(|m| m.controls.frozen);
            let commanded_move = cmd.forwardmove != 0 || cmd.rightmove != 0;
            // bo2zm: the keys as sent, before the move holds any back.
            let sent_move = [cmd.forwardmove, cmd.rightmove];
            context.old_move = world.old_cmd_move(*id).unwrap_or([0, 0]);
            // bo2mp: each brush model with its mover's entity number, so a
            // hit on it names the mover (the platform the player rides), and
            // whose it is, for the touches the move makes.
            let (linked_brushes, linked_owners): (Vec<(u16, LinkedBrushCollisionBrush)>, Vec<_>) = world
                .entity_collision_capabilities()
                .iter()
                .flat_map(|c| {
                    let entnum = c
                        .owner
                        .script_model()
                        .and_then(|id| mover_numbers.get(&id).copied())
                        .unwrap_or(trace_t6::ENTITYNUM_WORLD);
                    c.solid_brushes().iter().map(move |b| ((entnum, b.clone()), c.owner))
                })
                .unzip();
            let struck = RefCell::new(Vec::new());
            let touched = RefCell::new(Vec::new());
            let bodies = alive_body_clips(&world);
            let blockers = oriented_blocker_brushes(&world);
            let glass_damage = world.world_objects().glass_damage_pairs();
            let backend = ClipBackend {
                brushes: &brushes,
                bsp: &bsp,
                mesh: &mesh,
                glass_damage: &glass_damage,
                bodies: &bodies,
                self_entnum: id.0 as u16,
                cmodels: &cmodel_models,
                linked_brushes: &linked_brushes,
                linked_owners: &linked_owners,
                struck: Some(&struck),
                touched: Some(&touched),
                blockers: &blockers,
                movers: &movers,
                vehicles: &[],
                gjk,
            };
            let mantle = world.xanims();
            let (
                walking,
                linked_bounds,
                anim_movetype,
                moved_from,
                moved_to,
                view_change,
                mantle_landing,
                body_anims,
            ) = {
                let ps = world
                    .player_mut(*id)
                    .expect("Alive client has a player row");
                let was_mantling = ps.pm_flags & playerstate_t6::game::pm_flags::MANTLE != 0;

                if ps.shellshock_time.wrapping_add(ps.shellshock_duration) < level_time {
                    ps.pm_flags &= !playerstate_t6::game::pm_flags::SHELLSHOCKED;
                }
                // bo2zm: BO2 sets the walk speed every frame (move-faster +7%).
                ps.speed = movement_t6::bridge::player_speed(ps, feel);
                // bo2mp: bg_gravity is everyone's gravity (the mod menu's
                // Low Gravity sets it).
                if !feel.zombies {
                    ps.gravity = feel.gravity;
                }
                if frozen {
                    ps.velocity = [0.0; 3];
                }
                let moved_from = ps.origin;
                // bo2mp: the player always moves with Black Ops II's own
                // frame (movement_t6).
                let (result, body_anims) = movement_t6::pmove_with_body(
                    ps,
                    &mut cmd,
                    context,
                    &backend,
                    mantle.as_ref(),
                    mantle.as_ref(),
                    !predicting,
                );
                let pml = result.pml;
                let anim_movetype = footsteps_anim_move_type(
                    ps,
                    cmd.forwardmove,
                    cmd.rightmove,
                    pml.almost_ground_plane != 0,
                );
                let moved_to = ps.origin;
                // bo2mp: a climb that started this move (pm.mantleStarted):
                // where it ends and how long it takes.
                let mantle_landing = (!was_mantling
                    && ps.pm_flags & playerstate_t6::game::pm_flags::MANTLE != 0)
                    .then(|| {
                        let tree = mantle.as_ref();
                        let duration = movement_t6::bridge::mantle::duration(ps, tree);
                        let now = movement_t6::bridge::mantle::sample_root_track(ps, ps.mantle_state.timer, tree, tree);
                        let end = movement_t6::bridge::mantle::sample_root_track(ps, duration, tree, tree);
                        ([0, 1, 2].map(|i| ps.origin[i] + end[i] - now[i]), duration)
                    });
                (
                    pml.walking as i32,
                    result.bounds,
                    anim_movetype,
                    moved_from,
                    moved_to,
                    result.view_change,
                    mantle_landing,
                    body_anims,
                )
            };
            // bo2mp: BO2's movement's own jump and land for the legs.
            crate::t6::note_body_anims(world.ecs(), id.0, body_anims);
            world
                .client_meta_mut(*id)
                .input_receipt
                .record(commanded_move, moved_from, moved_to);
            // bo2mp: what his move touched, for the server's "touch"
            // notifies (G_TouchEnts after his Pmove).
            let touched = touched.take();
            if !touched.is_empty() && world.publishes_snapshot() {
                world
                    .ecs()
                    .get_resource_or_insert_with(PlayerModelTouches::default)
                    .0
                    .extend(touched.into_iter().map(|owner| (id.0, owner)));
            }
            if let Some(movetype) = anim_movetype {
                world.set_last_anim_movetype(*id, movetype);
            }
            world.set_pmove_walking(*id, walking);
            world.set_pmove_view_change(*id, view_change); // bo2zm
            world.link_player_area(*id, linked_bounds);
            // bo2mp: Black Ops II's ClientThink after a climb starts
            // (G_AddPlayerMantleBlockage): the spot he lands on is his until
            // g_mantleBlockTimeBuffer after the climb ends.
            if let Some((origin, duration)) = mantle_landing {
                world.add_mantle_blocker(crate::world::MantleBlocker {
                    owner: id.0 as u16,
                    origin,
                    ends: level_time + duration,
                    until: level_time + duration + crate::world::MANTLE_BLOCK_TIME_BUFFER_MS,
                });
            }

            let shots = advance_weapon_command(&mut world, tick, *id, cmd, delta.min(200));
            for shot in shots {
                crate::missile::fire_accepted_shot(&mut world, tick, &shot);

                let hitbox_ids = [*id];
                let hitboxes = if world.publishes_snapshot() {
                    None
                } else {
                    Some(hitbox_ids.as_slice())
                };
                world.record_collision_history(tick, 0, hitboxes);
                if world.publishes_snapshot() {
                    world.record_entity_collision_history(tick);
                }
                let emissions = phase_emit(&world, core::slice::from_ref(&shot));
                phase_trace(&mut world, tick, &emissions);
            }
            world.set_old_cmd(*id, cmd.buttons, cmd.angles);
            world.set_old_cmd_move(*id, sent_move);
            consumed.push((*id, cmd));
        }
    }

    *world.old_buttons_mut() = original_buttons;
    *world.old_cmd_angles_mut() = original_angles;
    *world.old_cmd_moves_mut() = original_moves;

    restamp_debug_move_look(&mut world, &input.actions);

    if allow_move && world.publishes_snapshot() {
        phase_stuck_in_client(&mut world);
    }
    drop(world);
    let mut request = ecs.resource_mut::<StepRequest>();
    if allow_move {
        request.input.cmds = consumed;
    }
}

fn record_collision_state_system(ecs: &mut World) {
    let _stall = StallGuard::new("record_collision_state");
    let tick = ecs.resource::<StepRequest>().tick;
    let msec = ecs.resource::<StepRequest>().msec;
    let cmd_ids: Vec<crate::world::ClientId> = ecs
        .resource::<StepRequest>()
        .input
        .cmds
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(crate::gentity::KernelPhase::RecordCollisionState);

    let hitbox_cmds = if world.publishes_snapshot() {
        None
    } else {
        Some(cmd_ids)
    };
    world.record_collision_history(tick, msec, hitbox_cmds.as_deref());
}

fn run_entity_types_system(ecs: &mut World) {
    let tick = ecs.resource::<StepRequest>().tick;
    let msec = ecs.resource::<StepRequest>().msec;
    let mut world = frame_world(ecs);
    let allow_move = world.phase() == MatchPhase::Playing;
    world.enter_kernel_phase(crate::gentity::KernelPhase::RunEntityTypes);
    if allow_move {
        {
            let _stall = StallGuard::new("materialize_entity_dobjs");
            phase_materialize_entity_dobjs(&mut world);
        }

        if world.publishes_snapshot() {
            let _stall = StallGuard::new("entity_collision_history");
            world.record_entity_collision_history(tick);
        }

        {
            let _stall = StallGuard::new("remote_missiles");
            crate::remote_missile::advance(&mut world, tick);
        }
        {
            let _stall = StallGuard::new("entity_thinks");
            crate::entity_run::phase_run_entity_thinks(&mut world, tick);
        }
        if world.publishes_snapshot() {
            {
                let _stall = StallGuard::new("glass");
                world.world_objects_mut().glass_update(
                    i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX),
                );
            }
            let _stall = StallGuard::new("animated_map_models");
            phase_animated_map_models(&mut world, tick, msec);
        }
    } else {
        crate::entity_run::phase_walk_entity_thinks(&mut world);
    }
}

fn dispatch_touches_system(ecs: &mut World) {
    let _stall = StallGuard::new("dispatch_touches");
    let tick = ecs.resource::<StepRequest>().tick;
    let input = ecs.resource::<StepRequest>().input.clone();
    let mut world = frame_world(ecs);
    let allow_move = world.phase() == MatchPhase::Playing;
    world.enter_kernel_phase(crate::gentity::KernelPhase::DispatchTouches);
    if allow_move {
        crate::item::phase_touch_items(&mut world, tick);
    }
    let command_buttons: Vec<_> = input
        .cmds
        .iter()
        .map(|(id, cmd)| (id.0, cmd.buttons))
        .collect();
    let latest: std::collections::BTreeMap<_, _> = input.cmds.iter().copied().collect();
    let latest_cmds: Vec<_> = latest.into_iter().collect();
    let cmds: Vec<_> = latest_cmds
        .iter()
        .map(|(id, cmd)| (id.0, cmd.buttons))
        .collect();
    let old: Vec<(u32, u32)> = world
        .old_buttons_mut()
        .iter()
        .map(|(id, bits)| (id.0, *bits))
        .collect();
    let presses = crate::collect_use_presses(&command_buttons, &old);
    world.stamp_use_presses(presses.clone());
    if allow_move && world.publishes_snapshot() {
        crate::item::phase_use_items(&mut world, tick, &presses, &cmds);
    }
}

fn finalize_system(ecs: &mut World) {
    let _stall = StallGuard::new("finalize");
    let tick = ecs.resource::<StepRequest>().tick;
    let input = ecs.resource::<StepRequest>().input.clone();
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(crate::gentity::KernelPhase::Finalize);
    for &(id, cmd) in &input.cmds {
        emit_attack_events(&mut world, tick, &[(id, cmd)]);
        if world.client_meta(id).is_some() {
            world.set_old_cmd(id, cmd.buttons, cmd.angles);
            world.set_old_cmd_move(id, [cmd.forwardmove, cmd.rightmove]);
        }
    }

    let mut old_buttons = std::mem::take(world.old_buttons_mut());
    old_buttons.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_buttons_mut() = old_buttons;
    let mut old_angles = std::mem::take(world.old_cmd_angles_mut());
    old_angles.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_cmd_angles_mut() = old_angles;
    let mut old_moves = std::mem::take(world.old_cmd_moves_mut());
    old_moves.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_cmd_moves_mut() = old_moves;

    harvest_predictable_events(&mut world, tick);
    drown_players(&mut world, tick);
    world.script_gaps_mut().report();
}

fn publish_snapshot_system(ecs: &mut World) {
    let _stall = StallGuard::new("publish_snapshot");
    let tick = ecs.resource::<StepRequest>().tick;
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(crate::gentity::KernelPhase::PublishSnapshot);

    if world.publishes_snapshot() {
        emit_player_ticks(&world, tick);
    }
    let snapshot = if world.publishes_snapshot() {
        let snapshot = world.snapshot(tick);
        emit_snapshot_events(&world, &snapshot);
        snapshot
    } else {
        Snapshot::unpublished(tick)
    };
    drop(world);
    ecs.resource_mut::<StepRequest>().output = Some(snapshot);
}

fn emit_player_ticks(world: &FrameWorld, tick: Tick) {
    let bot_name = entity_t6::client_state::strncpyz::<16>("bot");
    let level_time_ms = crate::level_time_ms(tick);
    let mut rows = Vec::new();
    world.visit_players(|id, ps| {
        rows.push((
            id,
            ps.command_time,
            ps.origin,
            ps.velocity[2],
            ps.viewangles[1],
            ps.viewangles[0],
            ps.jump_time,
            ps.weapon_state_t6,
            ps.legs_anim,
            ps.torso_anim,
        ));
    });
    for (id, time_ms, origin, vz, yaw, pitch, jump_time, weaponstate, legs_anim, torso_anim) in rows
    {
        let Some(meta) = world.client_meta(id) else {
            continue;
        };
        perf::player_tick(
            id.0,
            meta.name == bot_name,
            time_ms,
            level_time_ms,
            origin,
            vz,
            yaw,
            pitch,
            jump_time,
            world.old_buttons(id).unwrap_or(0),
            weaponstate,
            meta.ammo_clip,
            world.pmove_walking(id),
            legs_anim,
            torso_anim,
            lifecycle_label(meta.lifecycle),
        );
    }
}

fn lifecycle_label(life: ClientLifecycle) -> &'static str {
    match life {
        ClientLifecycle::Connecting => "Connecting",
        ClientLifecycle::ChoosingClass => "ChoosingClass",
        ClientLifecycle::SpawnPending => "SpawnPending",
        ClientLifecycle::Alive => "Alive",
        ClientLifecycle::Dead => "Dead",
        ClientLifecycle::RespawnPending => "RespawnPending",
        ClientLifecycle::Spectating => "Spectating",
        ClientLifecycle::Intermission => "Intermission",
    }
}

fn emit_snapshot_events(_world: &FrameWorld, snapshot: &Snapshot) {
    for slot in &snapshot.meta.corpses.slots {
        perf::corpse(
            i64::from(slot.occupied),
            slot.occupied.then_some(slot.origin[2]),
            None,
            None,
        );
    }
    for es in snapshot
        .meta
        .entities
        .iter()
        .filter(|es| es.e_type == entity_t6::et::ITEM)
    {
        let ammo = snapshot
            .meta
            .item_ammo
            .iter()
            .find(|row| row.entnum == es.number);
        perf::item(
            i32::from(es.e_type),
            ammo.map(|row| row.clip_r),
            ammo.map(|row| row.scavenger),
            None,
        );
    }
}

fn harvest_predictable_events(world: &mut FrameWorld, tick: Tick) {
    let mut falls = Vec::new();
    world.for_each_player_mut(|id, ps| {
        let mut cursor = i32::from(ps.predictable_event_sequence_old);
        consume_player_events(ps, &mut cursor, |e| {
            if entity_t6::EntityEventKind(e.event).is_landing_pain() {
                falls.push((id, e.event_parm, ps.max_health, ps.origin));
            }
        });
        // The sequence is masked to 0x7ff, so it fits BO2's i16.
        ps.predictable_event_sequence_old = cursor as i16;
    });
    for (id, parm, max_health, origin) in falls {
        apply_fall_damage(world, tick, id, parm, max_health, origin);
    }
}

/// BO2 feel M12: a landing that hurts (only Black Ops movement sends one)
/// costs parm% of max health, or 110% from a full-height fall, worked out in
/// floats and rounded down (to be checked in real BO2); BO2's own scripts take it as
/// MOD_FALLING through CodeCallback_PlayerDamage.
fn fall_damage(parm: i32, max_health: i32) -> i32 {
    let fraction = if parm < 100 { parm as f32 * 0.01 } else { 1.1 };
    (max_health as f32 * fraction) as i32
}

fn apply_fall_damage(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    parm: i32,
    max_health: i32,
    origin: [f32; 3],
) {
    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        return;
    }
    let amount = fall_damage(parm, max_health);
    if amount <= 0 {
        return;
    }
    let hit = crate::script_player::Hit {
        victim: id,
        attacker: None,
        amount,
        flags: 0,
        means: "MOD_FALLING",
        weapon: 0,
        point: origin,
        dir: [0.0; 3],
        hitloc: 0,
        inflictor: None,
        commit: None,
    };
    crate::script::player_damage(world.ecs(), tick, &hit);
}

/// BO2's breath under water (Player_WaterUpdate, after the move): from water
/// level 5 up (the surface at 54 of the 60 eye height) a player holds their
/// breath for `player_swimTime` (16 s), then loses `player_swimDamage` (15) every
/// `player_swimDamagerInterval` (1 s) until they come up.
const SWIM_TIME_MS: i32 = 16_000;
const SWIM_DAMAGE: i32 = 15;
const SWIM_DAMAGE_INTERVAL_MS: i32 = 1_000;
/// Above this water level the player has no air.
const BREATH_WATER_LEVEL: i32 = 4;
/// BO2 hands the drowning to G_Damage with no armour and no knockback.
const DROWN_DAMAGE_FLAGS: i32 = 2 | 4;

/// When each player last had air (BO2's `outWaterTime`), in level ms.
#[derive(Resource, Default)]
struct Breath(Vec<(ClientId, i32)>);

/// One frame of breath for a player at `waterlevel`: whether they drown a
/// little now. `out` is when they last had air.
fn breath_step(out: &mut i32, waterlevel: i32, now: i32) -> bool {
    if waterlevel <= BREATH_WATER_LEVEL {
        *out = now;
        return false;
    }
    if now - *out > SWIM_TIME_MS {
        *out += SWIM_DAMAGE_INTERVAL_MS;
        return true;
    }
    false
}

fn drown_players(world: &mut FrameWorld, tick: Tick) {
    let now = i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
    let mut players = Vec::new();
    world.for_each_player_mut(|id, ps| players.push((id, ps.waterlevel, ps.origin)));
    let mut drowning = Vec::new();
    {
        let mut breath = world.ecs().get_resource_or_insert_with(Breath::default);
        breath.0.retain(|(id, _)| players.iter().any(|(p, ..)| p == id));
        for &(id, level, origin) in &players {
            let i = match breath.0.iter().position(|(b, _)| *b == id) {
                Some(i) => i,
                None => {
                    breath.0.push((id, now));
                    breath.0.len() - 1
                }
            };
            if breath_step(&mut breath.0[i].1, level, now) {
                drowning.push((id, origin));
            }
        }
    }
    for (id, origin) in drowning {
        if !world
            .client_meta(id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
        {
            continue;
        }
        let hit = crate::script_player::Hit {
            victim: id,
            attacker: None,
            amount: SWIM_DAMAGE,
            flags: DROWN_DAMAGE_FLAGS,
            means: "MOD_DROWN",
            weapon: 0,
            point: origin,
            dir: [0.0, 0.0, -1.0],
            hitloc: 0,
            inflictor: None,
            commit: None,
        };
        crate::script::player_damage(world.ecs(), tick, &hit);
    }
}

fn emit_attack_events(
    world: &mut FrameWorld,
    tick: Tick,
    cmds: &[(ClientId, playerstate_t6::game::UserCmd)],
) {
    for (id, cmd) in cmds {
        if !world
            .client_meta(*id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
        {
            continue;
        }
        let old = world
            .old_buttons_mut()
            .iter()
            .find(|(c, _)| c == id)
            .map_or(0, |(_, b)| *b);
        let attack = cmd.buttons & buttons::ATTACK != 0;
        let was = old & buttons::ATTACK != 0;
        if !attack && was {
            world.push_event(tick, EventAudience::Client(*id), SimEvent::AttackReleased);
        }
    }
}

fn action_needs_player_row(action: &ClientAction) -> bool {
    !matches!(
        action,
        ClientAction::JoinMatch { .. }
            | ClientAction::LeaveMatch { .. }
            | ClientAction::SetName { .. }
            | ClientAction::UseCopycat { .. }
            | ClientAction::ActionSlot { .. }
            | ClientAction::ChooseDefaultClass { .. }
            | ClientAction::MenuResponse { .. }
    )
}

fn apply_actions(world: &mut FrameWorld, tick: Tick, actions: &[(ClientId, ClientAction)]) {
    for (id, action) in actions {
        match *action {
            ClientAction::JoinMatch { request_id: _ } => {
                let meta = world.client_meta_mut(*id);
                if meta.lifecycle == ClientLifecycle::Connecting {
                    meta.lifecycle = ClientLifecycle::ChoosingClass;
                }
            }
            ClientAction::ChooseDefaultClass {
                request_id: _,
                index,
            } => {
                crate::script::answer_join(world.ecs(), id.0);
                choose_bot_class(world, *id, index);
            }
            ClientAction::MenuResponse {
                request_id: _,
                menu,
                response,
            } => {
                let menu = crate::menu_response_text(&menu);
                let response = crate::menu_response_text(&response);
                // A BO2 map's menus answer its own scripts.
                if crate::t6::menu_response(world.ecs(), id.0, menu, response) {
                    continue;
                }
                crate::script::note_team_answer(world.ecs(), id.0, menu);
                if !answer_custom_class(world, *id, menu, response) {
                    crate::script::answer_menu(world.ecs(), id.0, menu, response);
                }
            }
            ClientAction::LeaveMatch { request_id: _ } => {
                world.retire_client(*id);
            }
            ClientAction::SetName {
                request_id: _,
                name,
            } => {
                world.client_meta_mut(*id).name = name;
            }
            ClientAction::UseCopycat { .. } | ClientAction::SpawnClient { .. } => {}
            ClientAction::ActionSlot {
                request_id: _,
                slot,
            } => {
                crate::script::action_slot_command(world.ecs(), id.0, slot);
                crate::t6::action_slot_pressed(world.ecs(), id.0, slot);
            }
            ClientAction::SelectClass {
                request_id,
                class_id,
                revision,
                loadout,
            } => {
                apply_select_class(world, tick, *id, request_id, class_id, revision, loadout);
            }
            ClientAction::GiveWeapon { request_id, weapon } => {
                apply_give_weapon(world, tick, *id, request_id, weapon);
            }
            ClientAction::ChangeWeaponConfiguration {
                request_id,
                from,
                to,
            } => {
                apply_configuration_change(world, tick, *id, request_id, from, to);
            }
            ClientAction::ForceSpawn {
                request_id: _,
                pick,
            } => {
                if !world.bootstrap_ref().allow_debug_actions {
                    continue;
                }
                apply_force_spawn(world, *id, pick);
            }
            ClientAction::SpawnIntermission { request_id: _ } => {
                apply_spawn_intermission(world, *id);
            }
            ClientAction::GiveKillstreak {
                request_id: _,
                name,
            } => {
                if !world.bootstrap_ref().allow_debug_actions {
                    continue;
                }
                let name = crate::menu_response_text(&name);
                if !crate::t6::give_killstreak(world.ecs(), id.0, name) {
                    crate::script::give_killstreak(world.ecs(), id.0, name);
                }
            }
            ClientAction::ForceDeath { request_id: _ } => {
                if !world.bootstrap_ref().allow_debug_actions {
                    continue;
                }
                crate::script::force_death(world.ecs(), tick, id.0);
            }
            ClientAction::SetMatchPhase {
                request_id: _,
                phase,
            } => {
                if !world.bootstrap_ref().allow_debug_actions {
                    continue;
                }
                if phase == MatchPhase::Playing && world.phase() == MatchPhase::Warmup {
                    crate::score::finish_prematch(world);
                } else {
                    world.set_phase(phase);
                }
            }
            ClientAction::Move {
                request_id: _,
                origin,
                angles,
            } => {
                apply_debug_move(world, *id, origin, angles);
            }
            ClientAction::BeginScriptMoverRotateVelocity {
                request_id: _,
                speed,
            } => {
                if !world.bootstrap_ref().allow_debug_actions {
                    continue;
                }
                world.begin_script_movers_rotate_velocity_supplied(
                    speed,
                    crate::corpse::level_time_ms(tick),
                );
            }
            ClientAction::DebugDamage {
                request_id: _,
                amount,
            } => {
                apply_debug_damage(world, tick, *id, amount);
            }
        }
    }
}

fn apply_force_spawn(world: &mut FrameWorld, id: ClientId, pick: crate::SpawnPick) {
    if world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        crate::script_player::move_to_forced_spawn(world, id, pick);
    } else {
        world.client_meta_mut(id).forced_spawn = Some(pick);
    }
}

fn apply_spawn_intermission(world: &mut FrameWorld, id: ClientId) {
    if world
        .client_meta(id)
        .is_none_or(|m| m.lifecycle == ClientLifecycle::Intermission)
    {
        return;
    }
    let view = world.bootstrap_ref().intermission_view.clone();
    {
        let meta = world.client_meta_mut(id);
        meta.lifecycle = ClientLifecycle::Intermission;
        meta.dead_since_tick = None;
    }
    world.unlink_player_area(id);
    let Some(view) = view else {
        return;
    };
    let Some(ps) = world.player_mut(id) else {
        return;
    };
    ps.origin = view.origin;
    ps.velocity = [0.0, 0.0, 0.0];
    ps.viewangles = view.angles;
    ps.delta_angles = packed_look_delta(view.angles);
    ps.e_flags ^= playerstate_t6::game::eflags::TELEPORT;
}

fn apply_debug_move(world: &mut FrameWorld, id: ClientId, origin: [f32; 3], angles: [f32; 3]) {
    if !world.bootstrap_ref().allow_debug_actions {
        return;
    }
    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        return;
    }
    if !origin.iter().chain(angles.iter()).all(|v| v.is_finite()) {
        return;
    }
    let old_origin = {
        let Some(ps) = world.player_mut(id) else {
            return;
        };
        let old_origin = ps.origin;
        ps.origin = origin;
        ps.velocity = [0.0, 0.0, 0.0];
        ps.viewangles = angles;

        ps.e_flags ^= playerstate_t6::game::eflags::TELEPORT;
        ps.delta_angles = packed_look_delta(angles);
        old_origin
    };
    world.translate_player_area(
        id,
        [
            origin[0] - old_origin[0],
            origin[1] - old_origin[1],
            origin[2] - old_origin[2],
        ],
    );
}

fn packed_look_delta(viewangles: [f32; 3]) -> [f32; 3] {
    [
        viewangles[0] - (viewangles[0] * ANGLE2SHORT) as i32 as f32 * SHORT2ANGLE,
        viewangles[1] - (viewangles[1] * ANGLE2SHORT) as i32 as f32 * SHORT2ANGLE,
        viewangles[2] - (viewangles[2] * ANGLE2SHORT) as i32 as f32 * SHORT2ANGLE,
    ]
}

fn restamp_debug_move_look(world: &mut FrameWorld, actions: &[(ClientId, ClientAction)]) {
    for (id, action) in actions {
        let ClientAction::Move { angles, .. } = *action else {
            continue;
        };
        if !world.bootstrap_ref().allow_debug_actions {
            continue;
        }
        if !world
            .client_meta(*id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
        {
            continue;
        }
        let Some(ps) = world.player_mut(*id) else {
            continue;
        };
        ps.viewangles = angles;
        ps.delta_angles = packed_look_delta(angles);
    }
}

fn apply_debug_damage(world: &mut FrameWorld, tick: Tick, id: ClientId, amount: i32) {
    if !world.bootstrap_ref().allow_debug_actions {
        return;
    }
    if amount <= 0 {
        return;
    }
    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        return;
    }
    crate::script_player::debug_damage(world, tick, id, amount);
}

fn configuration_change_ammo(
    clip0: i32,
    clip1: i32,
    stock: i32,
    clip_capacity: i32,
    max_ammo: i32,
    dual: bool,
) -> (i32, i32, i32) {
    let total = clip0
        .max(0)
        .saturating_add(clip1.max(0))
        .saturating_add(stock.max(0));
    let cap = clip_capacity.max(0);
    let first = clip0.max(0).min(cap).min(total);
    let second = if dual {
        clip1.max(0).min(cap).min(total - first)
    } else {
        0
    };
    (first, second, (total - first - second).min(max_ammo.max(0)))
}

fn apply_configuration_change(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    from: u32,
    to: u32,
) {
    let reject = |world: &mut FrameWorld, reason: crate::ConfigurationChangeRejectReason| {
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::ConfigurationChangeRejected {
                request_id,
                from,
                to,
                reason,
            },
        );
    };
    use crate::ConfigurationChangeRejectReason as Reason;
    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        reject(world, Reason::NotAlive);
        return;
    }
    let Some(ps) = world.player(id).copied() else {
        reject(world, Reason::NotAlive);
        return;
    };
    if ps.weapon != from {
        reject(world, Reason::StaleSource);
        return;
    }
    if !world.can_transition_weapon(from, to) {
        reject(world, Reason::DifferentFamily);
        return;
    }
    let (Some(old), Some(new)) = (world.combat_facts_for(from), world.combat_facts_for(to)) else {
        reject(world, Reason::InvalidTarget);
        return;
    };
    if world
        .equipment_facts_for(to)
        .is_some_and(|eq| eq.is_offhand())
    {
        reject(world, Reason::InvalidTarget);
        return;
    }
    // BO2's gun state, ready as our old MW2 mirror read it (TODO(fact)).
    let busy = |s: i32| !weapon_t6::state::is_ready_as_mirrored(s);
    if busy(ps.weapon_state_t6)
        || ps.weapon_time > 0
        || (ps.last_weapon_hand >= 1 && (busy(ps.weapon_state_left_t6) || ps.weapon_time_secondary > 0))
        || ps.weap_flags & playerstate_t6::game::weap_flags::OFFHAND_VIEW != 0
    {
        reject(world, Reason::Busy);
        return;
    }
    let Some(slot) = ps.weapons.iter().position(|&weapon| weapon == from as i32) else {
        reject(world, Reason::NoInventorySlot);
        return;
    };
    if ps.weapons.contains(&(to as i32)) {
        reject(world, Reason::InvalidTarget);
        return;
    }
    let old_ammo_key = weapon_t6::ammo_table::ammo_table_key(old.ammo_index, from);
    let old_clip_key = weapon_t6::ammo_table::clip_table_key(old.clip_index, from);
    let new_ammo_key = weapon_t6::ammo_table::ammo_table_key(new.ammo_index, to);
    let new_clip_key = weapon_t6::ammo_table::clip_table_key(new.clip_index, to);
    let another_owns = |key: i32, clip: bool| {
        ps.weapons
            .iter()
            .filter(|&&weapon| weapon > 0 && weapon != from as i32)
            .any(|&weapon| {
                world.combat_facts_for(weapon as u32).is_some_and(|facts| {
                    key == if clip {
                        weapon_t6::ammo_table::clip_table_key(facts.clip_index, weapon as u32)
                    } else {
                        weapon_t6::ammo_table::ammo_table_key(facts.ammo_index, weapon as u32)
                    }
                })
            })
    };
    let clip0 = weapon_t6::ammo_table::get_clip_for_hand(&ps.ammoclip, old_clip_key, 0);
    let raw_clip1 = weapon_t6::ammo_table::get_clip_for_hand(&ps.ammoclip, old_clip_key, 1);
    let clip1 = if ps.last_weapon_hand >= 1 {
        raw_clip1
    } else {
        0
    };
    let stock = weapon_t6::ammo_table::get_ammo_not_in_clip(&ps.ammo, old_ammo_key);
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(to));
    let (next_clip0, next_clip1, next_stock) =
        configuration_change_ammo(clip0, clip1, stock, new.clip_size, new.max_ammo, akimbo);
    if (old_ammo_key != new_ammo_key
        && (another_owns(old_ammo_key, false) || another_owns(new_ammo_key, false)))
        || (old_clip_key != new_clip_key
            && (another_owns(old_clip_key, true) || another_owns(new_clip_key, true)))
        || (old_ammo_key == new_ammo_key
            && next_stock != stock
            && another_owns(old_ammo_key, false))
        || (old_clip_key == new_clip_key
            && (next_clip0 != clip0 || next_clip1 != raw_clip1)
            && another_owns(old_clip_key, true))
    {
        reject(world, Reason::SharedAmmoConflict);
        return;
    }
    let mut next = ps;
    next.weapons[slot] = to as i32;
    next.weapon_data[slot * 5..slot * 5 + 5].fill(0);
    weapon_t6::ammo_table::latch_weapon_dual_wield(&next.weapons, &mut next.weapon_data, to, akimbo);
    next.weapon = to;
    next.weapon_primary = to;
    next.last_weapon_hand = weapon_t6::ammo_table::num_hands_for_held(&next.weapons, &next.weapon_data, to);
    if old_ammo_key != new_ammo_key {
        for row in next.ammo.chunks_exact_mut(8) {
            if row[..4] == old_ammo_key.to_le_bytes() {
                row.fill(0);
            }
        }
    }
    if old_clip_key != new_clip_key {
        for row in next.ammoclip.chunks_exact_mut(12) {
            if row[..4] == old_clip_key.to_le_bytes() {
                row.fill(0);
            }
        }
    }
    if !weapon_t6::ammo_table::set_ammo_not_in_clip(&mut next.ammo, new_ammo_key, next_stock)
        || !weapon_t6::ammo_table::set_clip_for_hand(&mut next.ammoclip, new_clip_key, 0, next_clip0)
        || !weapon_t6::ammo_table::set_clip_for_hand(&mut next.ammoclip, new_clip_key, 1, next_clip1)
    {
        reject(world, Reason::AmmoTableFull);
        return;
    }
    let alternate_ammo = match (
        world.combat_facts_for(old.alternate_weapon),
        world.combat_facts_for(new.alternate_weapon),
    ) {
        (Some(old_alt), Some(new_alt))
            if old.alternate_weapon != 0
                && new.alternate_weapon != 0
                && old_alt.weap_type == new_alt.weap_type
                && old_alt.weap_class == new_alt.weap_class =>
        {
            let old_key = weapon_t6::ammo_table::ammo_table_key(old_alt.ammo_index, old.alternate_weapon);
            let old_clip = weapon_t6::ammo_table::clip_table_key(old_alt.clip_index, old.alternate_weapon);
            let new_key = weapon_t6::ammo_table::ammo_table_key(new_alt.ammo_index, new.alternate_weapon);
            let new_clip = weapon_t6::ammo_table::clip_table_key(new_alt.clip_index, new.alternate_weapon);
            if old_key != old_ammo_key
                && old_clip != old_clip_key
                && new_key != new_ammo_key
                && new_clip != new_clip_key
                && weapon_t6::ammo_table::ammo_row_present(&ps.ammo, old_key)
                && weapon_t6::ammo_table::clip_row_present(&ps.ammoclip, old_clip)
            {
                let shared = ps
                    .weapons
                    .iter()
                    .filter(|&&w| w > 0 && w != from as i32)
                    .any(|&w| {
                        let alt = world
                            .combat_facts_for(w as u32)
                            .map_or(0, |f| f.alternate_weapon);
                        [w as u32, alt].into_iter().filter(|&w| w != 0).any(|w| {
                            world.combat_facts_for(w).is_some_and(|f| {
                                let ammo = weapon_t6::ammo_table::ammo_table_key(f.ammo_index, w);
                                let clip = weapon_t6::ammo_table::clip_table_key(f.clip_index, w);
                                (old_key != new_key && (ammo == old_key || ammo == new_key))
                                    || (old_clip != new_clip
                                        && (clip == old_clip || clip == new_clip))
                            })
                        })
                    });
                if shared {
                    reject(world, Reason::SharedAmmoConflict);
                    return;
                }
                let (clip, _, stock) = configuration_change_ammo(
                    weapon_t6::ammo_table::get_clip_for_hand(&ps.ammoclip, old_clip, 0),
                    0,
                    weapon_t6::ammo_table::get_ammo_not_in_clip(&ps.ammo, old_key),
                    new_alt.clip_size,
                    new_alt.max_ammo,
                    false,
                );
                if old_key != new_key {
                    for row in next.ammo.chunks_exact_mut(8) {
                        if row[..4] == old_key.to_le_bytes() {
                            row.fill(0);
                        }
                    }
                }
                if old_clip != new_clip {
                    for row in next.ammoclip.chunks_exact_mut(12) {
                        if row[..4] == old_clip.to_le_bytes() {
                            row.fill(0);
                        }
                    }
                }
                if !weapon_t6::ammo_table::set_ammo_not_in_clip(&mut next.ammo, new_key, stock)
                    || !weapon_t6::ammo_table::set_clip_for_hand(&mut next.ammoclip, new_clip, 0, clip)
                {
                    reject(world, Reason::AmmoTableFull);
                    return;
                }
                Some((new.alternate_weapon, clip, stock))
            } else {
                None
            }
        }
        _ => None,
    };
    let hand = weapon_t6::combat_facts::spawn_weapon_hand(to, &new);
    next.weaponstate_primary = hand.weaponstate;
    next.weapon_time = hand.weapon_time;
    next.weapon_delay = hand.weapon_delay;
    next.weap_anim = hand.weap_anim;
    next.weaponstate_secondary = hand.weaponstate;
    next.weapon_time_secondary = hand.weapon_time;
    next.weapon_delay_secondary = hand.weapon_delay;
    next.weap_anim_secondary = hand.weap_anim;
    next.weapon_shot_count = 0;
    next.weapon_shot_count_secondary = 0;
    next.weap_flags &= !(playerstate_t6::game::weap_flags::NO_ADS
        | playerstate_t6::game::weap_flags::DOUBLEBARREL_RECOIL
        | playerstate_t6::game::weap_flags::RECOIL_SCALE);
    *world.player_mut(id).expect("validated alive player") = next;
    let meta = world.client_meta_mut(id);
    if let Some((alternate, clip, stock)) = alternate_ammo {
        meta.ammo_by_weapon
            .retain(|row| row.0 != old.alternate_weapon);
        meta.set_ammo(alternate, clip, stock);
    }
    let quick_reload_ready = meta.quick_reload_ready(from);
    meta.ammo_by_weapon.retain(|row| row.0 != from);
    meta.set_ammo(to, next_clip0, next_stock);
    meta.set_quick_reload_ready(from, true);
    meta.set_quick_reload_ready(to, quick_reload_ready);
    meta.mirror_held_ammo(to);
    meta.weapon_shot_count = 0;
    meta.burst_latch = false;
    meta.burst_latch_secondary = false;
    meta.rechamber_pending = false;
    meta.rechamber_pending_secondary = false;
    meta.t6_gun = None;
    world.push_event(
        tick,
        EventAudience::Client(id),
        SimEvent::ConfigurationChangeAccepted {
            request_id,
            from,
            to,
        },
    );
}

fn apply_give_weapon(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    weapon: u32,
) {
    let reject = |world: &mut FrameWorld, reason: crate::GiveRejectReason| {
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::GiveRejected {
                request_id,
                weapon,
                reason,
            },
        );
    };

    if !world
        .client_meta(id)
        .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        reject(world, crate::GiveRejectReason::NotAlive);
        return;
    }
    if weapon == 0 {
        reject(world, crate::GiveRejectReason::InvalidWeapon);
        return;
    }
    let table_len = world.weapon_combat_len();
    if (weapon as usize) >= table_len {
        reject(world, crate::GiveRejectReason::UnknownWeaponId);
        return;
    }
    if !world.weapon_runnable(weapon) {
        reject(world, crate::GiveRejectReason::UnsupportedWeapon);
        return;
    }
    let Some(facts) = world.combat_facts_for(weapon) else {
        reject(world, crate::GiveRejectReason::UnknownWeaponId);
        return;
    };
    if world
        .equipment_facts_for(weapon)
        .is_some_and(|eq| eq.is_offhand())
    {
        apply_give_offhand(world, tick, id, request_id, weapon, &facts);
        return;
    }
    if facts.fire_time_ms <= 0 && facts.raise_time_ms <= 0 {
        reject(world, crate::GiveRejectReason::EmptyCombatProfile);
        return;
    }

    if world.weapon_script_name(weapon).starts_with("killstreak_") {
        let Some(ps) = world.player_mut(id) else {
            reject(world, crate::GiveRejectReason::NotAlive);
            return;
        };
        inventory_add_weapon(ps, weapon, false);
        if !ps.weapons.contains(&(weapon as i32)) {
            reject(world, crate::GiveRejectReason::InvalidWeapon);
            return;
        }
        let (clip, clip_alt, stock) = weapon_t6::combat_facts::spawn_clip_stock(&facts, 0);
        seed_ps_ammo_tables(ps, weapon, &facts, clip, clip_alt, false, stock);
        world.client_meta_mut(id).set_ammo(weapon, clip, stock);
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::GiveAccepted { request_id, weapon },
        );
        return;
    }
    let akimbo = gsc_give_weapon_is_akimbo(world.weapon_script_name(weapon));
    let Some(mut next) = world.player(id).copied() else {
        reject(world, crate::GiveRejectReason::NotAlive);
        return;
    };

    let outgoing = if world
        .combat_facts_for(next.weapon)
        .is_some_and(|facts| facts.inventory_type == 3)
        && next.weapons.contains(&(next.weapon_primary as i32))
    {
        next.weapon_primary
    } else {
        next.weapon
    };
    let mut replaced = false;
    if !next.weapons.contains(&(weapon as i32)) {
        if let Some(slot) = next
            .weapons
            .iter()
            .position(|&w| w != 0 && w == outgoing as i32)
        {
            next.weapons[slot] = weapon as i32;
            next.weapon_data[slot * 5..slot * 5 + 5].fill(0);
            replaced = true;
        }
    }
    if replaced {
        let outgoing_facts = world
            .combat_facts_for(outgoing)
            .expect("held weapon combat facts");

        for (table, stride, clip) in [
            (&mut next.ammo[..], 8, false),
            (&mut next.ammoclip[..], 12, true),
        ] {
            let key = if clip {
                weapon_t6::ammo_table::clip_table_key(outgoing_facts.clip_index, outgoing)
            } else {
                weapon_t6::ammo_table::ammo_table_key(outgoing_facts.ammo_index, outgoing)
            };
            let owned = next.weapons.iter().filter(|&&w| w > 0).any(|&w| {
                world.combat_facts_for(w as u32).is_some_and(|f| {
                    key == if clip {
                        weapon_t6::ammo_table::clip_table_key(f.clip_index, w as u32)
                    } else {
                        weapon_t6::ammo_table::ammo_table_key(f.ammo_index, w as u32)
                    }
                })
            });
            if !owned {
                for row in table.chunks_exact_mut(stride) {
                    if row[..4] == key.to_le_bytes() {
                        row.fill(0);
                    }
                }
            }
        }
    }
    give_weapon_to_ps_akimbo(&mut next, weapon, akimbo);
    *world.player_mut(id).expect("validated alive player") = next;
    let (clip, stock) = if let Some(ps_mut) = world.player_mut(id) {
        arm_held_weapon(ps_mut, weapon, &facts)
    } else {
        (0, 0)
    };
    let meta = world.client_meta_mut(id);
    if replaced {
        meta.ammo_by_weapon.retain(|row| row.0 != outgoing);
        meta.set_quick_reload_ready(outgoing, true);
    }
    meta.set_ammo(weapon, clip, stock);
    meta.set_quick_reload_ready(weapon, true);
    meta.mirror_held_ammo(weapon);
    meta.weapon_shot_count = 0;
    meta.burst_latch = false;
    meta.burst_latch_secondary = false;
    meta.rechamber_pending = false;
    meta.rechamber_pending_secondary = false;
    meta.t6_gun = None;
    world.push_event(
        tick,
        EventAudience::Client(id),
        SimEvent::GiveAccepted { request_id, weapon },
    );
}

fn apply_give_offhand(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    weapon: u32,
    facts: &weapon_t6::combat_facts::WeaponCombatFacts,
) {
    let reject = |world: &mut FrameWorld, reason: crate::GiveRejectReason| {
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::GiveRejected {
                request_id,
                weapon,
                reason,
            },
        );
    };
    let Some(eq) = world.equipment_facts_for(weapon) else {
        reject(world, crate::GiveRejectReason::InvalidWeapon);
        return;
    };
    let Some(mut next) = world.player(id).copied() else {
        reject(world, crate::GiveRejectReason::NotAlive);
        return;
    };
    for slot in &mut next.weapons {
        if *slot > 0 && *slot != weapon as i32 {
            let same_slot = world
                .equipment_facts_for(*slot as u32)
                .is_some_and(|other| {
                    (matches!(eq.offhand_class, 1 | 4 | 5)
                        && matches!(other.offhand_class, 1 | 4 | 5))
                        || (matches!(eq.offhand_class, 2 | 3)
                            && matches!(other.offhand_class, 2 | 3))
                });
            if same_slot {
                *slot = 0;
            }
        }
    }
    inventory_add_weapon(&mut next, weapon, false);
    if !next.weapons.contains(&(weapon as i32)) {
        reject(world, crate::GiveRejectReason::InvalidWeapon);
        return;
    }
    match eq.offhand_class {
        1 | 4 | 5 => next.offhand_primary = eq.offhand_class,
        2 | 3 => next.offhand_secondary = eq.offhand_class,
        _ => {}
    }
    let clip = eq.spawn_clip_count();
    seed_ps_ammo_tables(&mut next, weapon, facts, clip, 0, false, 0);
    *world.player_mut(id).expect("validated alive player") = next;
    {
        let meta = world.client_meta_mut(id);
        meta.ammo_by_weapon
            .retain(|(id, _, _)| next.weapons.contains(&(*id as i32)));
        meta.set_ammo(weapon, clip, 0);
        if let Some(loadout) = meta.loadout.as_mut() {
            match eq.offhand_class {
                1 | 4 | 5 => loadout.lethal = weapon,
                2 | 3 => loadout.tactical = weapon,
                _ => {}
            }
        }
    }
    world.push_event(
        tick,
        EventAudience::Client(id),
        SimEvent::GiveAccepted { request_id, weapon },
    );
}

fn choose_bot_class(world: &mut FrameWorld, id: ClientId, index: u8) {
    let t5 = crate::script::is_t5(world.ecs());
    let bot_classes = &world.bootstrap_ref().bot_classes;
    let def = (!bot_classes.is_empty() && !t5)
        .then(|| bot_classes[index as usize % bot_classes.len()].clone())
        .filter(|def| !def.locked && validate_class_content(world, def).is_ok());
    match def {
        Some(def) => crate::script::choose_class(world.ecs(), id.0, &def),
        None => crate::script::choose_default_class(world.ecs(), id.0, index),
    }
}

fn answer_custom_class(world: &mut FrameWorld, id: ClientId, menu: &str, response: &str) -> bool {
    if !menu.eq_ignore_ascii_case("changeclass") {
        return false;
    }
    let Some(slot) = response
        .strip_prefix("custom")
        .and_then(|n| n.parse::<u32>().ok())
        .and_then(|n| n.checked_sub(1))
    else {
        return false;
    };
    let Some(def) = crate::script::personal_class(world.ecs(), id.0, crate::ClassId(slot)) else {
        return true;
    };
    if validate_class_content(world, &def).is_err() {
        return true;
    }
    crate::script::choose_class(world.ecs(), id.0, &def);
    true
}

fn apply_select_class(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    request_id: u32,
    class_id: crate::ClassId,
    revision: u32,
    loadout: crate::PersonalClass,
) {
    let accepted = (class_id.0 < crate::match_state::PERSONAL_CLASS_SLOTS as u32)
        .then(|| loadout.definition(class_id, revision))
        .flatten();
    let Some(def) = accepted else {
        diag::info!(
            Sim,
            "class select: rejected client={} id={} rev={} reason=unknown_or_stale_class",
            id.0,
            class_id.0,
            revision
        );
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::ClassRejected {
                request_id,
                class_id,
                revision,
                reason: crate::ClassRejectReason::UnknownOrStaleClass,
            },
        );
        return;
    };

    if let Err(reason) = validate_class_content(world, &def) {
        diag::info!(
            Sim,
            "class select: rejected client={} id={} rev={} reason={}",
            id.0,
            class_id.0,
            revision,
            reason.as_str()
        );
        world.push_event(
            tick,
            EventAudience::Client(id),
            SimEvent::ClassRejected {
                request_id,
                class_id,
                revision,
                reason,
            },
        );
        return;
    }

    crate::script::answer_join(world.ecs(), id.0);
    crate::script::choose_class(world.ecs(), id.0, &def);
    world.push_event(
        tick,
        EventAudience::Client(id),
        SimEvent::ClassAccepted {
            request_id,
            class_id,
            revision,
        },
    );
}

fn validate_class_content(
    world: &FrameWorld,
    def: &crate::ClassDef,
) -> Result<(), crate::ClassRejectReason> {
    for (slot, perk) in def.perks.iter().copied().enumerate() {
        if perk != 0 && crate::match_state::perk_slot_from_class_catalog(perk) != Some(slot) {
            return Err(crate::ClassRejectReason::LockedContent);
        }
    }
    let max_attachments = if def.perks[0] == 12 { 2 } else { 1 };
    let table_len = world.weapon_combat_len();
    for id in [def.primary, def.secondary] {
        if id == 0 {
            continue;
        }
        if (id as usize) >= table_len {
            return Err(crate::ClassRejectReason::UnknownWeaponId);
        }
        let Some(row) = world.weapon_combat_row(id) else {
            return Err(crate::ClassRejectReason::UnknownWeaponId);
        };
        if !row.is_usable()
            || !world.weapon_runnable(id)
            || world
                .weapon_setup(id)
                .is_some_and(|setup| setup.attachments.len() > max_attachments)
        {
            return Err(crate::ClassRejectReason::LockedContent);
        }
    }
    for (slot, id) in [def.lethal, def.tactical].into_iter().enumerate() {
        if id == 0 {
            continue;
        }
        if (id as usize) >= table_len {
            return Err(crate::ClassRejectReason::UnknownWeaponId);
        }
        let Some(facts) = world.offhand_loadout_row(id) else {
            return Err(crate::ClassRejectReason::LockedContent);
        };
        if !world.weapon_runnable(id)
            || !matches!((slot, facts.offhand_class), (0, 1 | 4 | 5) | (1, 2 | 3))
        {
            return Err(crate::ClassRejectReason::LockedContent);
        }
    }

    if def.primary_attachments.iter().any(|&a| a != 0)
        || def.secondary_attachments.iter().any(|&a| a != 0)
    {
        return Err(crate::ClassRejectReason::LockedContent);
    }

    Ok(())
}

pub(crate) fn log_forced_spawn(
    id: ClientId,
    pick: crate::SpawnPick,
    report: &crate::spawn::SpawnAttemptReport,
) {
    let asked = match pick {
        crate::SpawnPick::Seeded(seed) => format!("seed={seed}"),
        crate::SpawnPick::At { origin, yaw } => format!(
            "at=[{:.1}, {:.1}, {:.1}] yaw={yaw:.1}",
            origin[0], origin[1], origin[2]
        ),
    };
    let fallback = report
        .rejected
        .first()
        .filter(|(source, _)| *source == crate::spawn::FORCED_SPAWN_SOURCE)
        .map(|(_, reason)| format!(" fallback={}", reason.as_str()))
        .unwrap_or_default();
    match &report.accepted {
        Some(d) => diag::info!(
            Sim,
            "spawn: forced client={} {asked} origin=[{:.1}, {:.1}, {:.1}] yaw={:.1} source={} tried={}{fallback}",
            id.0,
            d.traced_origin[0],
            d.traced_origin[1],
            d.traced_origin[2],
            d.raw_angles[1],
            if d.source_index == crate::spawn::FORCED_SPAWN_SOURCE {
                String::from("at")
            } else {
                d.source_index.to_string()
            },
            report.tried
        ),
        None => diag::info!(
            Sim,
            "spawn: forced client={} {asked} refused tried={}{fallback}",
            id.0,
            report.tried
        ),
    }
}

struct PlayerBodyClip {
    entnum: u16,
    origin: [f32; 3],
    /// bo2zm M3: an entity's own box (a script mover's packed `solid`: a
    /// Black Ops II zombie); None = a player's.
    bounds: Option<([f32; 3], [f32; 3])>,
    /// bo2mp: whose own traces pass through it (a body: its own number; a
    /// mantle blocker: the climber's).
    owner: u16,
    /// What it stops: a body, or (a mantle blocker) player clip.
    contents: u32,
}

fn alive_body_clips(world: &FrameWorld) -> Vec<PlayerBodyClip> {
    let mut bodies: Vec<_> = world
        .client_ids_sorted()
        .into_iter()
        .filter(|id| {
            world
                .client_meta(*id)
                .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
        })
        .filter_map(|id| {
            world.player(id).map(|ps| PlayerBodyClip {
                entnum: id.0 as u16,
                origin: ps.origin,
                bounds: None,
                owner: id.0 as u16,
                contents: crate::world::CONTENTS_BODY,
            })
        })
        .collect();
    bodies.extend(
        world
            .prediction_remote_bodies()
            .iter()
            .map(|body| PlayerBodyClip {
                entnum: body.client.0 as u16,
                origin: body.origin,
                bounds: None,
                owner: body.client.0 as u16,
                contents: crate::world::CONTENTS_BODY,
            }),
    );
    // bo2zm M3: entities solid as a packed box block as that box, as the legacy
    // movement clips against entities' packed bounds (BO2's zombies body-
    // block: a crowd corners him). The same rows reach his client's
    // prediction in the snapshot.
    bodies.extend(
        world
            .packed_solid_movers()
            .into_iter()
            .map(|(entnum, origin, solid)| {
                let x = (solid & 0xff) as f32;
                let below = ((solid >> 8) & 0xff) as f32;
                let above = ((solid >> 16) & 0xff) as f32 - 32.0;
                PlayerBodyClip {
                    entnum,
                    origin,
                    bounds: Some(([-x, -x, -below], [x, x, above])),
                    owner: entnum,
                    contents: crate::world::CONTENTS_BODY,
                }
            }),
    );
    // bo2mp: the climbers' landing spots (G_AddPlayerMantleBlockage), the
    // climber's box in player clip. BO2 spawns an entity for each, not a
    // client, so a hit names no player (stepping onto one is allowed).
    bodies.extend(world.mantle_blockers().iter().map(|b| PlayerBodyClip {
        entnum: trace_t6::ENTITYNUM_WORLD,
        origin: b.origin,
        bounds: None,
        owner: b.owner,
        contents: crate::world::CONTENTS_PLAYERCLIP,
    }));
    bodies
}

/// Does the box at `a` (`amins..amaxs`) overlap the box at `b`?
fn boxes_overlap(
    a: [f32; 3],
    amins: [f32; 3],
    amaxs: [f32; 3],
    b: [f32; 3],
    bmins: [f32; 3],
    bmaxs: [f32; 3],
) -> bool {
    (0..3).all(|i| a[i] + amins[i] < b[i] + bmaxs[i] && a[i] + amaxs[i] > b[i] + bmins[i])
}

#[derive(Clone, Copy)]
struct ClipBackend<'a> {
    brushes: &'a [SimBrush],
    bsp: &'a SimClipBsp,
    mesh: &'a SimClipMesh,
    glass_damage: &'a [(u32, u16)],
    bodies: &'a [PlayerBodyClip],
    self_entnum: u16,
    cmodels: &'a [asset_world::ClipCmodel],
    /// The brush models (doors, lifts), each with its entity number.
    linked_brushes: &'a [(u16, LinkedBrushCollisionBrush)],
    /// bo2mp: whose each linked brush is (in step with `linked_brushes`).
    linked_owners: &'a [crate::AuthorityModelOwner],
    /// bo2mp: each trace a linked brush decided (fraction, end, its place);
    /// None where nothing reads the touches (vehicles, benches).
    struck: Option<&'a RefCell<Vec<(f32, [f32; 3], usize)>>>,
    /// bo2mp: the owners his move touched (BO2's pmove touchents, 32 at most).
    touched: Option<&'a RefCell<Vec<crate::AuthorityModelOwner>>>,
    /// bo2zm: solid collision models as turned boxes (entity number, brush).
    blockers: &'a [(u16, SimBrush)],
    /// bo2mp: the script movers, by entity number, for the platforms players
    /// ride ([`movement_t6::Bo2Collision::mover_pose`]).
    movers: &'a [ScriptMoverGentity],
    /// bo2mp: the other scorestreak vehicles (a vehicle's own move only).
    vehicles: &'a [OtherVehicle],
    /// bo2mp: Black Ops II's own collision is on (a Black Ops II game always uses it).
    gjk: bool,
}

/// bo2mp: the script models each player's move touched this tick (BO2's
/// pmove touch list); the map scripts get their "touch" notifies.
#[derive(Resource, Default)]
pub(crate) struct PlayerModelTouches(pub(crate) Vec<(u32, crate::AuthorityModelOwner)>);

/// bo2zm M3: the world's solid collision models (Nuketown's patch walls,
/// perk machines, the Mystery Box) as six-plane brushes.
fn oriented_blocker_brushes(world: &FrameWorld) -> Vec<(u16, SimBrush)> {
    world
        .oriented_blockers()
        .iter()
        .map(|b| {
            (
                b.entnum,
                SimBrush {
                    planes: b.planes().to_vec(),
                    contents: crate::bullet_collision::CONTENTS_SOLID,
                    plane_surface_flags: vec![0; 6],
                    glass_encoded: 0,
                    verts: Vec::new(),
                    hit_surface_flags: 0,
                },
            )
        })
        .collect()
}

/// bo2zm M3: the move clipped against the turned boxes too. One he is
/// already inside never holds him (he walks out of it), as with entities'
/// boxes.
fn clip_move_to_blockers(
    mut hit: Trace,
    blockers: &[(u16, SimBrush)],
    input: GroundTraceInput,
) -> Trace {
    for (entnum, brush) in blockers {
        let other = crate::world::t6_brushes_trace(
            &[brush],
            input.start,
            input.end,
            input.mins,
            input.maxs,
            input.tracemask,
        );
        if other.startsolid || other.allsolid || other.fraction >= hit.fraction {
            continue;
        }
        hit = Trace {
            hit_type: TraceHitType::Entity,
            hit_id: *entnum,
            ..other
        };
    }
    hit
}

/// bo2mp: a vehicle's move clipped against the other vehicles' turned boxes
/// too, as against the blockers (one it starts inside never holds it).
fn clip_move_to_vehicles(
    hit: Trace,
    vehicles: &[OtherVehicle],
    self_entnum: u16,
    input: GroundTraceInput,
) -> Trace {
    if input.tracemask & VEHICLE_CONTENTS == 0 {
        return hit;
    }
    let boxes: Vec<(u16, SimBrush)> = vehicles
        .iter()
        .filter(|v| v.entnum != self_entnum)
        .map(|v| (v.entnum, v.brush.clone()))
        .collect();
    clip_move_to_blockers(hit, &boxes, input)
}

/// bo2mp: the world's shapes for Black Ops II's own collision, as its server
/// gathers them for the box a move can reach: the world's brushes and terrain
/// patches, the brush models (doors, movers), the other players' bodies and
/// (a vehicle's move) the other vehicles.
/// Static models are not in it (BO2's query has only brushes and terrain);
/// the bo2zm blockers and glass are not either.
impl movement_t6::Bo2Collision for ClipBackend<'_> {
    fn gjk_on(&self) -> bool {
        self.gjk
    }

    fn gjk_query<'a>(
        &'a self,
        mins: [f32; 3],
        maxs: [f32; 3],
        mask: u32,
        out: &mut Vec<movement_t6::gjk::Geom<'a>>,
    ) {
        use movement_t6::gjk::{Geom, IDENTITY, Shape, player_hull};
        let reaches = |lo: [f32; 3], hi: [f32; 3]| (0..3).all(|a| lo[a] <= maxs[a] && mins[a] <= hi[a]);

        // The brushes of the leaves the box reaches (with no map, a test's
        // hand-made room: every brush).
        let mut brush_ids: Vec<u16> = Vec::new();
        match self.bsp.t6 {
            Some(map) => clipmap_t6::box_brushes(map, mins, maxs, mask, &mut brush_ids),
            None => brush_ids.extend((0..self.brushes.len()).filter_map(|i| u16::try_from(i).ok())),
        }
        for &id in &brush_ids {
            let Some(brush) = self.brushes.get(usize::from(id)) else {
                continue;
            };
            if brush.contents & mask == 0 {
                continue;
            }
            if let Some(geom) = brush_geom(brush, 1 << 32 | u64::from(id))
                && reaches(geom.mins, geom.maxs)
            {
                out.push(geom);
            }
        }

        // Terrain: each tree of those leaves the box reaches, down to its
        // patches.
        if let Some(map) = self.bsp.t6 {
            let mut leafs = [0u16; 1024];
            let mut ll = clipmap_t6::LeafList::new(&mut leafs, mins, maxs);
            clipmap_t6::box_leafnums(map, &mut ll, 0);
            let mut roots: Vec<usize> = Vec::new();
            for &l in ll.leafs() {
                let Some(leaf) = map.leafs.get(usize::from(l)) else {
                    continue;
                };
                let first = usize::from(leaf.first_coll_aabb);
                for root in first..first + usize::from(leaf.coll_aabb_count) {
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            }
            let mut stack: Vec<usize> = roots.into_iter().rev().collect();
            while let Some(index) = stack.pop() {
                let Some(node) = map.aabb_trees.get(index) else {
                    continue;
                };
                let lo = [0, 1, 2].map(|a| node.origin[a] - node.half_size[a]);
                let hi = [0, 1, 2].map(|a| node.origin[a] + node.half_size[a]);
                if !reaches(lo, hi) {
                    continue;
                }
                let u = node.u as usize;
                if node.child_count != 0 {
                    stack.extend((u..u + usize::from(node.child_count)).rev());
                    continue;
                }
                let Some(part) = map.partitions.get(u) else {
                    continue;
                };
                let Ok(first) = usize::try_from(part.first_tri) else {
                    continue;
                };
                let Some(tris) = map.tri_indices.get(first..first + usize::from(part.tri_count)) else {
                    continue;
                };
                // BO2 asks the tree's material.
                let material = map.materials.get(usize::from(node.material_index)).copied().unwrap_or_default();
                if material.content_flags & mask == 0 {
                    continue;
                }
                out.push(Geom {
                    shape: Shape::Partition { verts: map.verts, indices: tris.as_flattened() },
                    axis: IDENTITY,
                    origin: [0.0; 3],
                    mins: lo,
                    maxs: hi,
                    contents: material.content_flags,
                    surface_flags: asset_world::t5_surface_flags(material.surface_flags),
                    hit_id: trace_t6::ENTITYNUM_WORLD,
                    sentient: false,
                    no_push_out: false,
                    key: 2 << 32 | u as u64,
                });
            }
        }

        // Brush models, turned and moved as their entity is.
        for (n, (entnum, linked)) in self.linked_brushes.iter().enumerate() {
            let Some(cmodel) = self.cmodels.get(linked.cmodel_handle as usize) else {
                continue;
            };
            let reach = cmodel.radius + 1.0;
            if !reaches(linked.origin.map(|o| o - reach), linked.origin.map(|o| o + reach)) {
                continue;
            }
            let axis = game_math::angles_to_axis(linked.angles);
            let first = cmodel.first_brush as usize;
            let ids = self
                .bsp
                .leafbrushes
                .get(first..first + usize::from(cmodel.num_brushes))
                .unwrap_or(&[]);
            for &id in ids {
                let Some(brush) = self.brushes.get(usize::from(id)) else {
                    continue;
                };
                if brush.contents & mask == 0 {
                    continue;
                }
                let Some(mut geom) = brush_geom(brush, 3 << 32 | (n as u64) << 16 | u64::from(id)) else {
                    continue;
                };
                (geom.mins, geom.maxs) = turned_box(geom.mins, geom.maxs, &axis, linked.origin);
                geom.axis = axis;
                geom.origin = linked.origin;
                geom.hit_id = *entnum;
                if reaches(geom.mins, geom.maxs) {
                    out.push(geom);
                }
            }
        }

        // The other players' bodies (and the bo2zm zombies' boxes, and the
        // climbers' landing spots).
        {
            for (n, body) in self.bodies.iter().enumerate() {
                if body.owner == self.self_entnum || mask & body.contents == 0 {
                    continue;
                }
                let (body_mins, body_maxs) = body.bounds.unwrap_or((PLAYER_MINS, PLAYER_MAXS));
                let hull = player_hull(body_mins, body_maxs, 0.0);
                let (lo, hi) = hull.calc_aabb();
                let geom = Geom {
                    shape: Shape::Player { hull },
                    axis: IDENTITY,
                    origin: body.origin,
                    mins: [0, 1, 2].map(|a| body.origin[a] + lo[a]),
                    maxs: [0, 1, 2].map(|a| body.origin[a] + hi[a]),
                    contents: body.contents,
                    surface_flags: 0,
                    hit_id: body.entnum,
                    sentient: body.contents == crate::world::CONTENTS_BODY,
                    no_push_out: true,
                    key: 4 << 32 | n as u64,
                };
                if reaches(geom.mins, geom.maxs) {
                    out.push(geom);
                }
            }
        }

        // bo2mp: the other vehicles, each its model's collision piece turned
        // to its angles at its origin (never the vehicle moving).
        if mask & VEHICLE_CONTENTS != 0 {
            for (n, v) in self.vehicles.iter().enumerate() {
                let (lo, hi) = v.world;
                if v.entnum == self.self_entnum || !reaches(lo, hi) {
                    continue;
                }
                out.push(Geom {
                    shape: Shape::Brush { verts: &v.verts, mins: v.mins, maxs: v.maxs, sides: &[] },
                    axis: v.axis,
                    origin: v.origin,
                    mins: lo,
                    maxs: hi,
                    contents: VEHICLE_CONTENTS,
                    surface_flags: 0,
                    hit_id: v.entnum,
                    sentient: false,
                    no_push_out: v.no_push_out,
                    key: 5 << 32 | n as u64,
                });
            }
        }
    }

    /// CM_GetWaterHeight: a point traced down through the feet, from 200
    /// above them to 200 below, that only water stops.
    fn water_height(&self, origin: [f32; 3]) -> f32 {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER, WATER_SEARCH};
        let start = [origin[0], origin[1], origin[2] + WATER_SEARCH];
        let end = [origin[0], origin[1], origin[2] - WATER_SEARCH];
        let hit = clip_trace(
            self.brushes,
            self.bsp,
            self.mesh,
            start,
            end,
            [0.0; 3],
            [0.0; 3],
            CONTENTS_WATER,
            &|_| true,
        );
        if hit.fraction < 1.0 {
            start[2] + (end[2] - start[2]) * hit.fraction
        } else {
            NO_WATER
        }
    }

    /// A script mover's pose on its path at `time`: where it was at the
    /// last command and is at this one (BG_EvaluateTrajectory on its
    /// position and its angles).
    fn mover_pose(&self, entnum: i32, time: i32) -> Option<([f32; 3], [f32; 3])> {
        let i = self
            .movers
            .binary_search_by_key(&entnum, |mover| mover.state.number)
            .ok()?;
        let state = &self.movers[i].state;
        Some((
            entity_t6::evaluate_trajectory(&pos_from_entity_state(state), time),
            entity_t6::evaluate_trajectory(&apos_from_entity_state(state), time),
        ))
    }
}

/// bo2mp: one brush as Black Ops II's collision sees it, in its own frame: a
/// box when it has only its six axial sides, else its corner points and its
/// other sides. None: a brush with sides but no corner points (not a Black
/// Ops II map's).
fn brush_geom(brush: &SimBrush, key: u64) -> Option<movement_t6::gjk::Geom<'_>> {
    use movement_t6::gjk::{Geom, IDENTITY, Shape};
    let p = brush.planes.get(..6)?;
    let mins = [-p[1][3], -p[3][3], -p[5][3]];
    let maxs = [p[0][3], p[2][3], p[4][3]];
    let shape = if brush.planes.len() == 6 {
        Shape::Box {
            center: [0, 1, 2].map(|a| (mins[a] + maxs[a]) * 0.5),
            half: [0, 1, 2].map(|a| (maxs[a] - mins[a]) * 0.5),
            brush: true,
        }
    } else if brush.verts.is_empty() {
        return None;
    } else {
        Shape::Brush { verts: &brush.verts, mins, maxs, sides: &brush.planes[6..] }
    };
    Some(Geom {
        shape,
        axis: IDENTITY,
        origin: [0.0; 3],
        mins,
        maxs,
        contents: brush.contents,
        surface_flags: brush.hit_surface_flags,
        hit_id: trace_t6::ENTITYNUM_WORLD,
        sentient: false,
        no_push_out: false,
        key,
    })
}

/// bo2mp: the world box around a local box turned by `axis` (rows) and moved
/// to `origin`.
fn turned_box(
    mins: [f32; 3],
    maxs: [f32; 3],
    axis: &[[f32; 3]; 3],
    origin: [f32; 3],
) -> ([f32; 3], [f32; 3]) {
    let mut lo = origin;
    let mut hi = origin;
    for a in 0..3 {
        for row in 0..3 {
            let (x, y) = (axis[row][a] * mins[row], axis[row][a] * maxs[row]);
            lo[a] += x.min(y);
            hi[a] += x.max(y);
        }
    }
    (lo, hi)
}

/// bo2mp: a map's collision as the player's moves see it, with nobody else
/// in it, for headless benches (`t6walk`). `gjk` picks Black Ops II's own
/// collision or the box traces.
#[doc(hidden)]
pub fn bo2_map_collision<'a>(
    brushes: &'a [SimBrush],
    bsp: &'a SimClipBsp,
    mesh: &'a SimClipMesh,
    gjk: bool,
) -> impl movement_t6::Bo2Collision + 'a {
    ClipBackend {
        brushes,
        bsp,
        mesh,
        glass_damage: &[],
        bodies: &[],
        self_entnum: 0,
        cmodels: &[],
        linked_brushes: &[],
        linked_owners: &[],
        struck: None,
        touched: None,
        blockers: &[],
        movers: &[],
        vehicles: &[],
        gjk,
    }
}

/// bo2mp: a vehicle's contents (BO2's `VEH_InitEntity`): what a vehicle's
/// own clip mask meets of another.
const VEHICLE_CONTENTS: u32 = 0x0081_2080;

/// bo2mp: another scorestreak vehicle as a vehicle's move meets it. BO2's
/// collision query adds a vehicle by its model's collision pieces, turned to
/// its angles at its origin; one whose model has none is not in it, and a
/// hidden one has no contents.
#[derive(Clone)]
pub(crate) struct OtherVehicle {
    entnum: u16,
    origin: [f32; 3],
    axis: [[f32; 3]; 3],
    /// Its collision piece's corner points and their box, in its own frame.
    verts: Vec<[f32; 3]>,
    mins: [f32; 3],
    maxs: [f32; 3],
    /// That box turned and placed: its world box, and as a brush (with the
    /// world box's sides) for the box traces.
    world: ([f32; 3], [f32; 3]),
    brush: SimBrush,
    /// It is driven from afar (its def's `remoteControl`): BO2's move is
    /// never pushed out of one, as out of a player.
    no_push_out: bool,
}

impl OtherVehicle {
    /// None: no collision piece's corner points.
    pub(crate) fn new(
        entnum: u16,
        origin: [f32; 3],
        angles: [f32; 3],
        verts: Vec<[f32; 3]>,
        no_push_out: bool,
    ) -> Option<Self> {
        let first = *verts.first()?;
        let (mut mins, mut maxs) = (first, first);
        for p in &verts {
            for a in 0..3 {
                mins[a] = mins[a].min(p[a]);
                maxs[a] = maxs[a].max(p[a]);
            }
        }
        let axis = game_math::angles_to_axis(angles);
        let (lo, hi) = turned_box(mins, maxs, &axis, origin);
        let mut planes = vec![
            [1.0, 0.0, 0.0, hi[0]],
            [-1.0, 0.0, 0.0, -lo[0]],
            [0.0, 1.0, 0.0, hi[1]],
            [0.0, -1.0, 0.0, -lo[1]],
            [0.0, 0.0, 1.0, hi[2]],
            [0.0, 0.0, -1.0, -lo[2]],
        ];
        for (row, n) in axis.iter().enumerate() {
            // A side along the world's axes is the world box's own (the box
            // traces push a turned side out less, as a capsule's).
            if n.iter().any(|c| c.abs() > 0.9999) {
                continue;
            }
            let d = n[0] * origin[0] + n[1] * origin[1] + n[2] * origin[2];
            planes.push([n[0], n[1], n[2], d + maxs[row]]);
            planes.push([-n[0], -n[1], -n[2], -(d + mins[row])]);
        }
        let brush = SimBrush {
            plane_surface_flags: vec![0; planes.len()],
            planes,
            contents: VEHICLE_CONTENTS,
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        };
        Some(Self {
            entnum,
            origin,
            axis,
            verts,
            mins,
            maxs,
            world: (lo, hi),
            brush,
            no_push_out,
        })
    }
}

/// bo2mp: what a scorestreak vehicle's move meets besides the map, as a
/// player's move meets them (BO2's vehicle clip mask has bodies and
/// movers): the players' bodies, the brush models (doors, lifts), the
/// solid script movers, the bo2zm blockers and the other vehicles
/// ([`VehicleClip::with_vehicles`]), never the vehicle itself.
/// Gathered once for a move; [`VehicleClip::collision`] lends it with the
/// map.
pub struct VehicleClip {
    bodies: Vec<PlayerBodyClip>,
    linked_brushes: Vec<(u16, LinkedBrushCollisionBrush)>,
    blockers: Vec<(u16, SimBrush)>,
    movers: Vec<ScriptMoverGentity>,
    glass_damage: Vec<(u32, u16)>,
    /// The vehicle's own entity number: its own box never stops it.
    self_entnum: u16,
    vehicles: Vec<OtherVehicle>,
}

impl VehicleClip {
    pub(crate) fn gather(world: &mut FrameWorld, self_entnum: u16) -> Self {
        let movers = crate::frame::collect_script_movers(world.ecs());
        let mover_numbers: std::collections::BTreeMap<_, u16> = movers
            .iter()
            .map(|mover| (mover.id, mover.state.number as u16))
            .collect();
        let linked_brushes = world
            .entity_collision_capabilities()
            .iter()
            .flat_map(|c| {
                let entnum = c
                    .owner
                    .script_model()
                    .and_then(|id| mover_numbers.get(&id).copied())
                    .unwrap_or(trace_t6::ENTITYNUM_WORLD);
                c.solid_brushes().iter().map(move |b| (entnum, b.clone()))
            })
            .filter(|(entnum, _)| *entnum != self_entnum)
            .collect();
        let mut blockers = oriented_blocker_brushes(world);
        blockers.retain(|(entnum, _)| *entnum != self_entnum);
        Self {
            bodies: alive_body_clips(world),
            linked_brushes,
            blockers,
            movers,
            glass_damage: world.world_objects().glass_damage_pairs(),
            self_entnum,
            vehicles: Vec::new(),
        }
    }

    /// The other scorestreak vehicles too (its own is left out).
    pub(crate) fn with_vehicles(mut self, mut vehicles: Vec<OtherVehicle>) -> Self {
        let own = self.self_entnum;
        vehicles.retain(|v| v.entnum != own);
        self.vehicles = vehicles;
        self
    }

    /// The map in `content` with what was gathered; `gjk` picks Black Ops
    /// II's own collision or the box traces.
    pub fn collision<'a>(&'a self, content: &'a crate::world::SimContent, gjk: bool) -> impl movement_t6::Bo2Collision + 'a {
        self.backend(
            content.clip_brushes(),
            content.clip_bsp(),
            content.clip_mesh(),
            &content.clip_cmodels().models,
            gjk,
        )
    }

    fn backend<'a>(
        &'a self,
        brushes: &'a [SimBrush],
        bsp: &'a SimClipBsp,
        mesh: &'a SimClipMesh,
        cmodels: &'a [asset_world::ClipCmodel],
        gjk: bool,
    ) -> ClipBackend<'a> {
        ClipBackend {
            brushes,
            bsp,
            mesh,
            glass_damage: &self.glass_damage,
            bodies: &self.bodies,
            self_entnum: self.self_entnum,
            cmodels,
            linked_brushes: &self.linked_brushes,
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &self.blockers,
            movers: &self.movers,
            vehicles: &self.vehicles,
            gjk,
        }
    }
}

impl CollisionBackend for ClipBackend<'_> {
    fn trace(&self, input: GroundTraceInput) -> trace_t6::Trace {
        let world_hit = clip_trace(
            self.brushes,
            self.bsp,
            self.mesh,
            input.start,
            input.end,
            input.mins,
            input.maxs,
            input.tracemask,
            &|piece| {
                let damage = self
                    .glass_damage
                    .iter()
                    .find(|(id, _)| *id == u32::from(piece))
                    .map_or(0, |(_, d)| *d);
                crate::world_objects::glass_piece_is_solid(damage)
            },
        );
        let (with_bmodels, struck) = clip_move_to_bmodels_struck(
            world_hit,
            self.bsp.t6,
            self.linked_brushes.iter().map(|(entnum, brush)| (*entnum, brush)),
            input,
        );
        let with_bodies = clip_move_to_players(with_bmodels, self.bsp.t6, self.bodies, self.self_entnum, input);
        let with_blockers = clip_move_to_blockers(with_bodies, self.blockers, input);
        let hit = clip_move_to_vehicles(with_blockers, self.vehicles, self.self_entnum, input);
        if let Some(index) = struck
            && let Some(log) = self.struck
            && hit.fraction == with_bmodels.fraction
            && hit.hit_type == with_bmodels.hit_type
            && hit.hit_id == with_bmodels.hit_id
        {
            log.borrow_mut().push((hit.fraction, hit.end_pos(input.start, input.end), index));
        }
        hit
    }

    fn touch_entity(&self, trace: &trace_t6::Trace, end_pos: [f32; 3]) {
        let (Some(log), Some(touched)) = (self.struck, self.touched) else {
            return;
        };
        let Some(&(_, _, index)) = log
            .borrow()
            .iter()
            .rev()
            .find(|(fraction, end, _)| *fraction == trace.fraction && *end == end_pos)
        else {
            return;
        };
        let Some(&owner) = self.linked_owners.get(index) else {
            return;
        };
        let mut touched = touched.borrow_mut();
        // BO2's PM_AddTouchEnt: each once, 32 at most.
        if touched.len() < 32 && !touched.contains(&owner) {
            touched.push(owner);
        }
    }
}

/// `t6`: on a Black Ops II map each body is its temporary box
/// (`CM_TempBoxModel`), traced as a capsule.
fn clip_move_to_players(
    mut hit: Trace,
    t6: Option<&clipmap_t6::ClipMap<'_>>,
    bodies: &[PlayerBodyClip],
    self_entnum: u16,
    input: GroundTraceInput,
) -> Trace {
    if hit.fraction == 0.0 {
        return hit;
    }
    for body in bodies {
        if body.owner == self_entnum {
            continue;
        }
        let (body_mins, body_maxs) = body.bounds.unwrap_or((PLAYER_MINS, PLAYER_MAXS));
        // bo2zm M3: an entity's box never traps him: inside one (a zombie
        // stepped into him as he stepped into it) he may move away from its
        // centre, never deeper; deeper stops him on a plane facing away.
        if body.bounds.is_some()
            && boxes_overlap(
                input.start,
                input.mins,
                input.maxs,
                body.origin,
                body_mins,
                body_maxs,
            )
        {
            let flat =
                |p: [f32; 3]| (p[0] - body.origin[0]).powi(2) + (p[1] - body.origin[1]).powi(2);
            if flat(input.end) >= flat(input.start) {
                continue;
            }
            let d = [
                input.start[0] - body.origin[0],
                input.start[1] - body.origin[1],
            ];
            let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
            hit.fraction = 0.0;
            hit.normal = if len > 1e-3 {
                [d[0] / len, d[1] / len, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            hit.cflags = body.contents;
            hit.hit_type = TraceHitType::Entity;
            hit.hit_id = body.entnum;
            hit.walkable = false;
            return hit;
        }
        // With no map (no brushes at all) the temporary box needs none.
        let other = crate::world::t6_transformed_trace(
            t6.unwrap_or(&clipmap_t6::ClipMap::default()),
            input.start,
            input.end,
            input.mins,
            input.maxs,
            crate::world::T6Model::TempBox {
                mins: body_mins,
                maxs: body_maxs,
                contents: body.contents,
            },
            input.tracemask,
            body.origin,
            [0.0; 3],
        );
        if other.fraction >= hit.fraction && !other.startsolid && !other.allsolid {
            continue;
        }
        let startsolid = hit.startsolid | other.startsolid;
        let allsolid = hit.allsolid | other.allsolid;
        if other.fraction < hit.fraction {
            hit = other;
            hit.hit_type = TraceHitType::Entity;
            hit.hit_id = body.entnum;
        }
        hit.startsolid = startsolid;
        hit.allsolid = allsolid;
    }
    hit
}

/// bo2zm: the rules the game in this process last ran with. The local
/// player's own guess of their move (the prediction world) has no scripts of
/// its own, so it borrows these; without them it guessed with MW2 rules while
/// the game ran BO2's, and a dive jumped back on every correction.
static HOST_FEEL: std::sync::Mutex<Option<movement_t6::bridge::Bo2Feel>> = std::sync::Mutex::new(None);

/// bo2zm: the Black Ops II movement rules, always on in a Black Ops II
/// game. Values the scripts set win over the defaults. `predicting` = this
/// is the local player's guess, not the game itself; `sent` = the rules the
/// game sent in its last snapshot.
fn bo2_feel(
    ecs: &World,
    predicting: bool,
    sent: Option<movement_t6::bridge::Bo2Feel>,
) -> movement_t6::bridge::Bo2Feel {
    // The Black Ops II scripts' own dvars (setdvar); none = no T6 game.
    let Some(t6) = ecs.get_resource::<crate::t6::T6Runtime>() else {
        if predicting {
            // The game's own rules first: a second PC has no scripts (it
            // guessed with MW2's), and a game hosted here before may have
            // left its rules behind.
            return sent
                .or_else(|| HOST_FEEL.lock().ok().and_then(|f| *f))
                .unwrap_or_else(|| bo2_feel_rules(|_| None, false));
        }
        if let Ok(mut f) = HOST_FEEL.lock() {
            *f = None;
        }
        return bo2_feel_rules(|_| None, false);
    };
    let feel = bo2_feel_from(t6);
    if !predicting && let Ok(mut f) = HOST_FEEL.lock() {
        *f = Some(feel);
    }
    feel
}

/// bo2zm: the Controls setting "Omni Movement" (10-10: off by
/// default, an option in the Controls menu for whoever wants it).
static OMNI_SETTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// bo2zm: his "Omni Movement" setting; the next game started here reads it.
pub fn set_omni_movement(on: bool) {
    OMNI_SETTING.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// bo2zm: the "Omni Movement" setting as a game starting now takes it.
pub(crate) fn omni_movement_setting() -> bool {
    OMNI_SETTING.load(std::sync::atomic::Ordering::Relaxed)
}

fn bo2_feel_from(t6: &crate::t6::T6Runtime) -> movement_t6::bridge::Bo2Feel {
    bo2_feel_rules(|name| t6.vm.dvars.get(name).map(String::as_str), t6.omni)
}

/// The Black Ops II movement rules from the scripts' dvars (`dvar`); a dvar
/// no script set takes BO2's default. With no scripts at all (`|_| None`)
/// these are BO2 multiplayer's own rules.
fn bo2_feel_rules<'a>(dvar: impl Fn(&str) -> Option<&'a str>, omni: bool) -> movement_t6::bridge::Bo2Feel {
    let number = |name: &str, default: f32| {
        dvar(name)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(default)
    };
    let zombies = dvar("mapname").is_some_and(|m| m.starts_with("zm_"));
    movement_t6::bridge::Bo2Feel {
        sprint_strafe_speed_scale: number("player_sprintStrafeSpeedScale", 0.667),
        // BO2 turns the repeated-jump slowdown off in Zombies and on in
        // multiplayer, unless a script sets the dvar.
        jump_slowdown: number("jump_slowdownEnable", if zombies { 0.0 } else { 1.0 }) != 0.0,
        // Both modes read the same fall heights.
        fall_damage_min_height: number("bg_fallDamageMinHeight", 128.0),
        fall_damage_max_height: number("bg_fallDamageMaxHeight", 300.0),
        zombies,
        gravity: number("bg_gravity", 800.0) as i32,
        // The fall perk works in both modes (Turned's zombies get it);
        // Flak Jacket only changes falls in Zombies, where it is PhD Flopper.
        fall_damage_perk: playerstate_t6::perks::PERK_FALLHEIGHT,
        flak_jacket_perk: playerstate_t6::perks::PERK_FLAKJACKET,
        fast_ads_perk: playerstate_t6::perks::PERK_FASTADS,
        fast_ads_multiplier: number("perk_weapAdsMultiplier", 0.5),
        // Stamin-Up: sprint time x perk_sprintMultiplier, and in Zombies a
        // sprint at least 1.1 times the gun's speed.
        longer_sprint_perk: playerstate_t6::perks::PERK_LONGERSPRINT,
        sprint_multiplier: number("perk_sprintMultiplier", 2.0),
        // Move-faster: the walk speed x perk_speedMultiplier (190 -> 203).
        move_faster_perk: playerstate_t6::perks::PERK_MOVEFASTER,
        speed_multiplier: number("perk_speedMultiplier", 1.07),
        g_speed: number("g_speed", 190.0),
        // dtp: BO2's dive to prone switch, on in both modes.
        dive: number("dtp", 1.0) != 0.0,
        // Set per player from the gun in hand.
        sprint_cycle_scale: 1.0,
        ducked_sprint_cycle_scale: 1.0,
        dtp_cycle_scale: 1.0,
        // BO2 sprints forward only (forwardmove above
        // player_sprintForwardMinimum); 10-10: off by default, the
        // Controls menu's "Omni Movement" turns it on (read at game start).
        omni,
        // Low climbs keep the gun in hand (defaults until real BO2 is read).
        mantle_weapon_height: number("mantle_weapon_height", 39.0),
        mantle_weapon_anim_height: number("mantle_weapon_anim_height", 32.0),
        // jump_height (BO2 39; the mod menu's Super Jump sets it).
        jump_height: number("jump_height", 39.0),
    }
}

fn pmove_context(
    old_buttons: u32,
    weapon_scales: (f32, f32, f32),
    ads_allowed: bool,
    aim_down_sight: bool,
    ads_in_rate: f32,
    ads_out_rate: f32,
    rechamber_while_ads: bool,
    ads_reload: (bool, i32, bool),
    ads_fire_only: bool,
    melee_delay_ms: i32,
    melee_charge_delay_ms: i32,
    overlay_reticle: i32,
    shellshock_movement_scale: f32,
    feel: movement_t6::bridge::Bo2Feel,
) -> PmoveSingleContext {
    let player_sprint_time = 4.0_f32;
    let weapon_max_sprint_time = get_max_sprint_time(weapon_scales.2, player_sprint_time);

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
                weapon_move_speed_scale: weapon_scales.0,
                weapon_ads_move_speed_scale: weapon_scales.1,

                shellshock_movement_scale,
                // BO2 feel M10b: going prone takes 600 ms (MW2 400).
                prone_lerp_ms: movement_t6::bridge::PRONE_LERP_MS,
                // BO2 feel M10c: sprint speed only while standing. BO2 keeps
                // the 0.4 slow while aiming, leaning or aiming prone (its
                // machine code, 10-19).
            },
            weapon_move_scale: 1.0,
            old_buttons,
            jump: JumpLaunchContext {
                jump_height: feel.jump_height,
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
            weapon_max_sprint_time,
            sprint_forever: false,
            min_sprint_time_seconds: 1.0,
            sprint_delay_seconds: 0.0,

            sprint_forward_minimum: 105,
            stand_up_clear: true,
            sprint_recharge_pause_seconds: 0.0,
        },
        ads_intent: movement_t6::bridge::AdsIntentContext {
            ads_allowed,
            weapon_def_scope: overlay_reticle != 0,
            sprint_hold_ads: false,
        },
        ads_frac: movement_t6::bridge::AdsFracContext {
            aim_down_sight,
            ads_in_rate,
            ads_out_rate,
            rechamber_while_ads,
            ads_fire_only,
            reload_lowers_ads: ads_reload.0,
            position_reload_trans_ms: ads_reload.1,
            segmented_reload: ads_reload.2,
            weapon_brain_owns: false,
        },
        melee_charge: movement_t6::bridge::MeleeChargeWeaponDelays {
            melee_delay_ms,
            melee_charge_delay_ms,
        },
        player_melee_range: movement_t6::bridge::MELEE_CHARGE_PLAYER_MELEE_RANGE_DEFAULT,
        old_buttons,
        old_move: [0, 0],
        weapon_blocks_prone: false,
    }
}

pub(crate) fn seed_ps_ammo_tables(
    ps: &mut PlayerState,
    weapon: u32,
    facts: &weapon_t6::combat_facts::WeaponCombatFacts,
    clip: i32,
    clip_alt: i32,
    dual: bool,
    stock: i32,
) {
    let ammo_index = weapon_t6::ammo_table::ammo_table_key(facts.ammo_index, weapon);
    let clip_index = weapon_t6::ammo_table::clip_table_key(facts.clip_index, weapon);
    if ammo_index != 0 {
        let _ = weapon_t6::ammo_table::set_ammo_not_in_clip(&mut ps.ammo, ammo_index, stock);
    }
    if clip_index != 0 {
        let _ = weapon_t6::ammo_table::set_clip_for_hand(&mut ps.ammoclip, clip_index, 0, clip);
        if dual {
            let _ = weapon_t6::ammo_table::set_clip_for_hand(&mut ps.ammoclip, clip_index, 1, clip_alt);
        }
    }
}

pub(crate) fn arm_held_weapon(
    ps: &mut PlayerState,
    weapon: u32,
    facts: &weapon_t6::combat_facts::WeaponCombatFacts,
) -> (i32, i32) {
    ps.weapon_primary = weapon;
    let last_hand = weapon_t6::ammo_table::num_hands_for_held(&ps.weapons, &ps.weapon_data, weapon);
    ps.last_weapon_hand = last_hand;
    let (clip0, clip1, stock) = weapon_t6::combat_facts::spawn_clip_stock(facts, last_hand);
    let hand = weapon_t6::combat_facts::spawn_weapon_hand(weapon, facts);
    ps.weaponstate_primary = hand.weaponstate;
    ps.weapon_time = hand.weapon_time;
    ps.weapon_delay = hand.weapon_delay;
    ps.weap_anim = hand.weap_anim;
    if last_hand >= 1 {
        ps.weaponstate_secondary = hand.weaponstate;
        ps.weapon_time_secondary = hand.weapon_time;
        ps.weapon_delay_secondary = hand.weapon_delay;
        ps.weap_anim_secondary = hand.weap_anim;
    }
    seed_ps_ammo_tables(ps, weapon, facts, clip0, clip1, last_hand >= 1, stock);
    (clip0, stock)
}


#[cfg(test)]
mod sent_feel_tests {
    use super::*;

    fn zombies_feel() -> movement_t6::bridge::Bo2Feel {
        movement_t6::bridge::Bo2Feel {
            zombies: true,
            g_speed: 175.0,
            ..movement_t6::bridge::Bo2Feel::BO2_MP
        }
    }

    #[test]
    fn a_second_pc_guesses_with_the_rules_the_game_sent() {
        let feel = zombies_feel();
        assert_eq!(bo2_feel(&World::new(), true, Some(feel)), feel);
    }

    #[test]
    fn the_snapshot_carries_the_rules_to_the_other_pc() {
        let feel = zombies_feel();
        let mut host = crate::carrier::SimWorld::default();
        host.frame().set_feel(feel);
        let snapshot = host.snapshot(crate::world::Tick(1));
        assert_eq!(snapshot.meta.feel, Some(feel));
        let mut guess = crate::carrier::SimWorld::default();
        guess.adopt_snapshot(&snapshot);
        assert_eq!(guess.frame().feel(), Some(feel));
    }
}

#[cfg(test)]
mod fall_damage_tests {
    use super::fall_damage;

    #[test]
    fn falls_cost_a_share_of_max_health_rounded_down() {
        // Worked out in floats: event parm 15 on 100 health is 14 damage.
        assert_eq!(fall_damage(15, 100), 14);
        assert_eq!(fall_damage(0, 100), 0);
        assert_eq!(fall_damage(100, 100), 110);
        assert_eq!(fall_damage(250, 150), 165);
    }

    #[test]
    fn a_player_under_water_breathes_for_16_seconds_then_drowns_each_second() {
        use super::breath_step;
        let mut out = 0;
        // Swimming with the head up (level 4) is fine.
        assert!(!breath_step(&mut out, 4, 50_000));
        assert_eq!(out, 50_000);
        // Under: 16 s of breath, the first hurt just after.
        let hurts: Vec<i32> = (1..=400)
            .map(|f| 50_000 + f * 50)
            .filter(|&t| breath_step(&mut out, 6, t))
            .collect();
        assert_eq!(hurts, vec![66_050, 67_050, 68_050, 69_050]);
        // Coming up for one frame refills the breath.
        assert!(!breath_step(&mut out, 1, 70_050));
        assert!(!breath_step(&mut out, 5, 86_050));
        assert!(breath_step(&mut out, 5, 86_100));
    }
}

#[cfg(test)]
mod mover_ride_tests {
    use super::*;
    use crate::world::clip_move_to_bmodels;
    use movement_t6::Bo2Collision;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    /// A 64-unit floor 8 thick, its top at its origin.
    fn floor() -> SimBrush {
        SimBrush {
            planes: vec![
                [1.0, 0.0, 0.0, 32.0],
                [-1.0, 0.0, 0.0, 32.0],
                [0.0, 1.0, 0.0, 32.0],
                [0.0, -1.0, 0.0, 32.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, -1.0, 8.0],
            ],
            contents: crate::bullet_collision::CONTENTS_SOLID,
            plane_surface_flags: vec![0; 6],
            glass_encoded: 0,
            verts: Vec::new(),
            hit_surface_flags: 0,
        }
    }

    #[test]
    fn the_water_surface_is_found_over_and_under_the_feet() {
        use movement_t6::water::{CONTENTS_WATER, NO_WATER};
        // A pool 100 deep, its surface at 0.
        let mut pool = floor();
        pool.contents = CONTENTS_WATER;
        pool.planes[5] = [0.0, 0.0, -1.0, 100.0];
        let brushes = [pool];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        let world = bo2_map_collision(&brushes, &bsp, &mesh, false);
        for feet in [-50.0, -99.0, 30.0] {
            let height = world.water_height([0.0, 0.0, feet]);
            assert!(height.abs() < 0.5, "feet {feet}: {height}");
        }
        assert_eq!(world.water_height([100.0, 0.0, -50.0]), NO_WATER, "beside it");
        assert_eq!(world.water_height([0.0, 0.0, 250.0]), NO_WATER, "too high over it");
        // Solid ground is not water.
        let solid = [floor()];
        let world = bo2_map_collision(&solid, &bsp, &mesh, false);
        assert_eq!(world.water_height([0.0, 0.0, 10.0]), NO_WATER);
    }

    #[test]
    fn a_lift_on_its_path_has_a_pose_at_each_command() {
        let mut lift = ScriptMoverGentity::default();
        lift.state.number = 40;
        lift.state.lerp.pos.tr_type = entity_t6::trajectory::tr_type::LINEAR;
        lift.state.lerp.pos.tr_time = 1000;
        lift.state.lerp.pos.tr_base = [0.0, 0.0, 100.0];
        lift.state.lerp.pos.tr_delta = [0.0, 0.0, 200.0];
        lift.state.lerp.apos.tr_type = entity_t6::trajectory::tr_type::LINEAR;
        lift.state.lerp.apos.tr_time = 1000;
        lift.state.lerp.apos.tr_delta = [0.0, 90.0, 0.0];
        let mut still = ScriptMoverGentity::default();
        still.state.number = 41;
        still.state.lerp.pos.tr_base = [5.0, 0.0, 0.0];
        let movers = [lift, still];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        let backend = ClipBackend {
            brushes: &[],
            bsp: &bsp,
            mesh: &mesh,
            glass_damage: &[],
            bodies: &[],
            self_entnum: 0,
            cmodels: &[],
            linked_brushes: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &movers,
            vehicles: &[],
            gjk: false,
        };
        let (origin, angles) = backend.mover_pose(40, 1500).expect("the lift has a pose");
        assert!(close(origin, [0.0, 0.0, 200.0]), "{origin:?}");
        assert!(close(angles, [0.0, 45.0, 0.0]), "{angles:?}");
        // Standing still: the same pose whenever it is asked.
        assert_eq!(backend.mover_pose(41, 1500), backend.mover_pose(41, 900));
        assert_eq!(backend.mover_pose(41, 1500).map(|p| p.0), Some([5.0, 0.0, 0.0]));
        // Not a script mover: no pose, nothing carried.
        assert_eq!(backend.mover_pose(42, 1500), None);
    }

    #[test]
    fn landing_on_a_brush_model_names_its_entity() {
        let brushes = [floor()];
        // The world (model 0), then the lift: the floor brush.
        let cmodels = [
            asset_world::ClipCmodel::default(),
            asset_world::ClipCmodel {
                mins: [-32.0, -32.0, -8.0],
                maxs: [32.0, 32.0, 0.0],
                radius: 46.0,
                first_brush: 0,
                num_brushes: 1,
            },
        ];
        let store = crate::clip_fallback::clip_store(&brushes, &cmodels, &[0]);
        let input = GroundTraceInput {
            start: [0.0, 0.0, 50.0],
            end: [0.0, 0.0, -50.0],
            mins: [-15.0, -15.0, 0.0],
            maxs: [15.0, 15.0, 70.0],
            tracemask: crate::bullet_collision::CONTENTS_SOLID,
        };
        let lift = LinkedBrushCollisionBrush {
            cmodel_handle: 1,
            origin: [0.0; 3],
            angles: [0.0; 3],
        };
        let open = Trace::new();
        let hit = store.with_map(|map| clip_move_to_bmodels(open, Some(map), [(40, &lift)], input));
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!((hit.hit_type, hit.hit_id), (TraceHitType::Entity, 40));
        // A brush model nobody numbered stays the world's.
        let hit = store.with_map(|map| {
            clip_move_to_bmodels(open, Some(map), [(trace_t6::ENTITYNUM_WORLD, &lift)], input)
        });
        assert!(hit.fraction < 1.0, "{hit:?}");
        assert_eq!(
            trace_t6::trace_get_entity_hit_id(&hit),
            trace_t6::ENTITYNUM_WORLD
        );
    }

    /// bo2mp: on a BO2 map a body's temporary box stops a line at its
    /// near face, turned or not, with the hit's normal facing the line.
    #[test]
    fn a_bo2_temp_box_stops_a_line_at_its_near_face() {
        use crate::world::{T6Model, t6_transformed_trace};
        let map = clipmap_t6::ClipMap::default();
        let body = T6Model::TempBox {
            mins: [-5.0; 3],
            maxs: [5.0; 3],
            contents: 1,
        };
        for angles in [[0.0; 3], [0.0, 90.0, 0.0]] {
            let hit = t6_transformed_trace(
                &map,
                [0.0; 3],
                [20.0, 0.0, 0.0],
                [0.0; 3],
                [0.0; 3],
                body,
                u32::MAX,
                [10.0, 0.0, 0.0],
                angles,
            );
            assert!((hit.fraction - 0.25).abs() < 0.01, "{angles:?} {hit:?}");
            assert!(hit.normal[0] < -0.99, "{angles:?} {hit:?}");
            assert!((hit.end_pos([0.0; 3], [20.0, 0.0, 0.0])[0] - 5.0).abs() < 0.2, "{angles:?} {hit:?}");
        }
    }

    /// A driven Dragonfire flown at a player stops at his body (both
    /// collisions), and its own number's box never stops it.
    #[test]
    fn a_vehicle_stops_at_a_players_body_not_its_own() {
        use movement_t6::gjk::MoverBrush;
        use movement_t6::heli::{HeliCollision, HeliControls, HeliDef, HeliState, heli_move};
        let body = |entnum: u16, x: f32| PlayerBodyClip {
            entnum,
            origin: [x, 0.0, 0.0],
            bounds: None,
            owner: entnum,
            contents: crate::world::CONTENTS_BODY,
        };
        let def = HeliDef {
            max_speed: 352.0,
            accel: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            rot_rate: 200.0,
            rot_accel: 1500.0,
            max_body_pitch: 20.0,
            max_body_roll: 25.0,
            tilt_from_accel: [0.01, 0.01],
            tilt_from_decel: [0.05, 0.2],
            tilt_speed: [5.0, 5.0],
            decel: [2.0, 2.0, 2.5],
            tilt_from_axes: 0.4,
            tilt_from_fwd_yaw: 0.5,
            vel_at_max_tilt: 0.75,
            tilt_momentum: 0.1,
            ..HeliDef::default()
        };
        let brush = MoverBrush::new(
            &(0..8)
                .map(|c: u32| {
                    std::array::from_fn(|k| if c >> k & 1 == 0 { [-16.0, -16.0, -10.0][k] } else { [16.0, 16.0, 10.0][k] })
                })
                .collect::<Vec<_>>(),
        )
        .expect("a brush");
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        for gjk in [false, true] {
            // Player 3 stands 300 ahead; the drone (number 90) has a box of
            // its own where it starts.
            let clip = VehicleClip {
                bodies: vec![body(3, 300.0), body(90, 0.0)],
                linked_brushes: Vec::new(),
                blockers: Vec::new(),
                movers: Vec::new(),
                glass_damage: Vec::new(),
                self_entnum: 90,
                vehicles: Vec::new(),
            };
            let backend = clip.backend(&[], &bsp, &mesh, &[], gjk);
            let flight = HeliCollision { collision: &backend, brush, gjk };
            let mut st = HeliState { origin: [0.0, 0.0, 40.0], ..HeliState::default() };
            let ahead = HeliControls { forward: 127, ..HeliControls::default() };
            for _ in 0..60 {
                heli_move(&def, &mut st, &ahead, 0.05, &flight);
            }
            // It left its own box and stopped at him (his body is a capsule
            // 15 round, so its corners reach a little past his side, 285).
            assert!(st.origin[0] > 250.0, "{gjk} it left its own box: {st:?}");
            assert!(st.origin[0] < 285.0 && st.velocity[0].abs() < 1.0, "{gjk} it passed him: {st:?}");
        }
    }

    /// A driven Dragonfire flown at another vehicle, a long one turned a
    /// quarter across its way, stops at its turned side (both collisions);
    /// a vehicle never stops at its own.
    #[test]
    fn a_vehicle_stops_at_another_vehicle_turned_to_its_angles() {
        use movement_t6::gjk::MoverBrush;
        use movement_t6::heli::{HeliCollision, HeliControls, HeliDef, HeliState, heli_move};
        let def = HeliDef {
            max_speed: 352.0,
            accel: 440.0,
            max_speed_vertical: 193.6,
            accel_vertical: 369.6,
            rot_rate: 200.0,
            rot_accel: 1500.0,
            max_body_pitch: 20.0,
            max_body_roll: 25.0,
            tilt_from_accel: [0.01, 0.01],
            tilt_from_decel: [0.05, 0.2],
            tilt_speed: [5.0, 5.0],
            decel: [2.0, 2.0, 2.5],
            tilt_from_axes: 0.4,
            tilt_from_fwd_yaw: 0.5,
            vel_at_max_tilt: 0.75,
            tilt_momentum: 0.1,
            ..HeliDef::default()
        };
        let corners = |lo: [f32; 3], hi: [f32; 3]| -> Vec<[f32; 3]> {
            (0..8)
                .map(|c: u32| std::array::from_fn(|k| if c >> k & 1 == 0 { lo[k] } else { hi[k] }))
                .collect()
        };
        let brush = MoverBrush::new(&corners([-16.0, -16.0, -10.0], [16.0, 16.0, 10.0])).expect("a brush");
        // 100 long, 32 wide, at 240 and turned a quarter: its near side is
        // at 224 (unturned it would be at 190).
        let other = OtherVehicle::new(
            9,
            [240.0, 0.0, 40.0],
            [0.0, 90.0, 0.0],
            corners([-50.0, -16.0, -10.0], [50.0, 16.0, 10.0]),
            true,
        )
        .expect("a vehicle");
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        for gjk in [false, true] {
            for own in [90, 9] {
                let clip = VehicleClip {
                    bodies: Vec::new(),
                    linked_brushes: Vec::new(),
                    blockers: Vec::new(),
                    movers: Vec::new(),
                    glass_damage: Vec::new(),
                    self_entnum: own,
                    vehicles: vec![other.clone()],
                };
                let backend = clip.backend(&[], &bsp, &mesh, &[], gjk);
                let flight = HeliCollision { collision: &backend, brush, gjk };
                let mut st = HeliState { origin: [0.0, 0.0, 40.0], ..HeliState::default() };
                let ahead = HeliControls { forward: 127, ..HeliControls::default() };
                for _ in 0..60 {
                    heli_move(&def, &mut st, &ahead, 0.05, &flight);
                }
                if own == 9 {
                    assert!(st.origin[0] > 300.0, "{gjk} its own stopped it: {st:?}");
                } else {
                    // Its brush reaches 16 ahead; the box traces' shape only
                    // 10 (theirs is a capsule as wide as it is tall).
                    let reach = if gjk { 16.0 } else { 10.0 };
                    assert!(st.origin[0] > 200.0, "{gjk} it stopped at the unturned box: {st:?}");
                    assert!(
                        st.origin[0] < 224.5 - reach && st.velocity[0].abs() < 1.0,
                        "{gjk} it passed it: {st:?}"
                    );
                }
            }
        }
        // Only the other one is kept.
        let mine = OtherVehicle::new(90, [0.0; 3], [0.0; 3], corners([-1.0; 3], [1.0; 3]), false);
        let clip = VehicleClip {
            bodies: Vec::new(),
            linked_brushes: Vec::new(),
            blockers: Vec::new(),
            movers: Vec::new(),
            glass_damage: Vec::new(),
            self_entnum: 90,
            vehicles: Vec::new(),
        }
        .with_vehicles(vec![other, mine.expect("a vehicle")]);
        assert_eq!(clip.vehicles.iter().map(|v| v.entnum).collect::<Vec<_>>(), [9]);
        assert!(OtherVehicle::new(5, [0.0; 3], [0.0; 3], Vec::new(), false).is_none());
    }

    #[test]
    fn a_climbers_landing_stops_the_others_not_him() {
        let landing = PlayerBodyClip {
            entnum: trace_t6::ENTITYNUM_WORLD,
            origin: [0.0; 3],
            bounds: None,
            owner: 1,
            contents: crate::world::CONTENTS_PLAYERCLIP,
        };
        let bodies = [landing];
        let (bsp, mesh) = (SimClipBsp::default(), SimClipMesh::default());
        let backend = |self_entnum| ClipBackend {
            brushes: &[],
            bsp: &bsp,
            mesh: &mesh,
            glass_damage: &[],
            bodies: &bodies,
            self_entnum,
            cmodels: &[],
            linked_brushes: &[],
            linked_owners: &[],
            struck: None,
            touched: None,
            blockers: &[],
            movers: &[],
            vehicles: &[],
            gjk: false,
        };
        let walk = |tracemask| GroundTraceInput {
            start: [-100.0, 0.0, 0.0],
            end: [100.0, 0.0, 0.0],
            mins: PLAYER_MINS,
            maxs: PLAYER_MAXS,
            tracemask,
        };
        // Another player walking across the spot stops at it, on no player.
        let hit = backend(2).trace(walk(0x0281_0011));
        assert!(hit.fraction < 0.5, "{hit:?}");
        assert_ne!(
            trace_t6::trace_get_entity_hit_id(&hit),
            2,
            "a hit on it names no player"
        );
        assert!(trace_t6::trace_get_entity_hit_id(&hit) >= 64);
        // The climber passes through his own.
        assert_eq!(backend(1).trace(walk(0x0281_0011)).fraction, 1.0);
        // It is player clip: a trace that only bodies stop ignores it.
        assert_eq!(backend(2).trace(walk(crate::world::CONTENTS_BODY)).fraction, 1.0);
    }
}

#[cfg(test)]
mod feel_tests {
    use super::bo2_feel_rules;

    /// With no scripts the rules are BO2 multiplayer's own defaults (what a
    /// guess made before the first snapshot now uses instead of MW2's).
    #[test]
    fn no_scripts_is_bo2_multiplayer() {
        let feel = bo2_feel_rules(|_| None, false);
        assert_eq!(feel, movement_t6::bridge::Bo2Feel::BO2_MP);
        assert!(!feel.zombies && !feel.omni && feel.jump_slowdown && feel.dive);
        assert_eq!(feel.sprint_strafe_speed_scale, 0.667);
        assert_eq!((feel.gravity, feel.g_speed, feel.jump_height), (800, 190.0, 39.0));
        assert_eq!((feel.fall_damage_min_height, feel.fall_damage_max_height), (128.0, 300.0));
        assert_eq!((feel.sprint_multiplier, feel.speed_multiplier, feel.fast_ads_multiplier), (2.0, 1.07, 0.5));
        assert_eq!(feel.fall_damage_perk, playerstate_t6::perks::PERK_FALLHEIGHT);
    }

    /// A game's scripts still win over every default, as before.
    #[test]
    fn script_dvars_win() {
        let dvars = [("mapname", "zm_transit"), ("bg_gravity", " 400 "), ("g_speed", "200"), ("dtp", "0")];
        let feel = bo2_feel_rules(|name| dvars.iter().find(|(n, _)| *n == name).map(|(_, v)| *v), true);
        assert!(feel.zombies && feel.omni && !feel.jump_slowdown && !feel.dive);
        assert_eq!((feel.gravity, feel.g_speed), (400, 200.0));
    }
}
