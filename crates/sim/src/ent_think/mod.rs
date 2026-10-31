//! Once per tick every live entity gets a turn to think, and a parent gets its turn before the entities that ride
//! on it.
//!
//! This module settles who goes when, and keeps anything from thinking twice in one frame. What a think does is
//! decided per entity kind in [`take_turn`]; how a rider follows its parent is in [`follow`].

mod follow;

use crate::bodies::level_time_ms;
use crate::bodies::settle_corpses;
use crate::ent_links::entnum_matched;
use crate::step_view::StepWorldView;
use crate::ent_slots::{EntityBehaviorClass, ThinkEntryResult};
use crate::ent_slots::{EntHandle, PROTECTED_ENTITY_SLOTS};
use crate::sim_parts::StepNo;

/// What a walk over the entities is for.
#[derive(Clone, Copy)]
enum Pass {
    /// A real tick, in which each kind runs its own think.
    Think { tick: StepNo, clock_stamp_ms: i32 },
    /// The entities only spend their turn for this frame, so that none can be given one again before the frame ends.
    SpendOnly,
}

/// One tick of thinking: falling bodies move first, then each entity takes its turn.
pub(crate) fn run_due_thinks(world: &mut StepWorldView, step: StepNo) {
    let clock_millis = level_time_ms(step);
    settle_corpses(world, clock_millis);
    walk(world, Pass::Think { tick: step, clock_stamp_ms: clock_millis });
}

/// Spends the turn of every entity for this frame; nothing runs.
pub(crate) fn walk_thinks(world: &mut StepWorldView) {
    walk(world, Pass::SpendOnly);
}

fn walk(world: &mut StepWorldView, pass: Pass) {
    // The end is read again after every turn, because a think can create more entities and they take their turn in
    // this very walk.
    for slot in PROTECTED_ENTITY_SLOTS.. {
        if slot >= world.entity_kernel().high_water() {
            break;
        }
        take_turn(world, slot, pass);
    }
}

fn take_turn(view: &mut StepWorldView, slot: i32, pass: Pass) {
    let (me, above) = match view.entity_kernel_mut().open_think(slot) {
        Ok(ThinkEntryResult::Run { entity: me, parent: above, .. }) => (me, above),
        Err(_) | Ok(ThinkEntryResult::Skip) => return,
    };
    // Whatever rides on a parent needs the parent's pose from this frame, so the parent goes first.
    if let Some(above) = above {
        take_turn(view, above.slot(), pass);
    }
    let Pass::Think { tick, clock_stamp_ms } = pass else {
        return;
    };
    let Ok(child) = view.entity_kernel().look_up(me) else {
        return;
    };
    let rides = child.ties.parent.is_some();
    let entnum = me.slot();

    match child.class {
        EntityBehaviorClass::Missile => crate::gear::think_projectile(view, tick, entnum),
        EntityBehaviorClass::Item if !rides => {
            follow::follow_parent(view, clock_stamp_ms, me);
            crate::floor_guns::think_item_move(view, clock_stamp_ms, entnum);
        }
        EntityBehaviorClass::Item | EntityBehaviorClass::General => {
            follow::follow_parent(view, clock_stamp_ms, me);
            settle_think(view, me);
        }
        EntityBehaviorClass::MainLightSource | EntityBehaviorClass::ScriptedMover if rides => {
            follow::follow_parent(view, clock_stamp_ms, me);
            settle_think(view, me);
        }
        EntityBehaviorClass::MainLightSource | EntityBehaviorClass::ScriptedMover => follow::pose_mover_models(view, tick, entnum),
        EntityBehaviorClass::TempEvent => (),
        EntityBehaviorClass::FallenPlayerBody => settle_corpse(view, me, entnum),
    }
}

/// A body entity must have a row in the corpse table before it may idle.
fn settle_corpse(world: &mut StepWorldView, body: EntHandle, slot_num: i32) {
    let table_entnums = world.body_pool().slot_numbers();
    assert!(
        entnum_matched(&table_entnums, slot_num),
        "ET_PLAYER_CORPSE occupancy number not in the corpse table; a miss falls back to slot 0"
    );
    settle_think(world, body);
}

/// An entity with nothing scheduled has nothing to do. If its scheduled time has come it would need a handler, which
/// these plain kinds do not have.
fn settle_think(world: &mut StepWorldView, idle: EntHandle) {
    let kernel = world.entity_kernel_mut();
    let handler_due = kernel.pop_due_think(idle).expect("scheduled think occupancy vanished");
    assert!(!handler_due, "generic think has no handler");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WholeSim;
    use crate::bodies::CorpseBay;

    /// A world whose clock has started its first frame, so entities can think.
    fn started() -> WholeSim {
        let mut world = WholeSim::new();
        world.frame().entity_kernel_mut().open_tick(50);
        world
    }

    fn spawn(world: &mut WholeSim, kind: EntityBehaviorClass) -> EntHandle {
        world.frame().entity_kernel_mut().claim_slot(kind).expect("room for an entity")
    }

    /// True if `entity` is still allowed to think this frame.
    fn can_still_think(world: &mut WholeSim, entity: EntHandle) -> bool {
        matches!(
            world.frame().entity_kernel_mut().open_think(entity.slot()),
            Ok(ThinkEntryResult::Run { .. })
        )
    }

    #[test]
    fn a_spend_only_walk_uses_up_every_entitys_turn() {
        let mut world = started();
        let general = spawn(&mut world, EntityBehaviorClass::General);
        let event = spawn(&mut world, EntityBehaviorClass::TempEvent);

        walk_thinks(&mut world.frame());

        assert!(!can_still_think(&mut world, general));
        assert!(!can_still_think(&mut world, event));
    }

    #[test]
    fn untouched_entities_can_think_before_the_walk() {
        let mut world = started();
        let general = spawn(&mut world, EntityBehaviorClass::General);

        assert!(can_still_think(&mut world, general));
    }

    #[test]
    fn a_full_tick_leaves_idle_entities_alone_and_uses_their_turn() {
        let mut world = started();
        let general = spawn(&mut world, EntityBehaviorClass::General);
        let event = spawn(&mut world, EntityBehaviorClass::TempEvent);

        run_due_thinks(&mut world.frame(), StepNo(1));

        assert!(!can_still_think(&mut world, general));
        assert!(!can_still_think(&mut world, event));
    }

    #[test]
    fn a_new_frame_lets_every_entity_think_again() {
        let mut world = started();
        let general = spawn(&mut world, EntityBehaviorClass::General);
        walk_thinks(&mut world.frame());

        world.frame().entity_kernel_mut().open_tick(100);

        assert!(can_still_think(&mut world, general));
    }

    #[test]
    fn walking_twice_in_one_frame_is_the_same_as_once() {
        let mut world = started();
        let general = spawn(&mut world, EntityBehaviorClass::General);

        run_due_thinks(&mut world.frame(), StepNo(1));
        run_due_thinks(&mut world.frame(), StepNo(1));

        assert!(!can_still_think(&mut world, general));
    }

    #[test]
    fn an_empty_world_walks_without_incident() {
        let mut world = started();
        run_due_thinks(&mut world.frame(), StepNo(3));
        walk_thinks(&mut world.frame());
    }

    #[test]
    #[should_panic(expected = "ET_PLAYER_CORPSE")]
    fn a_body_entity_missing_from_the_corpse_table_is_refused() {
        let mut world = started();
        spawn(&mut world, EntityBehaviorClass::FallenPlayerBody);

        run_due_thinks(&mut world.frame(), StepNo(1));
    }

    #[test]
    fn a_body_entity_with_a_matching_slot_thinks_quietly() {
        let mut world = started();
        let body = spawn(&mut world, EntityBehaviorClass::FallenPlayerBody);
        world.frame().corpses_mut().place(2, body.slot(), CorpseBay::default());

        run_due_thinks(&mut world.frame(), StepNo(1));

        assert!(!can_still_think(&mut world, body));
    }

    #[test]
    fn bodies_in_the_air_fall_during_a_full_tick_only() {
        let mut world = started();
        let body = spawn(&mut world, EntityBehaviorClass::FallenPlayerBody);
        let mut airborne = CorpseBay::default();
        airborne.falling |= true;
        airborne.tr_type = entity_t6::path_kind::FALLING;
        airborne.tr_base = [0.0, 0.0, 500.0];
        world.frame().corpses_mut().place(0, body.slot(), airborne);

        walk_thinks(&mut world.frame());
        assert_eq!(world.frame().body_pool().slots[0].origin, [0.0; 3]);

        world.frame().entity_kernel_mut().open_tick(100);
        run_due_thinks(&mut world.frame(), StepNo(2));
        assert!(world.frame().body_pool().slots[0].origin != [0.0; 3]);
    }
}
