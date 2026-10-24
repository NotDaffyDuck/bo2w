//! Bodies of dead players.
//!
//! A player who dies leaves a *clone*: a stand-in entity that keeps the victim's last pose and
//! animation, tosses under gravity until it touches something, then lies where it came to rest.
//! The match keeps a small fixed pool of them (eight). When a ninth is needed, the body farthest
//! from the new death is the one that goes.
//!
//! This file owns the pool: which body sits in which slot and which entity number it wears.
//! The tumble itself is in [`fall`].

mod fall;
#[cfg(test)]
mod tests;

use crate::frame::FrameWorld;
use crate::world::ClientId;
use crate::{MATCH_TICK_MS, Tick};
use entity_t6::Trajectory;
use entity_t6::tr_type::GRAVITY;
use playerstate_t6::game::{
    AnimPair, ENTITYNUM_NONE, LINK_FLAGS_FORCE_THIRD_PERSON, MAX_CLIENT_CORPSES,
    PLAYER_CORPSE_ENTITY_BASE, PlayerState, eflags,
};

pub(crate) use fall::phase_run_corpse_move;

/// How many bodies the pool holds.
const BODIES: usize = MAX_CLIENT_CORPSES as usize;

/// A body never leaves the victim faster than this sideways (units per second); the fall and any
/// upward speed carry over untouched.
const TOSS_SPEED_CAP: f32 = 80.0;

/// Entity-flag bit that flips each time an entity number is handed out again (it is the same
/// bit the teleport flag uses, 0x2). The client starts a body's animation and ragdoll over when
/// it sees the bit change.
pub const CLONE_NEW_BODY_FLAG: u32 = eflags::TELEPORT;

/// One body in the pool, as the network layer and the entity think see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerCorpseSlot {
    /// Holds a body right now.
    pub occupied: bool,
    /// Entity number the body wears, or -1 while the slot is free.
    pub entnum: i32,
    /// Whose body it was.
    pub victim: ClientId,
    pub origin: [f32; 3],
    pub viewangles: [f32; 3],
    /// Leg and torso animation the victim died in.
    pub anim: AnimPair,
    pub view_height_current: f32,
    pub weapon: u32,
    pub e_flags: u32,

    // The trajectory the body is moving on (see `trajectory`).
    pub tr_type: u8,
    pub tr_time: i32,
    pub tr_duration: i32,
    pub tr_delta: [f32; 3],
    pub tr_base: [f32; 3],

    /// Still in the air, so the fall step moves it each tick.
    pub falling: bool,
    /// Entity the body came to rest on, `ENTITYNUM_NONE` for none.
    pub ground_entity_num: i32,
}

impl Default for PlayerCorpseSlot {
    fn default() -> Self {
        Self {
            occupied: false,
            entnum: -1,
            victim: ClientId(0),
            origin: [0.0; 3],
            viewangles: [0.0; 3],
            anim: AnimPair::default(),
            view_height_current: 0.0,
            weapon: 0,
            e_flags: 0,
            tr_type: 0,
            tr_time: 0,
            tr_duration: 0,
            tr_delta: [0.0; 3],
            tr_base: [0.0; 3],
            falling: false,
            ground_entity_num: ENTITYNUM_NONE,
        }
    }
}

impl PlayerCorpseSlot {
    /// The body's motion as an entity trajectory.
    pub fn trajectory(&self) -> Trajectory {
        Trajectory {
            tr_time: self.tr_time,
            tr_type: self.tr_type,
            tr_duration: self.tr_duration,
            tr_delta: self.tr_delta,
            tr_base: self.tr_base,
        }
    }
}

/// The eight body slots plus the counter that hands out entity numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerCorpsePool {
    pub slots: [PlayerCorpseSlot; BODIES],
    /// Position in the entity-number ring (see [`Self::next_clone_entity`]).
    pub spawn_ring: u8,
}

impl Default for PlayerCorpsePool {
    fn default() -> Self {
        Self { slots: [PlayerCorpseSlot::default(); BODIES], spawn_ring: 0 }
    }
}

impl PlayerCorpsePool {
    /// The next entity number for a new body, with the new-body bit for its flags.
    ///
    /// The counter runs twice around the eight numbers (0..16). The low half picks the entity;
    /// the lap flips the bit, so a number handed out again always carries the opposite bit from
    /// the body it replaces, and the client can tell "a new body" from "the same body again".
    /// Sixteen values still fit the single wire byte the counter is sent in.
    ///
    /// Whoever wore the number before loses their slot.
    pub(crate) fn next_clone_entity(&mut self) -> (i32, u32) {
        let lap_len = BODIES as u8;
        let step = self.spawn_ring % (lap_len * 2);
        let entnum = PLAYER_CORPSE_ENTITY_BASE + i32::from(step % lap_len);
        let new_body_bit = if step >= lap_len { CLONE_NEW_BODY_FLAG } else { 0 };
        self.spawn_ring = (step + 1) % (lap_len * 2);
        self.clear_entnum(entnum);
        (entnum, new_body_bit)
    }

    /// Frees every slot wearing `entnum`.
    pub fn clear_entnum(&mut self, entnum: i32) {
        for slot in self.slots.iter_mut().filter(|slot| slot.entnum == entnum) {
            *slot = PlayerCorpseSlot::default();
        }
    }

    /// The slot a new body at `near` should go in: the first free one, otherwise the body
    /// farthest away, which is evicted. (Equal distances go to the lowest slot.)
    pub(crate) fn slot_for_new_body(&mut self, near: [f32; 3]) -> u8 {
        if let Some(free) = self.slots.iter().position(|slot| slot.entnum == -1) {
            return free as u8;
        }
        let mut farthest = 0;
        let mut farthest_sq = -1.0f32;
        for (i, slot) in self.slots.iter().enumerate() {
            let gap = [
                near[0] - slot.origin[0],
                near[1] - slot.origin[1],
                near[2] - slot.origin[2],
            ];
            let dist_sq = gap[0] * gap[0] + gap[1] * gap[1] + gap[2] * gap[2];
            if dist_sq > farthest_sq {
                farthest_sq = dist_sq;
                farthest = i;
            }
        }
        let evicted = self.slots[farthest].entnum;
        self.clear_entnum(evicted);
        farthest as u8
    }

    /// The entity number in each slot, -1 for a free one.
    pub fn entnums(&self) -> [i32; BODIES] {
        self.slots.map(|slot| slot.entnum)
    }

    /// Puts `body` in `slot` (taken modulo the pool size) wearing `entnum`; returns the slot used.
    pub(crate) fn place(&mut self, slot: u8, entnum: i32, mut body: PlayerCorpseSlot) -> u8 {
        let index = usize::from(slot) % BODIES;
        body.occupied = true;
        body.entnum = entnum;
        self.slots[index] = body;
        index as u8
    }
}

/// Level time in milliseconds at the start of `tick`.
pub fn level_time_ms(tick: Tick) -> i32 {
    i32::try_from(tick.0).unwrap_or(0).saturating_mul(MATCH_TICK_MS as i32)
}

/// Leaves `victim`'s body in the pool, in whichever slot is free or farthest, under the next
/// entity number. Returns the slot used.
pub(crate) fn occupy_player_clone(
    world: &mut FrameWorld,
    victim: ClientId,
    ps: &PlayerState,
    time_ms: i32,
) -> u8 {
    let pool = world.corpses_mut();
    let (entnum, new_body_bit) = pool.next_clone_entity();
    let slot = pool.slot_for_new_body(ps.origin);
    occupy_player_clone_at(world, victim, ps, time_ms, slot, entnum, new_body_bit)
}

/// As [`occupy_player_clone`], for a caller that chooses the slot, entity number and new-body bit
/// itself (BO2's four shared bodies, in the t6 player animation code).
pub(crate) fn occupy_player_clone_at(
    world: &mut FrameWorld,
    victim: ClientId,
    ps: &PlayerState,
    time_ms: i32,
    slot: u8,
    entnum: i32,
    new_body_bit: u32,
) -> u8 {
    world.corpses_mut().place(slot, entnum, body_from_player(victim, ps, time_ms, new_body_bit))
}

/// The body a dying player leaves: their pose and animation, set tossing under gravity from the
/// moment of death.
fn body_from_player(
    victim: ClientId,
    ps: &PlayerState,
    time_ms: i32,
    new_body_bit: u32,
) -> PlayerCorpseSlot {
    PlayerCorpseSlot {
        victim,
        origin: ps.origin,
        viewangles: [0.0, facing_yaw(ps), 0.0],
        anim: ps.anim(),
        view_height_current: ps.view_height_current,
        weapon: ps.weapon,
        // The player's own teleport bit means nothing for a body. The new-body bit replaces it,
        // so a reused entity number never repeats the previous body's flags.
        e_flags: (ps.e_flags & !CLONE_NEW_BODY_FLAG) | new_body_bit,
        tr_type: GRAVITY,
        tr_time: time_ms,
        tr_duration: 0,
        tr_delta: capped_toss(ps.velocity),
        tr_base: ps.origin,
        falling: true,
        ..PlayerCorpseSlot::default()
    }
}

/// The way the victim faced: the weapon angle when a script holds the camera in third person,
/// otherwise the view.
fn facing_yaw(ps: &PlayerState) -> f32 {
    if ps.link_flags & LINK_FLAGS_FORCE_THIRD_PERSON != 0 {
        ps.link_weapon_angles[1]
    } else {
        ps.viewangles[1]
    }
}

/// `velocity` with its sideways speed held to [`TOSS_SPEED_CAP`] on the high side only.
fn capped_toss(velocity: [f32; 3]) -> [f32; 3] {
    let cap = |v: f32| if v > TOSS_SPEED_CAP { TOSS_SPEED_CAP } else { v };
    [cap(velocity[0]), cap(velocity[1]), velocity[2]]
}
