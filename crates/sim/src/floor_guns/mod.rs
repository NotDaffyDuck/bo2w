//! Guns lying on the ground: letting one go, standing on one, and taking one with the use button.
//!
//! `launch` puts a gun into the world and `grab` takes one out of it into a player's hands. `reclaim` hands a thrown
//! knife or blade back, and `usekey` is the use button's aim pick and hold timing. How a gun falls and lands, and what
//! a touch is allowed to take, belong to `crate::floor_gun_facts`.

use entity_t6::{EventCode, EntitySnapshot};
use playerstate_t6::game::{MOVE_KIND_DEAD, AvatarState};
use playerstate_t6::perks::PERK_SCAVENGER;

use crate::PlayerSessionStage;
use crate::step_view::StepWorldView;
use crate::sim_parts::{SeatNo, StepNo};

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
pub struct LootOnGround {
    pub state: EntitySnapshot,
    pub origin: [f32; 3],
    pub dropping: bool,
    pub right_mag: i32,
    pub left_mag: i32,
    pub spare: i32,
    /// The second slot's reserve: an alt mode's or the left hand's pool.
    pub stock2: i32,
    pub scavenge: bool,
    /// The dropper cannot touch or use it before this level time (a second after it was launched); 0 when it has
    /// no such hold.
    pub owner_clear_ms: i32,
}

/// The top of a player's collision box, which follows their stance: 70 standing, 50 crouched or in last stand, 30
/// prone.
pub(crate) fn stance_maxs_z(ps: &AvatarState) -> f32 {
    use movement_t6::bridge::{SQUAT_HULL_TOP, BELLY_HULL_TOP, UPRIGHT_HULL_TOP, PostureFooting, posture_footing_kind};
    let surface = posture_footing_kind(ps);
    match surface {
        PostureFooting::Stand => UPRIGHT_HULL_TOP,
        PostureFooting::Crouch | PostureFooting::DownedGrace => SQUAT_HULL_TOP,
        PostureFooting::Prone => BELLY_HULL_TOP,
    }
}

/// Whether the two boxes, each given as an origin plus its corners, touch on every axis (sharing a face counts).
pub(crate) fn aabb_overlap(
    first_origin: [f32; 3],
    first_mins: [f32; 3],
    first_maxs: [f32; 3],
    second_origin: [f32; 3],
    second_mins: [f32; 3],
    second_maxs: [f32; 3],
) -> bool {
    let apart_on = |axis: usize| {
        first_origin[axis] + first_mins[axis] > second_origin[axis] + second_maxs[axis]
            || first_origin[axis] + first_maxs[axis] < second_origin[axis] + second_mins[axis]
    };
    !(0..3).any(apart_on)
}

/// Only a living player in play can touch, or use, anything on the ground.
pub(crate) fn walker_can_touch(world: &StepWorldView, id: SeatNo, ps: &AvatarState) -> bool {
    let alive = world.client_meta(id).is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive);
    alive && ps.health >= 1 && ps.movement_mode < MOVE_KIND_DEAD
}

pub(crate) fn has_scavenger_perk(ps: &AvatarState) -> bool {
    ps.has_perk_bit(PERK_SCAVENGER)
}

/// Runs one dropped gun's per-tick move, unless it is carried by something.
pub(crate) fn think_item_move(world: &mut StepWorldView, clock_stamp_ms: i32, item_num: i32) {
    let kernel = world.entity_kernel();
    let carried = kernel
        .ref_for(item_num)
        .is_ok_and(|entity| kernel.look_up(entity).is_ok_and(|view| view.ties.parent.is_some()));
    if !carried {
        crate::floor_gun_facts::run_item(world, clock_stamp_ms, item_num);
    }
}

/// Every player's touch against the guns and bags on the ground. Without BO2's weapon facts there is nothing for a
/// touch to read, and nothing lying about either.
pub(crate) fn phase_touch_items(world: &mut StepWorldView) {
    if !world.gun_facts().is_empty() {
        crate::floor_gun_facts::phase_touch(world);
    }
}

/// Writes `next` back as the player's state.
fn store_player(world: &mut StepWorldView, id: SeatNo, next: AvatarState) {
    if let Some(held) = world.occupant_mut(id) {
        *held = next;
    }
}

/// Tells everyone that `taker` took `weapon` (or ammo for it) at `origin`.
fn announce_pickup(
    world: &mut StepWorldView,
    tick: StepNo,
    kind: EventCode,
    taker: SeatNo,
    gun: u32,
    spot: [f32; 3],
) {
    let audience = crate::EventRecipients::All;
    let (who, parm) = (taker.0 as i32, gun as i32);
    let cargo = crate::HappeningBody { number: who, event_parm: parm, weapon: gun, origin: spot, ..Default::default() };
    world.post_entity_event(tick, audience, kind, cargo);
}
