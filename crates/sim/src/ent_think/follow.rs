//! Riding on a parent: a rider takes the parent's pose of this frame, shifted by the offset it was linked with.

use crate::ent_links::{
    link_entity_needs_rotated_radius, link_entity_world_bounds, parent_link_apply_local, parent_link_pose,
};
use crate::step_view::StepWorldView;
use crate::ent_slots::{EntityBehaviorClass, turn_path_of, move_path_of};
use crate::ent_slots::EntHandle;
use crate::round_clock::SIM_STEP_MILLIS;
use crate::sim_parts::StepNo;
use entity_t6::evaluate_trajectory;
use game_math::round_degrees;

/// Where something is and which way it faces.
#[derive(Clone, Copy)]
struct Pose {
    at: [f32; 3],
    turn: [f32; 3],
}

/// A script mover that stands free turns the animated models on it to its present angles.
pub(super) fn pose_mover_models(world: &mut StepWorldView, step: StepNo, mover_num: i32) {
    let Some(mover) = world.script_mover_by_number(mover_num) else {
        return;
    };
    let owner = mover.id;
    let angles = turn_path_of(&mover.state);
    let at_ms = i32::try_from(step.0.saturating_mul(SIM_STEP_MILLIS)).unwrap_or(i32::MAX);
    for caps in owned_by(world, owner) {
        if let Some(dobj) = caps.dobj.as_mut() {
            dobj.apply_trajectory_apos_at(angles, at_ms);
        }
    }
}

/// The collision parts that belong to the script model `owner`.
fn owned_by<'w>(
    world: &'w mut StepWorldView,
    owner: crate::match_ids::ScriptedModel,
) -> impl Iterator<Item = &'w mut crate::shot_trace::EntityHitAbilities> {
    let full_part_set = world.entity_collision_capabilities_mut();
    full_part_set.iter_mut().filter(move |caps| caps.owner.scripted_model() == Some(owner))
}

/// Puts `rider` where its parent is now, shifted by the offset it was linked with. An entity without a parent stays
/// where it is.
pub(super) fn follow_parent(world: &mut StepWorldView, clock_stamp_ms: i32, rider: EntHandle) {
    let Ok(child) = world.entity_kernel().look_up(rider) else {
        return;
    };
    let Some(parent) = child.ties.parent else {
        return;
    };
    let from = parent_pose(world, parent, clock_stamp_ms).expect("parented think has no parent currentOrigin/angles");
    assert!(
        child.ties.parent_tag < 0,
        "tag-linked think needs a computed bone matrix for the parent tag"
    );
    let (at, turn) = parent_link_apply_local(
        from.at,
        from.turn,
        child.ties.parent_link_axis,
        child.ties.parent_link_origin,
    );
    let slot_num = rider.slot();
    store_pose(world, child.class, slot_num, Pose { at, turn });
}

/// The pose of a parent at `clock_stamp_ms`; only movers and dropped items can be parents.
fn parent_pose(world: &StepWorldView, leader: EntHandle, clock_stamp_ms: i32) -> Option<Pose> {
    let parent_kind = world.entity_kernel().look_up(leader).ok()?.class;
    let number = leader.slot();
    match parent_kind {
        EntityBehaviorClass::MainLightSource | EntityBehaviorClass::ScriptedMover => {
            let state = &world.script_mover_by_number(number)?.state;
            Some(Pose {
                at: evaluate_trajectory(&move_path_of(state), clock_stamp_ms),
                turn: evaluate_trajectory(&turn_path_of(state), clock_stamp_ms),
            })
        }
        EntityBehaviorClass::Item => world.loot_at_slot(number).map(|dropped| Pose {
            at: dropped.origin,
            turn: evaluate_trajectory(&turn_path_of(&dropped.state), clock_stamp_ms),
        }),
        EntityBehaviorClass::Missile | EntityBehaviorClass::TempEvent | EntityBehaviorClass::General | EntityBehaviorClass::FallenPlayerBody => {
            None
        }
    }
}

/// Stores a followed pose in the child's record. Only movers and dropped items keep one.
fn store_pose(world: &mut StepWorldView, kind: EntityBehaviorClass, slot_num: i32, pose: Pose) {
    let Pose { at, turn } = pose;
    match kind {
        EntityBehaviorClass::MainLightSource | EntityBehaviorClass::ScriptedMover => {
            let Some(rider) = world.script_mover_mut_by_number(slot_num) else {
                return;
            };
            parent_link_pose(&mut rider.state, at, turn);
            assert!(
                !link_entity_needs_rotated_radius(round_degrees(turn), rider.box_span),
                "link yaw/pitch radius Bounds when r.half is non-zero"
            );
            let level_bounds = link_entity_world_bounds(at, rider.box_centre, rider.box_span);
            (rider.link_centre, rider.link_span) = (level_bounds.centre, level_bounds.half);
            let owner = rider.id;
            // The mover's brushes travel with it.
            for caps in owned_by(world, owner) {
                for part in caps.attached_solids.iter_mut() {
                    (part.origin, part.turn) = (at, turn);
                }
            }
        }
        EntityBehaviorClass::Item => match world.loot_at_slot_mut(slot_num) {
            Some(dropped) => {
                dropped.origin = at;
                parent_link_pose(&mut dropped.state, at, turn);
            }
            None => {}
        },
        EntityBehaviorClass::Missile | EntityBehaviorClass::TempEvent | EntityBehaviorClass::General | EntityBehaviorClass::FallenPlayerBody => {}
    }
}
