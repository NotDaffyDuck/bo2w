//! The phases that run between the movement phase and the end of the frame: the collision
//! history, the entity thinks, the touches and use presses, and the little per-frame jobs the
//! movement phase hands over to them.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gamemode_t6::{SHOVE_SPEED_BASE, ShoveBody, shove_apart};
use playerstate_t6::game::{AvatarState, InputCommand, secondary_state_bits};

use super::{StallGuard, FrameTicket, step_view_of, is_alive};
use crate::shot_trace::{BODY_BOX_UPPER, BODY_BOX_LOWER};
use crate::step_view::StepWorldView;
use crate::ent_slots::{EntityPassStage, turn_path_of};
use crate::match_ids::GameRoundStage;
use crate::sim_parts::{SeatNo, MatchSimulation, StepNo};

/// Turns every model's animation tree into the pose the collision code reads.
pub fn stage_build_entity_models(sim: &mut MatchSimulation) {
    for caps in sim.entity_collision_capabilities_mut().iter_mut() {
        if let Some(tree) = caps.dobj.as_mut() {
            tree.bake_collision_shape();
        }
    }
}

/// Milliseconds on the match clock at the start of `tick`, held at the largest value that fits.
fn animation_clock_ms(tick: StepNo) -> i32 {
    i32::try_from(tick.0.saturating_mul(crate::SIM_STEP_MILLIS)).unwrap_or(i32::MAX)
}

/// Moves the animated models of the map on by one frame: models playing an animation advance it,
/// models riding a script mover take the mover's rotation, and models with an own rotation
/// follow it.
fn advance_map_models(world: &mut StepWorldView, tick: StepNo, msec: i32) {
    let step_seconds = msec as f32 / 1000.0;
    let clock = animation_clock_ms(tick);

    let mut mover_rotation = Vec::new();
    world.each_mover_row(|mv| {
        mover_rotation.push((mv.id, turn_path_of(&mv.state)));
    });

    for caps in world.entity_collision_capabilities_mut() {
        let carried_model = caps.owner.scripted_model();
        let riding = carried_model
            .and_then(|model| mover_rotation.iter().find(|(mover, _)| *mover == model))
            .map(|&(_, rotation)| rotation);
        let Some(tree) = caps.dobj.as_mut() else {
            continue;
        };
        if tree.scripted_anim_playback.is_some() {
            tree.step_scripted_model_anim(step_seconds);
        }
        match riding {
            Some(rotation) => {
                tree.apply_trajectory_apos_at(rotation, clock);
            }
            None if tree.apos.is_some() => {
                tree.sample_scripted_motion_at(clock);
            }
            None => {}
        }
    }
}

/// Pushes apart the players who ended the movement phase standing inside each other. Only
/// players who are alive take part; the shove draws from the match's shared random seed.
pub(super) fn stage_unstick_players(world: &mut StepWorldView) {
    let mut ids = world.ordered_player_ids();
    ids.retain(|&id| is_alive(world, id) && world.avatar(id).is_some());
    let mut bodies: Vec<ShoveBody> = ids
        .iter()
        .filter_map(|&id| world.avatar(id).map(body_of))
        .collect();

    let mut seed = *world.stuck_holdrand_mut();
    for me in 0..bodies.len() {
        let _ = shove_apart(me, &mut bodies, SHOVE_SPEED_BASE, &mut seed);
    }
    *world.stuck_holdrand_mut() = seed;

    for (&id, body) in ids.iter().zip(&bodies) {
        let Some(player) = world.occupant_mut(id) else {
            continue;
        };
        player.velocity[..2].copy_from_slice(&body.velocity[..2]);
        player.move_timer = body.move_lock_ms;
        player.movement_state_bits = body.movement_state_bits;
    }
}

/// What the shove needs to know about one living player.
fn body_of(player: &AvatarState) -> ShoveBody {
    // The shove also wants the session state and the entity contents, which a player row does
    // not carry; every row that gets here is alive and playing, and a living body is solid to
    // other bodies.
    let middle = |axis: usize| player.origin[axis] + (BODY_BOX_LOWER[axis] + BODY_BOX_UPPER[axis]) * 0.5;
    let [half_width, ..] = BODY_BOX_UPPER;
    let half = |axis: usize| (BODY_BOX_UPPER[axis] - BODY_BOX_LOWER[axis]) * 0.5;
    ShoveBody {
        origin: player.origin,
        velocity: player.velocity,
        speed: player.speed,
        player: player.secondary_state_bits & secondary_state_bits::PLAYER != 0,
        playing: true,
        contents: crate::sim_parts::SOLID_PLAYER_BODY,
        health: player.health,
        move_lock_ms: player.move_timer,
        movement_state_bits: player.movement_state_bits,
        half_width,
        body_mid: [0, 1, 2].map(middle),
        body_half: [0, 1, 2].map(half),
    }
}

/// Files this frame's player positions in the history that lag compensation rewinds to. A
/// predicting world only keeps the boxes of the players whose commands it ran.
pub(super) fn record_collision_state_system(ecs: &mut World) {
    let _stall = StallGuard::new("record_collision_state");
    let (step, msec, commanding) = {
        let request = ecs.resource::<FrameTicket>();
        let commanding: Vec<SeatNo> = request.input.cmds.iter().map(|&(id, _)| id).collect();
        (request.tick, request.msec, commanding)
    };
    let mut level = step_view_of(ecs);
    level.begin_pass_stage(EntityPassStage::StoreCollision);

    let only_these = (!level.emits_state_snapshot()).then_some(commanding);
    level.record_collision_history(step, msec, only_these.as_deref());
}

/// Runs the entities: their thinks, the glass and the animated models while the match is being
/// played, and the thinks that keep a stopped match consistent otherwise.
pub(super) fn run_entity_types_system(ecs: &mut World) {
    let (tick, msec) = {
        let request = ecs.resource::<FrameTicket>();
        (request.tick, request.msec)
    };
    let mut level = step_view_of(ecs);
    level.begin_pass_stage(EntityPassStage::ThinkEntities);
    if level.phase() != GameRoundStage::Playing {
        crate::ent_think::walk_thinks(&mut level);
        return;
    }

    let serving = level.emits_state_snapshot();
    {
        let _stall = StallGuard::new("materialize_entity_dobjs");
        stage_build_entity_models(&mut level);
    }
    if serving {
        let _stall = StallGuard::new("entity_collision_history");
        level.log_entity_hitboxes(tick);
    }
    {
        let _stall = StallGuard::new("remote_missiles");
        crate::rider_missile::advance(&mut level, tick);
    }
    {
        let _stall = StallGuard::new("entity_thinks");
        crate::ent_think::run_due_thinks(&mut level, tick);
    }
    if serving {
        {
            let _stall = StallGuard::new("glass");
            level.world_objects_mut().bleed_glass(animation_clock_ms(tick));
        }
        let _stall = StallGuard::new("animated_map_models");
        advance_map_models(&mut level, tick, msec);
    }
}

/// Lets players pick up what they touch and use what they press the use key at.
pub(super) fn dispatch_touches_system(ecs: &mut World) {
    let _stall = StallGuard::new("dispatch_touches");
    let (tick, commands) = {
        let request = ecs.resource::<FrameTicket>();
        (request.tick, request.input.cmds.clone())
    };
    let mut level = step_view_of(ecs);
    level.begin_pass_stage(EntityPassStage::TouchPass);
    if level.phase() != GameRoundStage::Playing {
        return;
    }

    crate::floor_guns::phase_touch_items(&mut level);
    if !level.emits_state_snapshot() {
        return;
    }

    let prior_rows = level.prev_button_state_mut();
    let held_before: Vec<(u32, u32)> = prior_rows.iter().map(|&(seat, bits)| (seat.0, bits)).collect();
    let every_command: Vec<(u32, u32)> = commands.iter().map(|(seat, order)| (seat.0, order.buttons)).collect();
    let presses = crate::gather_presses(&every_command, &held_before);

    // A client that sent several commands is judged by its last one.
    let newest: BTreeMap<SeatNo, InputCommand> = commands.into_iter().collect();
    let newest_buttons: Vec<(u32, u32)> = newest.iter().map(|(seat, order)| (seat.0, order.buttons)).collect();
    crate::floor_guns::phase_use_items(&mut level, tick, &presses, &newest_buttons);
}

// The shove must read the rows exactly as the movement phase left them.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_clock_saturates_instead_of_wrapping() {
        assert_eq!(animation_clock_ms(StepNo(0)), 0);
        assert_eq!(animation_clock_ms(StepNo(10)), 10 * crate::SIM_STEP_MILLIS as i32);
        assert_eq!(animation_clock_ms(StepNo(u32::MAX)), i32::MAX);
    }

    #[test]
    fn a_body_row_is_centred_on_the_player_origin() {
        let mut ps = AvatarState::ZERO;
        ps.origin = [100.0, -50.0, 10.0];
        let row = body_of(&ps);
        assert_eq!(row.body_mid[0], 100.0 + (BODY_BOX_LOWER[0] + BODY_BOX_UPPER[0]) * 0.5);
        assert_eq!(row.body_half[2], (BODY_BOX_UPPER[2] - BODY_BOX_LOWER[2]) * 0.5);
        assert!(row.playing);
    }
}
