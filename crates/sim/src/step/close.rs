//! The last two systems of a step: the one that closes the frame (remembers what each player
//! pressed, settles the events the movement raised) and the one that hands out the snapshot.

use bevy_ecs::prelude::World;
use movement_t6::bridge::consume_player_events;
use playerstate_t6::game::{UserCmd, buttons};

use super::hazard::{apply_fall_damage, drown_players};
use super::{StallGuard, StepRequest, frame_world, is_alive};
use crate::frame::FrameWorld;
use crate::gentity::KernelPhase;
use crate::match_state::{ClientLifecycle, EventAudience, SimEvent};
use crate::snapshot::Snapshot;
use crate::world::{ClientId, Tick};

/// Closes the frame: tells clients when they let go of the trigger, records the commands as the
/// "previous" ones for the next frame, forgets the players who left, and settles the events the
/// movement code raised.
pub(super) fn finalize_system(ecs: &mut World) {
    let _stall = StallGuard::new("finalize");
    let (tick, commands) = {
        let request = ecs.resource::<StepRequest>();
        (request.tick, request.input.cmds.clone())
    };
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::Finalize);

    for &(id, cmd) in &commands {
        announce_trigger_release(&mut world, tick, id, &cmd);
        if world.client_meta(id).is_some() {
            world.set_old_cmd(id, cmd.buttons, cmd.angles);
            world.set_old_cmd_move(id, [cmd.forwardmove, cmd.rightmove]);
        }
    }
    forget_departed_players(&mut world);

    harvest_predictable_events(&mut world, tick);
    drown_players(&mut world, tick);
}

/// Drops the remembered commands of every client that is no longer in the match.
fn forget_departed_players(world: &mut FrameWorld) {
    let mut buttons_held = std::mem::take(world.old_buttons_mut());
    buttons_held.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_buttons_mut() = buttons_held;

    let mut angles = std::mem::take(world.old_cmd_angles_mut());
    angles.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_cmd_angles_mut() = angles;

    let mut moves = std::mem::take(world.old_cmd_moves_mut());
    moves.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_cmd_moves_mut() = moves;
}

/// Tells a living client that its fire button just came up.
fn announce_trigger_release(world: &mut FrameWorld, tick: Tick, id: ClientId, cmd: &UserCmd) {
    if !is_alive(world, id) {
        return;
    }
    let held_before = world
        .old_buttons_mut()
        .iter()
        .find(|(client, _)| *client == id)
        .is_some_and(|(_, bits)| bits & buttons::ATTACK != 0);
    let held_now = cmd.buttons & buttons::ATTACK != 0;
    if held_before && !held_now {
        world.push_event(tick, EventAudience::Client(id), SimEvent::AttackReleased);
    }
}

/// Turns the events the movement code queued on each player into their consequences. Today
/// only a hard landing has one: it hurts.
fn harvest_predictable_events(world: &mut FrameWorld, tick: Tick) {
    let mut landings = Vec::new();
    world.for_each_player_mut(|id, ps| {
        let mut cursor = i32::from(ps.predictable_event_sequence_old);
        consume_player_events(ps, &mut cursor, |event| {
            if entity_t6::EntityEventKind(event.event).is_landing_pain() {
                landings.push((id, event.event_parm, ps.max_health, ps.origin));
            }
        });
        // The sequence is masked to 0x7ff, so it fits BO2's i16.
        ps.predictable_event_sequence_old = cursor as i16;
    });
    for (id, parm, max_health, origin) in landings {
        apply_fall_damage(world, tick, id, parm, max_health, origin);
    }
}

/// Builds the snapshot of the frame (when this world serves clients) and stores it for the
/// caller of the step.
pub(super) fn publish_snapshot_system(ecs: &mut World) {
    let _stall = StallGuard::new("publish_snapshot");
    let tick = ecs.resource::<StepRequest>().tick;
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::PublishSnapshot);

    let snapshot = if world.publishes_snapshot() {
        emit_player_ticks(&world, tick);
        let snapshot = world.snapshot(tick);
        emit_snapshot_events(&snapshot);
        snapshot
    } else {
        Snapshot::unpublished(tick)
    };
    drop(world);
    ecs.resource_mut::<StepRequest>().output = Some(snapshot);
}

/// Feeds the profiler one row per player.
fn emit_player_ticks(world: &FrameWorld, tick: Tick) {
    struct Row {
        id: ClientId,
        time_ms: i32,
        origin: [f32; 3],
        vertical_speed: f32,
        yaw: f32,
        pitch: f32,
        jump_time: i32,
        weapon_state: i32,
        legs_anim: i32,
        torso_anim: i32,
    }

    let bot_name = entity_t6::client_state::strncpyz::<16>("bot");
    let level_time_ms = crate::level_time_ms(tick);
    let mut rows = Vec::new();
    world.visit_players(|id, ps| {
        rows.push(Row {
            id,
            time_ms: ps.command_time,
            origin: ps.origin,
            vertical_speed: ps.velocity[2],
            yaw: ps.viewangles[1],
            pitch: ps.viewangles[0],
            jump_time: ps.jump_time,
            weapon_state: ps.weapon_state_t6,
            legs_anim: ps.legs_anim,
            torso_anim: ps.torso_anim,
        });
    });
    for row in rows {
        let Some(meta) = world.client_meta(row.id) else {
            continue;
        };
        perf::player_tick(
            row.id.0,
            meta.name == bot_name,
            row.time_ms,
            level_time_ms,
            row.origin,
            row.vertical_speed,
            row.yaw,
            row.pitch,
            row.jump_time,
            world.old_buttons(row.id).unwrap_or(0),
            row.weapon_state,
            meta.ammo_clip,
            world.pmove_walking(row.id),
            row.legs_anim,
            row.torso_anim,
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

/// Feeds the profiler what the snapshot says about corpses and item pickups.
fn emit_snapshot_events(snapshot: &Snapshot) {
    for slot in &snapshot.meta.corpses.slots {
        perf::corpse(
            i64::from(slot.occupied),
            slot.occupied.then_some(slot.origin[2]),
            None,
            None,
        );
    }
    let items = snapshot
        .meta
        .entities
        .iter()
        .filter(|state| state.e_type == entity_t6::et::ITEM);
    for state in items {
        let ammo = snapshot
            .meta
            .item_ammo
            .iter()
            .find(|row| row.entnum == state.number);
        perf::item(
            i32::from(state.e_type),
            ammo.map(|row| row.clip_r),
            ammo.map(|row| row.scavenger),
            None,
        );
    }
}
