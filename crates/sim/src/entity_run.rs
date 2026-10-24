//! Once per tick every live entity gets a turn to think, and a parent gets its turn before the entities that ride
//! on it.
//!
//! This file settles who goes when, and keeps anything from thinking twice in one frame. What a think does is
//! decided per entity kind in [`take_turn`].

use crate::corpse::{level_time_ms, phase_run_corpse_move};
use crate::entity_local::{
    entnum_matched, link_entity_needs_rotated_radius, link_entity_world_bounds, parent_link_apply_local,
    parent_link_pose,
};
use crate::frame::FrameWorld;
use crate::gentity::{EntityRunKind, ThinkEnter, apos_from_entity_state, pos_from_entity_state};
use crate::{EntityRef, GENTITY_RESERVED_COUNT, MATCH_TICK_MS, Tick};
use entity_t6::evaluate_trajectory;
use game_math::snap_angles;

type Vec3 = [f32; 3];

/// Why the entities are being walked.
#[derive(Clone, Copy)]
enum Walk {
    /// A real tick, in which each kind runs its own think.
    Think { tick: Tick, time_ms: i32 },
    /// Every entity just spends its turn for this frame, so that it cannot be given one again before the frame ends.
    SpendTurns,
}

/// One tick of thinking: falling bodies move first, then each entity takes its turn.
pub(crate) fn phase_run_entity_thinks(world: &mut FrameWorld, tick: Tick) {
    let time_ms = level_time_ms(tick);
    phase_run_corpse_move(world, time_ms);
    walk_entities(world, Walk::Think { tick, time_ms });
}

/// Spends the turn of every entity for this frame; nothing runs.
pub(crate) fn phase_walk_entity_thinks(world: &mut FrameWorld) {
    walk_entities(world, Walk::SpendTurns);
}

fn walk_entities(world: &mut FrameWorld, how: Walk) {
    // The end is looked up for every entity because a think can create more of them, and they take their turn in
    // this very walk.
    for number in GENTITY_RESERVED_COUNT.. {
        if number >= world.entity_kernel().high_water() {
            break;
        }
        take_turn(world, number, how);
    }
}

fn take_turn(world: &mut FrameWorld, number: i32, how: Walk) {
    let (entity, parent) = match world.entity_kernel_mut().begin_think(number) {
        Ok(ThinkEnter::Run { entity, parent, .. }) => (entity, parent),
        Ok(ThinkEnter::Skip) | Err(_) => return,
    };
    // Whatever rides on a parent needs the parent's pose from this frame.
    if let Some(parent) = parent {
        take_turn(world, parent.number(), how);
    }
    let Walk::Think { tick, time_ms } = how else {
        return;
    };
    let Ok(view) = world.entity_kernel().resolve(entity) else {
        return;
    };
    let has_parent = view.relations.parent.is_some();
    let number = entity.number();

    match view.kind {
        EntityRunKind::Missile => crate::equipment::think_projectile(world, tick, number),
        EntityRunKind::Item if has_parent => {
            follow_parent(world, time_ms, entity);
            settle_think(world, entity);
        }
        EntityRunKind::Item => {
            follow_parent(world, time_ms, entity);
            crate::item::think_item_move(world, time_ms, number);
        }
        EntityRunKind::ScriptMover | EntityRunKind::PrimaryLight if has_parent => {
            follow_parent(world, time_ms, entity);
            settle_think(world, entity);
        }
        EntityRunKind::ScriptMover | EntityRunKind::PrimaryLight => pose_mover_models(world, tick, number),
        EntityRunKind::TempEvent => {}
        EntityRunKind::General => {
            follow_parent(world, time_ms, entity);
            settle_think(world, entity);
        }
        EntityRunKind::PlayerCorpse => {
            let numbers = world.corpses().entnums();
            assert!(
                entnum_matched(&numbers, number),
                "ET_PLAYER_CORPSE occupancy number not in the corpse table; a miss falls back to slot 0"
            );
            settle_think(world, entity);
        }
    }
}

/// An entity that has nothing scheduled has nothing to do. If its scheduled time has come, it would need a handler,
/// which these plain kinds do not have.
fn settle_think(world: &mut FrameWorld, entity: EntityRef) {
    let was_due = world
        .entity_kernel_mut()
        .take_due_think(entity)
        .expect("scheduled think occupancy vanished");
    assert!(!was_due, "generic think has no handler");
}

/// A script mover that stands free turns the animated models on it to its present angles.
fn pose_mover_models(world: &mut FrameWorld, tick: Tick, number: i32) {
    let Some(mover) = world.script_mover_by_number(number) else {
        return;
    };
    let (owner, angles) = (mover.id, apos_from_entity_state(&mover.state));
    let at_ms = i32::try_from(tick.0.saturating_mul(MATCH_TICK_MS)).unwrap_or(i32::MAX);
    let owned = world
        .entity_collision_capabilities_mut()
        .iter_mut()
        .filter(|caps| caps.owner.script_model() == Some(owner));
    for caps in owned {
        if let Some(dobj) = caps.dobj.as_mut() {
            dobj.apply_trajectory_apos_at(angles, at_ms);
        }
    }
}

/// Puts `entity` where its parent is now, shifted by the offset it was linked with. An entity without a parent stays
/// where it is.
fn follow_parent(world: &mut FrameWorld, time_ms: i32, entity: EntityRef) {
    let Ok(view) = world.entity_kernel().resolve(entity) else {
        return;
    };
    let Some(parent) = view.relations.parent else {
        return;
    };
    let (parent_origin, parent_angles) =
        parent_pose(world, parent, time_ms).expect("parented think has no parent currentOrigin/angles");
    assert!(
        view.relations.parent_tag < 0,
        "tag-linked think needs a computed bone matrix for the parent tag"
    );
    let (origin, angles) = parent_link_apply_local(
        parent_origin,
        parent_angles,
        view.relations.parent_link_axis,
        view.relations.parent_link_origin,
    );
    place(world, entity.number(), view.kind, origin, angles);
}

/// The origin and angles of a parent at `time_ms`; only movers and dropped items can be parents.
fn parent_pose(world: &FrameWorld, parent: EntityRef, time_ms: i32) -> Option<(Vec3, Vec3)> {
    let kind = world.entity_kernel().resolve(parent).ok()?.kind;
    let number = parent.number();
    match kind {
        EntityRunKind::ScriptMover | EntityRunKind::PrimaryLight => {
            let state = &world.script_mover_by_number(number)?.state;
            let origin = evaluate_trajectory(&pos_from_entity_state(state), time_ms);
            Some((origin, evaluate_trajectory(&apos_from_entity_state(state), time_ms)))
        }
        EntityRunKind::Item => {
            let item = world.dropped_item_by_number(number)?;
            let angles = evaluate_trajectory(&apos_from_entity_state(&item.state), time_ms);
            Some((item.origin, angles))
        }
        EntityRunKind::Missile | EntityRunKind::TempEvent | EntityRunKind::General | EntityRunKind::PlayerCorpse => {
            None
        }
    }
}

/// Stores a followed pose in the record of the child. Only movers and dropped items keep one.
fn place(world: &mut FrameWorld, number: i32, kind: EntityRunKind, origin: Vec3, angles: Vec3) {
    match kind {
        EntityRunKind::ScriptMover | EntityRunKind::PrimaryLight => {
            let Some(mover) = world.script_mover_mut_by_number(number) else {
                return;
            };
            parent_link_pose(&mut mover.state, origin, angles);
            assert!(
                !link_entity_needs_rotated_radius(snap_angles(angles), mover.box_half),
                "link yaw/pitch radius Bounds when r.half is non-zero"
            );
            let bounds = link_entity_world_bounds(origin, mover.box_mid, mover.box_half);
            mover.link_mid = bounds.mid;
            mover.link_half = bounds.half;
            let owner = mover.id;
            // The mover's brushes travel with it.
            let owned = world
                .entity_collision_capabilities_mut()
                .iter_mut()
                .filter(|caps| caps.owner.script_model() == Some(owner));
            for caps in owned {
                for brush in &mut caps.linked_brushes {
                    brush.origin = origin;
                    brush.angles = angles;
                }
            }
        }
        EntityRunKind::Item => {
            if let Some(item) = world.dropped_item_mut_by_number(number) {
                item.origin = origin;
                parent_link_pose(&mut item.state, origin, angles);
            }
        }
        EntityRunKind::Missile | EntityRunKind::TempEvent | EntityRunKind::General | EntityRunKind::PlayerCorpse => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimWorld;
    use crate::corpse::PlayerCorpseSlot;

    /// A world whose clock has started its first frame, so entities can think.
    fn started() -> SimWorld {
        let mut world = SimWorld::new();
        world.frame().entity_kernel_mut().begin_frame(50);
        world
    }

    fn spawn(world: &mut SimWorld, kind: EntityRunKind) -> EntityRef {
        world.frame().entity_kernel_mut().allocate(kind).expect("room for an entity")
    }

    /// True if entity `entity` is still allowed to think this frame.
    fn can_still_think(world: &mut SimWorld, entity: EntityRef) -> bool {
        matches!(
            world.frame().entity_kernel_mut().begin_think(entity.number()),
            Ok(ThinkEnter::Run { .. })
        )
    }

    #[test]
    fn marking_walk_uses_up_every_entitys_turn() {
        let mut world = started();
        let general = spawn(&mut world, EntityRunKind::General);
        let event = spawn(&mut world, EntityRunKind::TempEvent);

        phase_walk_entity_thinks(&mut world.frame());

        assert!(!can_still_think(&mut world, general));
        assert!(!can_still_think(&mut world, event));
    }

    #[test]
    fn untouched_entities_can_think_before_the_walk() {
        let mut world = started();
        let general = spawn(&mut world, EntityRunKind::General);

        assert!(can_still_think(&mut world, general));
    }

    #[test]
    fn a_full_tick_leaves_idle_entities_alone_and_uses_their_turn() {
        let mut world = started();
        let general = spawn(&mut world, EntityRunKind::General);
        let event = spawn(&mut world, EntityRunKind::TempEvent);

        phase_run_entity_thinks(&mut world.frame(), Tick(1));

        assert!(!can_still_think(&mut world, general));
        assert!(!can_still_think(&mut world, event));
    }

    #[test]
    fn a_new_frame_lets_every_entity_think_again() {
        let mut world = started();
        let general = spawn(&mut world, EntityRunKind::General);
        phase_walk_entity_thinks(&mut world.frame());

        world.frame().entity_kernel_mut().begin_frame(100);

        assert!(can_still_think(&mut world, general));
    }

    #[test]
    fn walking_twice_in_one_frame_is_the_same_as_once() {
        let mut world = started();
        let general = spawn(&mut world, EntityRunKind::General);

        phase_run_entity_thinks(&mut world.frame(), Tick(1));
        phase_run_entity_thinks(&mut world.frame(), Tick(1));

        assert!(!can_still_think(&mut world, general));
    }

    #[test]
    fn an_empty_world_walks_without_incident() {
        let mut world = started();
        phase_run_entity_thinks(&mut world.frame(), Tick(3));
        phase_walk_entity_thinks(&mut world.frame());
    }

    #[test]
    #[should_panic(expected = "ET_PLAYER_CORPSE")]
    fn a_body_entity_missing_from_the_corpse_table_is_refused() {
        let mut world = started();
        spawn(&mut world, EntityRunKind::PlayerCorpse);

        phase_run_entity_thinks(&mut world.frame(), Tick(1));
    }

    #[test]
    fn a_body_entity_with_a_matching_slot_thinks_quietly() {
        let mut world = started();
        let body = spawn(&mut world, EntityRunKind::PlayerCorpse);
        world.frame().corpses_mut().place(2, body.number(), PlayerCorpseSlot::default());

        phase_run_entity_thinks(&mut world.frame(), Tick(1));

        assert!(!can_still_think(&mut world, body));
    }

    #[test]
    fn bodies_in_the_air_fall_during_a_full_tick_only() {
        let mut world = started();
        let body = spawn(&mut world, EntityRunKind::PlayerCorpse);
        let mut falling = PlayerCorpseSlot { falling: true, ..PlayerCorpseSlot::default() };
        falling.tr_type = entity_t6::tr_type::GRAVITY;
        falling.tr_base = [0.0, 0.0, 500.0];
        world.frame().corpses_mut().place(0, body.number(), falling);

        phase_walk_entity_thinks(&mut world.frame());
        assert_eq!(world.frame().corpses().slots[0].origin, [0.0; 3]);

        world.frame().entity_kernel_mut().begin_frame(100);
        phase_run_entity_thinks(&mut world.frame(), Tick(2));
        assert!(world.frame().corpses().slots[0].origin != [0.0; 3]);
    }
}
