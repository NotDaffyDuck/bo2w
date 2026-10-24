//! Guns lying on the ground: letting one go, standing on one, and taking one with the use button.
//!
//! `launch` puts a gun into the world and `grab` takes one out of it into a player's hands. `reclaim` hands a thrown
//! knife or blade back, and `usekey` is the use button's aim pick and hold timing. How a gun falls and lands, and what
//! a touch is allowed to take, belong to `crate::item_t6`.

use entity_t6::{EntityEventKind, EntityState};
use playerstate_t6::game::{PERK_SCAVENGER, PM_TYPE_DEAD, PlayerState};

use crate::ClientLifecycle;
use crate::frame::FrameWorld;
use crate::world::{ClientId, Tick};

mod grab;
mod inventory;
mod launch;
mod reclaim;
#[cfg(test)]
mod tests;
mod usekey;

pub(crate) use grab::{grab_number, grab_scavenger};
pub(crate) use launch::{drop_scavenger_item, drop_weapon, launch_dropped_from_ps, push_dropped_item, random_unit};
pub(crate) use usekey::phase_use_items;

/// A gun (or scavenger bag) lying in the world, with the rounds it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItem {
    pub state: EntityState,
    pub origin: [f32; 3],
    pub falling: bool,
    pub clip_r: i32,
    pub clip_l: i32,
    pub stock: i32,
    /// The second slot's reserve: an alt mode's or the left hand's pool.
    pub stock2: i32,
    pub scavenger: bool,
    /// The dropper cannot touch or use it before this level time (a second after it was launched); 0 when it has
    /// no such hold.
    pub owner_clear_ms: i32,
}

/// The top of a player's collision box, which follows their stance: 70 standing, 50 crouched or in last stand, 30
/// prone.
pub(crate) fn stance_maxs_z(ps: &PlayerState) -> f32 {
    use movement_t6::bridge::{CROUCH_MAXS_Z, PRONE_MAXS_Z, STAND_MAXS_Z, StanceSurface, stance_surface_type};
    match stance_surface_type(ps) {
        StanceSurface::Stand => STAND_MAXS_Z,
        StanceSurface::Crouch | StanceSurface::LastStand => CROUCH_MAXS_Z,
        StanceSurface::Prone => PRONE_MAXS_Z,
    }
}

/// Whether the two boxes, each given as an origin plus its corners, touch on every axis (sharing a face counts).
pub(crate) fn aabb_overlap(
    a_origin: [f32; 3],
    a_mins: [f32; 3],
    a_maxs: [f32; 3],
    b_origin: [f32; 3],
    b_mins: [f32; 3],
    b_maxs: [f32; 3],
) -> bool {
    let apart_on = |axis: usize| {
        a_origin[axis] + a_mins[axis] > b_origin[axis] + b_maxs[axis]
            || a_origin[axis] + a_maxs[axis] < b_origin[axis] + b_mins[axis]
    };
    !(0..3).any(apart_on)
}

/// Only a living player in play can touch, or use, anything on the ground.
pub(crate) fn walker_can_touch(world: &FrameWorld, id: ClientId, ps: &PlayerState) -> bool {
    let alive = world.client_meta(id).is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
    alive && ps.health >= 1 && ps.pm_type < PM_TYPE_DEAD
}

pub(crate) fn has_scavenger_perk(ps: &PlayerState) -> bool {
    ps.perks[0] & PERK_SCAVENGER != 0
}

/// Runs one dropped gun's per-tick move, unless it is carried by something.
pub(crate) fn think_item_move(world: &mut FrameWorld, time_ms: i32, number: i32) {
    let kernel = world.entity_kernel();
    let carried = kernel
        .current_ref(number)
        .is_ok_and(|entity| kernel.resolve(entity).is_ok_and(|view| view.relations.parent.is_some()));
    if !carried {
        crate::item_t6::run_item(world, time_ms, number);
    }
}

/// Every player's touch against the guns and bags on the ground. Without BO2's weapon facts there is nothing for a
/// touch to read, and nothing lying about either.
pub(crate) fn phase_touch_items(world: &mut FrameWorld) {
    if !world.gun_facts().is_empty() {
        crate::item_t6::phase_touch(world);
    }
}

/// Writes `next` back as the player's state.
fn store_player(world: &mut FrameWorld, id: ClientId, next: PlayerState) {
    if let Some(slot) = world.player_mut(id) {
        *slot = next;
    }
}

/// Tells everyone that `walker` took `weapon` (or ammo for it) at `origin`.
fn announce_pickup(
    world: &mut FrameWorld,
    tick: Tick,
    kind: EntityEventKind,
    walker: ClientId,
    weapon: u32,
    origin: [f32; 3],
) {
    world.push_entity_event(
        tick,
        crate::EventAudience::All,
        kind,
        crate::EntityEventPayload {
            number: walker.0 as i32,
            event_parm: weapon as i32,
            weapon,
            origin,
            ..Default::default()
        },
    );
}
