//! The last two systems of a step: the one that closes the frame (remembers what each player
//! pressed, settles the events the movement raised) and the one that hands out the snapshot.

use bevy_ecs::prelude::World;
use movement_t6::bridge::drain_player_happenings;
use playerstate_t6::game::{AvatarState, InputCommand, buttons};

use super::hazard::{apply_fall_damage, drown_players};
use super::{StallGuard, FrameTicket, step_view_of, is_alive};
use crate::step_view::StepWorldView;
use crate::ent_slots::EntityPassStage;
use crate::player_ledger::{PlayerSessionStage, EventRecipients, MatchOccurrence};
use crate::tick_publish::TickSnapshot;
use crate::sim_parts::{SeatNo, StepNo};

/// Closes the frame: tells clients when they let go of the trigger, records the commands as the
/// "previous" ones for the next frame, forgets the players who left, and settles the events the
/// movement code raised.
pub(super) fn finalize_system(ecs: &mut World) {
    let _stall = StallGuard::new("finalize");
    let (tick, commands) = {
        let request = ecs.resource::<FrameTicket>();
        (request.tick, request.input.cmds.clone())
    };
    let mut world = step_view_of(ecs);
    world.begin_pass_stage(EntityPassStage::Finalize);

    for &(id, input) in &commands {
        announce_trigger_release(&mut world, tick, id, &input);
        if world.client_meta(id).is_none() {
            continue;
        }
        world.store_prior_input(id, input.buttons, input.angles);
        world.set_old_cmd_move(id, [input.thrust_ahead, input.thrust_lateral]);
    }
    forget_departed_players(&mut world);

    settle_movement_events(&mut world, tick);
    drown_players(&mut world, tick);
}

/// Drops the remembered commands of every client that is no longer in the match.
fn forget_departed_players(world: &mut StepWorldView) {
    let mut buttons_held = std::mem::take(world.prev_button_state_mut());
    buttons_held.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.prev_button_state_mut() = buttons_held;

    let mut angles = std::mem::take(world.prev_input_angles_mut());
    angles.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.prev_input_angles_mut() = angles;

    let mut moves = std::mem::take(world.old_cmd_moves_mut());
    moves.retain(|(id, _)| world.client_meta(*id).is_some());
    *world.old_cmd_moves_mut() = moves;
}

/// Tells a living client that its fire button just came up.
fn announce_trigger_release(level: &mut StepWorldView, step: StepNo, id: SeatNo, input: &InputCommand) {
    if !is_alive(level, id) {
        return;
    }
    let held_before = level
        .last_tick_buttons(id)
        .is_some_and(|bits| bits & buttons::ATTACK != 0);
    let held_now = input.buttons & buttons::ATTACK != 0;
    if held_before && !held_now {
        level.log_occurrence(step, EventRecipients::Client(id), MatchOccurrence::TriggerLet);
    }
}

/// Turns the events the movement code queued on each player into their consequences. Each
/// command's are settled right after it runs (`settle_player_events`); this catches any left.
fn settle_movement_events(world: &mut StepWorldView, tick: StepNo) {
    let mut ids = Vec::new();
    world.for_each_player_mut(|id, _| ids.push(id));
    for id in ids {
        settle_player_events(world, tick, id);
    }
}

/// Turns the events one player's moves queued since the last look into their consequences.
/// Today only a hard landing has one: it hurts. Black Ops II does this after every command
/// (`ClientThink_real`: the sequence before `Pmove`, `ClientEvents` after it): the ring holds
/// only four, so a frame's worth of commands (a dive's landing, its slide, the gun) would
/// overwrite the landing before a once-a-frame look saw it.
pub(super) fn settle_player_events(world: &mut StepWorldView, tick: StepNo, id: SeatNo) {
    let Some(pstate) = world.occupant_mut(id) else { return };
    for (parm, hit_point_ceiling, spot) in take_landings(pstate) {
        apply_fall_damage(world, tick, id, parm, hit_point_ceiling, spot);
    }
}

/// The hurting landings queued since the last look (each one's parm, max health and spot);
/// marks everything queued as seen.
fn take_landings(pstate: &mut AvatarState) -> Vec<(i32, i32, [f32; 3])> {
    let mut landings = Vec::new();
    let mut caret = i32::from(pstate.predictable_event_sequence_old);
    drain_player_happenings(pstate, &mut caret, |event| {
        if entity_t6::EventCode(event.event).hurts_on_landing() {
            landings.push((event.event_arg, pstate.hit_point_ceiling, pstate.origin));
        }
    });
    // The sequence is masked to 0x7ff, so it fits BO2's i16.
    pstate.predictable_event_sequence_old = caret as i16;
    landings
}

/// Builds the snapshot of the frame (when this world serves clients) and stores it for the
/// caller of the step.
pub(super) fn publish_snapshot_system(ecs: &mut World) {
    let _stall = StallGuard::new("publish_snapshot");
    let step = ecs.resource::<FrameTicket>().tick;
    let sink = {
        let mut world = step_view_of(ecs);
        world.begin_pass_stage(EntityPassStage::SendOut);
        world
            .emits_state_snapshot()
            .then(|| {
                report_player_rows(&world, step);
                let built = world.snapshot(step);
                report_snapshot_events(&built);
                built
            })
            .unwrap_or_else(|| TickSnapshot::unpublished(step))
    };
    ecs.resource_mut::<FrameTicket>().published = Some(sink);
}

/// Feeds the profiler one row per player.
fn report_player_rows(world: &StepWorldView, tick: StepNo) {
    struct Sample {
        client: SeatNo,
        command_ms: i32,
        pos: [f32; 3],
        climb_rate: f32,
        heading: f32,
        tilt: f32,
        jump_ms: i32,
        weap_state: i32,
        legs: i32,
        torso: i32,
    }

    let bot_name = entity_t6::client_info::strncpyz::<16>("bot");
    let buffered_depth_millis = crate::level_time_ms(tick);
    let mut table: Vec<Sample> = Vec::new();
    world.walk_avatars(|client, ps| {
        table.push(Sample {
            client,
            command_ms: ps.last_cmd_ms,
            pos: ps.origin,
            climb_rate: ps.velocity[2],
            heading: ps.look_angles[1],
            tilt: ps.look_angles[0],
            jump_ms: ps.hop_start_ms,
            weap_state: ps.weapon_state_t6,
            legs: ps.legs_anim,
            torso: ps.torso_anim,
        });
    });
    for entry in table {
        let Some(meta) = world.client_meta(entry.client) else {
            continue;
        };
        let is_bot = meta.name == bot_name;
        let clip = meta.mag_rounds;
        let label = lifecycle_label(meta.lifecycle);
        perf::avatar_step_mark(
            entry.client.0,
            is_bot,
            entry.command_ms,
            buffered_depth_millis,
            entry.pos,
            entry.climb_rate,
            entry.heading,
            entry.tilt,
            entry.jump_ms,
            world.last_tick_buttons(entry.client).unwrap_or(0),
            entry.weap_state,
            clip,
            world.footing_of(entry.client),
            entry.legs,
            entry.torso,
            label,
        );
    }
}

fn lifecycle_label(step_part: PlayerSessionStage) -> &'static str {
    match step_part {
        PlayerSessionStage::Connecting => "Connecting",
        PlayerSessionStage::PickingLoadout => "PickingLoadout",
        PlayerSessionStage::AwaitingFirstLife => "AwaitingFirstLife",
        PlayerSessionStage::Alive => "Alive",
        PlayerSessionStage::Dead => "Dead",
        PlayerSessionStage::AwaitingNextLife => "AwaitingNextLife",
        PlayerSessionStage::Spectating => "Spectating",
        PlayerSessionStage::Intermission => "Intermission",
    }
}

/// Feeds the profiler what the snapshot says about corpses and item pickups.
fn report_snapshot_events(snapshot: &TickSnapshot) {
    let meta = &snapshot.meta;
    for body_left in &meta.corpses.slots {
        perf::corpse(
            i64::from(body_left.occupied),
            body_left.occupied.then_some(body_left.origin[2]),
            None,
            None,
        );
    }
    for entry in meta.entities.iter().filter(|e| e.e_type == entity_t6::et::ITEM) {
        let stock = meta.item_ammo.iter().find(|row| row.entnum == entry.slot_index);
        perf::item(
            i32::from(entry.e_type),
            stock.map(|row| row.clip_r),
            stock.map(|row| row.scavenger),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::take_landings;
    use movement_t6::bridge::queue_forecast_happening;
    use playerstate_t6::game::AvatarState;

    const PAIN: i32 = entity_t6::event::LANDING_PAIN_FIRST as i32;
    const STEP: i32 = entity_t6::event::FOOTSTEP_RUN as i32;

    #[test]
    fn a_landing_is_hurt_by_the_command_that_made_it_not_lost_to_the_next_ones() {
        let mut ps = AvatarState { hit_point_ceiling: 100, ..AvatarState::ZERO };
        // The landing's command: looked at right after it, as BO2 does.
        queue_forecast_happening(&mut ps, PAIN, 40);
        assert_eq!(take_landings(&mut ps), vec![(40, 100, [0.0; 3])]);
        for _ in 0..4 {
            queue_forecast_happening(&mut ps, STEP, 2);
        }
        assert!(take_landings(&mut ps).is_empty(), "a landing hurts once");

        // Looked at once for the whole frame, four later events push it out of the ring.
        queue_forecast_happening(&mut ps, PAIN, 40);
        for _ in 0..4 {
            queue_forecast_happening(&mut ps, STEP, 2);
        }
        assert!(take_landings(&mut ps).is_empty());
    }
}
