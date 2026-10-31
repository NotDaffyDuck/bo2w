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

use crate::step_view::StepWorldView;
use crate::sim_parts::SeatNo;
use crate::{SIM_STEP_MILLIS, StepNo};
use entity_t6::Trajectory;
use entity_t6::path_kind::FALLING;
use playerstate_t6::game::{
    BodyAnims, NO_ENTITY_SLOT, LINK_FLAGS_FORCE_THIRD_PERSON, CORPSE_ENTITY_COUNT,
    BODY_ENTITY_FIRST_SLOT, AvatarState, entity_flag_bits,
};

pub(crate) use fall::settle_corpses;

/// Index of the yaw component in an angle triple.
const YAW: usize = 1;

/// How many bodies the pool holds.
const BODIES: usize = CORPSE_ENTITY_COUNT as usize;

/// A body never leaves the victim faster than this sideways (units per second); the fall and any
/// upward speed carry over untouched.
const TOSS_SPEED_CAP: f32 = 80.0;

/// Entity-flag bit that flips each time an entity number is handed out again (it is the same
/// bit the teleport flag uses, 0x2). The client starts a body's animation and ragdoll over when
/// it sees the bit change.
pub const CLONE_NEW_BODY_FLAG: u32 = entity_flag_bits::TELEPORT;

/// One body in the pool, as the network layer and the entity think see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CorpseBay {
    /// Holds a body right now.
    pub occupied: bool,
    /// Entity number the body wears, or -1 while the slot is free.
    pub entnum: i32,
    /// Whose body it was.
    pub victim: SeatNo,
    pub origin: [f32; 3],
    pub viewangles: [f32; 3],
    /// Leg and torso animation the victim died in.
    pub anim: BodyAnims,
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
    /// Entity the body came to rest on, `NO_ENTITY_SLOT` for none.
    pub ground_entity_num: i32,
}

/// Older spelling PlayerCorpseSlot still calls; the rename sweep moves it over.
pub use self::CorpseBay as PlayerCorpseSlot;

impl Default for CorpseBay {
    fn default() -> Self {
        Self {
            entnum: -1,
            ground_entity_num: NO_ENTITY_SLOT,
            occupied: false,
            falling: Default::default(),
            victim: Default::default(),
            anim: Default::default(),
            origin: Default::default(),
            viewangles: Default::default(),
            tr_delta: Default::default(),
            tr_base: Default::default(),
            tr_type: 0,
            tr_time: 0,
            tr_duration: 0,
            weapon: 0,
            e_flags: 0,
            view_height_current: f32::default(),
        }
    }
}

impl CorpseBay {
    /// The body's motion as an entity trajectory.
    pub fn trajectory(&self) -> Trajectory {
        let Self { tr_time, tr_type, tr_duration, tr_delta, tr_base, .. } = *self;
        Trajectory { tr_type, began_at: tr_time, span_ms: tr_duration, rate: tr_delta, base_point: tr_base }
    }

    /// The body a dying player leaves: their pose and animation, set tossing under gravity from the
    /// moment of death.
    fn left_by(
        victim: SeatNo,
        ps: &AvatarState,
        died_at_ms: i32,
        new_body_bit: u32,
    ) -> Self {
        let death_pos = ps.origin;
        let eye_height = ps.eye_height_now;
        let airborne = true;
        let pose = ps.anim();
        let gun_in_hand = ps.weapon;
        Self {
            victim,
            origin: death_pos,
            viewangles: [0.0, facing_yaw(ps), 0.0],
            anim: pose,
            view_height_current: eye_height,
            weapon: gun_in_hand,
            // The player's own teleport bit means nothing for a body. The new-body bit replaces it,
            // so a reused entity number never repeats the previous body's flags.
            e_flags: (ps.e_flags & !CLONE_NEW_BODY_FLAG) | new_body_bit,
            tr_type: FALLING,
            tr_time: died_at_ms,
            tr_duration: 0,
            tr_delta: capped_toss(ps.velocity),
            tr_base: death_pos,
            falling: airborne,
            ..Default::default()
        }
    }
}

/// The eight body slots plus the counter that hands out entity numbers. Starts with every slot free and the
/// counter at zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CorpseYard {
    pub slots: [CorpseBay; BODIES],
    /// Position in the entity-number ring (see [`Self::next_clone_entity`]).
    pub spawn_ring: u8,
}
/// Older spelling PlayerCorpsePool still calls; the rename sweep moves it over.
pub use self::CorpseYard as PlayerCorpsePool;

impl CorpseYard {
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
        let number = BODY_ENTITY_FIRST_SLOT + i32::from(step % lap_len);
        let new_body_bit = if step >= lap_len { CLONE_NEW_BODY_FLAG } else { 0 };
        self.spawn_ring = (step + 1) % (lap_len * 2);
        self.release_entity_slot(number);
        (number, new_body_bit)
    }

    /// Frees every slot wearing `entnum`.
    pub fn release_entity_slot(&mut self, slot_no: i32) {
        for slot in self.slots.iter_mut().filter(|slot| slot.entnum == slot_no) {
            *slot = Default::default();
        }
    }

    /// The slot a new body at `near` should go in: the first free one, otherwise the body
    /// farthest away, which is evicted. (Equal distances go to the lowest slot.)
    pub(crate) fn slot_for_new_body(&mut self, near: [f32; 3]) -> u8 {
        if let Some(free) = self.slots.iter().position(|slot| slot.entnum == -1) {
            return free as u8;
        }
        let (victim_index, _) = self.slots.iter().enumerate().fold((0, -1.0f32), |best, (index, body)| {
            let offset = [near[0] - body.origin[0], near[1] - body.origin[1], near[2] - body.origin[2]];
            let sq = offset.iter().map(|d| d * d).sum::<f32>();
            if sq > best.1 { (index, sq) } else { best }
        });
        let evicted = self.slots[victim_index].entnum;
        self.release_entity_slot(evicted);
        victim_index as u8
    }

    /// The entity number in each slot, -1 for a free one.
    pub fn slot_numbers(&self) -> [i32; BODIES] {
        self.slots.map(|slot| slot.entnum)
    }

    /// Puts `body` in `slot` (taken modulo the pool size) wearing `entnum`; returns the slot used.
    pub(crate) fn place(&mut self, slot: u8, slot_no: i32, body: CorpseBay) -> u8 {
        let index = usize::from(slot) % BODIES;
        self.slots[index] = CorpseBay { occupied: true, entnum: slot_no, ..body };
        index as u8
    }
}

/// Level time in milliseconds at the start of `tick`.
pub fn level_time_ms(tick: StepNo) -> i32 {
    i32::try_from(tick.0).unwrap_or(0).saturating_mul(SIM_STEP_MILLIS as i32)
}

/// Leaves `victim`'s body in the pool, in whichever slot is free or farthest, under the next
/// entity number. Returns the slot used.
pub(crate) fn fill_clone_slot(
    world: &mut StepWorldView,
    victim: SeatNo,
    ps: &AvatarState,
    clock_stamp_ms: i32,
) -> u8 {
    let ring = world.corpses_mut();
    let (entnum, new_body_bit) = ring.next_clone_entity();
    let slot = ring.slot_for_new_body(ps.origin);
    occupy_player_clone_at(world, victim, ps, clock_stamp_ms, slot, entnum, new_body_bit)
}

/// As [`fill_clone_slot`], for a caller that chooses the slot, entity number and new-body bit
/// itself (BO2's four shared bodies, in the t6 player animation code).
pub(crate) fn occupy_player_clone_at(
    world: &mut StepWorldView,
    victim: SeatNo,
    ps: &AvatarState,
    clock_stamp_ms: i32,
    slot: u8,
    entity_number: i32,
    new_body_bit: u32,
) -> u8 {
    world.corpses_mut().place(slot, entity_number, CorpseBay::left_by(victim, ps, clock_stamp_ms, new_body_bit))
}

/// The way the victim faced: the weapon angle when a script holds the camera in third person,
/// otherwise the view.
fn facing_yaw(ps: &AvatarState) -> f32 {
    let angles = if ps.attach_bits & LINK_FLAGS_FORCE_THIRD_PERSON != 0 {
        &ps.link_weapon_angles
    } else {
        &ps.look_angles
    };
    angles[YAW]
}

/// `velocity` with its sideways speed held to [`TOSS_SPEED_CAP`] on the high side only.
fn capped_toss(velocity: [f32; 3]) -> [f32; 3] {
    let cap = |v: f32| if v > TOSS_SPEED_CAP { TOSS_SPEED_CAP } else { v };
    [cap(velocity[0]), cap(velocity[1]), velocity[2]]
}
