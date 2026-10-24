//! The phases that run between the movement phase and the end of the frame: the collision
//! history, the entity thinks, the touches and use presses, and the little per-frame jobs the
//! movement phase hands over to them.

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gamemode_t6::{G_PLAYER_COLLISION_EJECT_SPEED_DEFAULT, StuckClient, stuck_in_client};
use playerstate_t6::game::{PlayerState, UserCmd, other_flags};

use super::{StallGuard, StepRequest, frame_world, is_alive};
use crate::bullet_collision::{PLAYER_MAXS, PLAYER_MINS};
use crate::frame::FrameWorld;
use crate::gentity::{KernelPhase, apos_from_entity_state};
use crate::identities::MatchPhase;
use crate::world::{ClientId, SimState, Tick};

/// Turns every model's animation tree into the pose the collision code reads.
pub fn phase_materialize_entity_dobjs(world: &mut SimState) {
    world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .filter_map(|capabilities| capabilities.dobj.as_mut())
        .for_each(|dobj| dobj.materialize());
}

/// Milliseconds on the match clock at the start of `tick`, held at the largest value that fits.
fn animation_clock_ms(tick: Tick) -> i32 {
    i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX)
}

/// Moves the animated models of the map on by one frame: models playing an animation advance it,
/// models riding a script mover take the mover's rotation, and models with an own rotation
/// follow it.
fn phase_animated_map_models(world: &mut FrameWorld, tick: Tick, msec: i32) {
    let step_seconds = msec as f32 / 1000.0;
    let clock = animation_clock_ms(tick);

    let mut mover_rotation = Vec::new();
    world.visit_script_movers(|mover| {
        mover_rotation.push((mover.id, apos_from_entity_state(&mover.state)));
    });

    for capabilities in world.entity_collision_capabilities_mut() {
        let riding = capabilities
            .owner
            .script_model()
            .and_then(|model| mover_rotation.iter().find(|(mover, _)| *mover == model))
            .map(|&(_, rotation)| rotation);
        let Some(dobj) = capabilities.dobj.as_mut() else {
            continue;
        };
        if dobj.play_anim.is_some() {
            dobj.advance_script_model_play_anim(step_seconds);
        }
        match riding {
            Some(rotation) => {
                dobj.apply_trajectory_apos_at(rotation, clock);
            }
            None if dobj.apos.is_some() => {
                dobj.apply_apos_at(clock);
            }
            None => {}
        }
    }
}

/// Pushes apart the players who ended the movement phase standing inside each other. Only
/// players who are alive take part; the shove draws from the match's shared random seed.
pub(super) fn phase_stuck_in_client(world: &mut FrameWorld) {
    let ids: Vec<ClientId> = world
        .client_ids_sorted()
        .into_iter()
        .filter(|&id| is_alive(world, id) && world.player(id).is_some())
        .collect();
    let mut bodies: Vec<StuckClient> = ids
        .iter()
        .filter_map(|&id| world.player(id).map(body_of))
        .collect();

    let mut seed = *world.stuck_holdrand_mut();
    for me in 0..bodies.len() {
        let _ = stuck_in_client(me, &mut bodies, G_PLAYER_COLLISION_EJECT_SPEED_DEFAULT, &mut seed);
    }
    *world.stuck_holdrand_mut() = seed;

    for (&id, body) in ids.iter().zip(&bodies) {
        let Some(ps) = world.player_mut(id) else {
            continue;
        };
        ps.velocity[..2].copy_from_slice(&body.velocity[..2]);
        ps.pm_time = body.pm_time;
        ps.pm_flags = body.pm_flags;
    }
}

/// What the shove needs to know about one living player.
fn body_of(ps: &PlayerState) -> StuckClient {
    // The shove also wants the session state and the entity contents, which a player row does
    // not carry; every row that gets here is alive and playing, and a living body is solid to
    // other bodies.
    let middle = |axis: usize| ps.origin[axis] + (PLAYER_MINS[axis] + PLAYER_MAXS[axis]) * 0.5;
    let half = |axis: usize| (PLAYER_MAXS[axis] - PLAYER_MINS[axis]) * 0.5;
    StuckClient {
        origin: ps.origin,
        velocity: ps.velocity,
        speed: ps.speed,
        player: ps.other_flags & other_flags::PLAYER != 0,
        playing: true,
        contents: crate::world::CONTENTS_BODY,
        health: ps.health,
        pm_time: ps.pm_time,
        pm_flags: ps.pm_flags,
        maxs_x: PLAYER_MAXS[0],
        bounds_mid: [0, 1, 2].map(middle),
        bounds_half: [0, 1, 2].map(half),
    }
}

/// Files this frame's player positions in the history that lag compensation rewinds to. A
/// predicting world only keeps the boxes of the players whose commands it ran.
pub(super) fn record_collision_state_system(ecs: &mut World) {
    let _stall = StallGuard::new("record_collision_state");
    let (tick, msec, commanding) = {
        let request = ecs.resource::<StepRequest>();
        let commanding: Vec<ClientId> = request.input.cmds.iter().map(|&(id, _)| id).collect();
        (request.tick, request.msec, commanding)
    };
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::RecordCollisionState);

    let only_these = (!world.publishes_snapshot()).then_some(commanding);
    world.record_collision_history(tick, msec, only_these.as_deref());
}

/// Runs the entities: their thinks, the glass and the animated models while the match is being
/// played, and the thinks that keep a stopped match consistent otherwise.
pub(super) fn run_entity_types_system(ecs: &mut World) {
    let (tick, msec) = {
        let request = ecs.resource::<StepRequest>();
        (request.tick, request.msec)
    };
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::RunEntityTypes);
    if world.phase() != MatchPhase::Playing {
        crate::entity_run::phase_walk_entity_thinks(&mut world);
        return;
    }

    let serving = world.publishes_snapshot();
    {
        let _stall = StallGuard::new("materialize_entity_dobjs");
        phase_materialize_entity_dobjs(&mut world);
    }
    if serving {
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
    if serving {
        {
            let _stall = StallGuard::new("glass");
            world.world_objects_mut().glass_update(animation_clock_ms(tick));
        }
        let _stall = StallGuard::new("animated_map_models");
        phase_animated_map_models(&mut world, tick, msec);
    }
}

/// Lets players pick up what they touch and use what they press the use key at.
pub(super) fn dispatch_touches_system(ecs: &mut World) {
    let _stall = StallGuard::new("dispatch_touches");
    let (tick, commands) = {
        let request = ecs.resource::<StepRequest>();
        (request.tick, request.input.cmds.clone())
    };
    let mut world = frame_world(ecs);
    world.enter_kernel_phase(KernelPhase::DispatchTouches);
    if world.phase() != MatchPhase::Playing {
        return;
    }

    crate::item::phase_touch_items(&mut world);
    if !world.publishes_snapshot() {
        return;
    }

    let held_before: Vec<(u32, u32)> = world
        .old_buttons_mut()
        .iter()
        .map(|&(id, bits)| (id.0, bits))
        .collect();
    let every_command: Vec<(u32, u32)> = commands.iter().map(|(id, cmd)| (id.0, cmd.buttons)).collect();
    let presses = crate::collect_use_presses(&every_command, &held_before);

    // A client that sent several commands is judged by its last one.
    let newest: BTreeMap<ClientId, UserCmd> = commands.into_iter().collect();
    let newest_buttons: Vec<(u32, u32)> = newest.iter().map(|(id, cmd)| (id.0, cmd.buttons)).collect();
    crate::item::phase_use_items(&mut world, tick, &presses, &newest_buttons);
}

// The shove must read the rows exactly as the movement phase left them.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_clock_saturates_instead_of_wrapping() {
        assert_eq!(animation_clock_ms(Tick(0)), 0);
        assert_eq!(animation_clock_ms(Tick(10)), 10 * crate::MATCH_TICK_MS as i32);
        assert_eq!(animation_clock_ms(Tick(u32::MAX)), i32::MAX);
    }

    #[test]
    fn a_body_row_is_centred_on_the_player_origin() {
        let mut ps = PlayerState::ZERO;
        ps.origin = [100.0, -50.0, 10.0];
        let row = body_of(&ps);
        assert_eq!(row.bounds_mid[0], 100.0 + (PLAYER_MINS[0] + PLAYER_MAXS[0]) * 0.5);
        assert_eq!(row.bounds_half[2], (PLAYER_MAXS[2] - PLAYER_MINS[2]) * 0.5);
        assert!(row.playing);
    }
}
