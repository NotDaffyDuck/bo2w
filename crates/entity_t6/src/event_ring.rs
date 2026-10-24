//! BO2's four-slot event rings: how events are added (`BG_AddEvent`),
//! copied from a player to its entity (`BG_PlayerToEntityProcessEvents`),
//! and played back on the client (`CG_CheckEntEvents`,
//! `CG_CheckPlayerstateEvents`).
//!
//! A ring is four event slots, four parameter slots and a sequence
//! number. Adding writes slot `sequence & 3` and steps the sequence,
//! which wraps at 256.

use playerstate_t6::PlayerState;

use crate::entity_state::EntityState;
use crate::event;

/// Slots in every event ring.
pub const EVENT_RING_LEN: usize = 4;
/// The sequence wraps at this (`(sequence + 1) & 0xff`).
pub const EVENT_SEQUENCE_WRAP: i16 = 0x100;
/// A previous sequence more than this far ahead of the new one has
/// wrapped, and is pulled back by [`EVENT_SEQUENCE_WRAP`].
pub const EVENT_SEQUENCE_WRAP_WINDOW: i16 = 0x40;
/// The largest event parameter BO2 allows: 8 weapon bits plus three
/// 6-bit attachments.
pub const EVENT_PARM_MAX: u32 = (1 << (8 + 6 * 3)) - 1;

/// Events the server keeps to itself: never copied to the player's
/// entity (`serverOnlyEvents`).
pub const SERVER_ONLY_EVENTS: [u8; 4] = [
    event::FIRE_MELEE,
    event::RELOAD_ADDAMMO,
    event::RELOAD_START_NOTIFY,
    event::WEAPON_SWITCH_STARTED,
];

/// Events only the player who caused them plays: never copied to the
/// player's entity for others to see (`singleClientEvents`).
pub const SINGLE_CLIENT_EVENTS: [u8; 11] = [
    event::STANCE_FORCE_STAND,
    event::STANCE_FORCE_CROUCH,
    event::STANCE_FORCE_PRONE,
    event::STANCE_INVALID,
    event::SWITCH_OFFHAND,
    event::EMPTY_OFFHAND,
    event::RESET_ADS,
    event::SWITCH_OFFHAND,
    event::MELEE_BLOOD,
    event::BULLET_HIT_CLIENT_SMALL,
    event::BULLET_HIT_CLIENT_LARGE,
];

/// An event BO2 asserts on: number 256 or more, or a parameter over
/// [`EVENT_PARM_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadEvent {
    pub event: i32,
    pub parm: u32,
}

/// `BG_AddEvent`: put `ev` and `parm` in slot `sequence & 3` and step
/// the sequence. Event 0 adds nothing.
pub fn add_event<E: Copy + TryFrom<i32>>(
    sequence: &mut i16,
    events: &mut [E; EVENT_RING_LEN],
    parms: &mut [u32; EVENT_RING_LEN],
    ev: i32,
    parm: u32,
) -> Result<(), BadEvent> {
    if ev == 0 {
        return Ok(());
    }
    let bad = BadEvent { event: ev, parm };
    if !(0..0x100).contains(&ev) || parm > EVENT_PARM_MAX {
        return Err(bad);
    }
    let slot = (*sequence & 3) as usize;
    events[slot] = E::try_from(ev).map_err(|_| bad)?;
    parms[slot] = parm;
    *sequence = (*sequence + 1) & 0xff;
    Ok(())
}

/// The previous sequence, pulled back by 256 when the new one has
/// wrapped past it.
#[must_use]
pub const fn unwrap_previous(sequence: i16, previous: i16) -> i16 {
    if (sequence as i32) + (EVENT_SEQUENCE_WRAP_WINDOW as i32) < previous as i32 {
        previous - EVENT_SEQUENCE_WRAP
    } else {
        previous
    }
}

fn is_listed(list: &[u8], ev: i32) -> bool {
    list.iter().any(|&e| e as i32 == ev)
}

/// Room for every event one playback can return: four predictable,
/// plus an unpredictable gap of up to the wrap window.
pub const EVENT_LIST_CAP: usize = EVENT_RING_LEN + EVENT_SEQUENCE_WRAP_WINDOW as usize;

/// Events with their parameters, in play order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventList {
    items: [(i32, u32); EVENT_LIST_CAP],
    len: usize,
}

impl Default for EventList {
    fn default() -> Self {
        Self { items: [(0, 0); EVENT_LIST_CAP], len: 0 }
    }
}

impl EventList {
    fn push(&mut self, ev: i32, parm: u32) {
        if self.len < self.items.len() {
            self.items[self.len] = (ev, parm);
            self.len += 1;
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[(i32, u32)] {
        &self.items[..self.len]
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl EntityState {
    /// `BG_AddEntityStateEvent`.
    pub fn add_event(&mut self, ev: u8, parm: u32) -> Result<(), BadEvent> {
        add_event(
            &mut self.event_sequence,
            &mut self.events,
            &mut self.event_parms,
            ev as i32,
            parm,
        )
    }

    /// `CG_CheckEntEvents`: the events to play since `previous`, at most
    /// the last four. Afterwards the caller's previous sequence becomes
    /// `self.event_sequence`. A sequence of 0 means none sent yet.
    #[must_use]
    pub fn new_events(&self, previous: i16) -> EventList {
        let sequence = self.event_sequence;
        let mut out = EventList::default();
        let mut from = if sequence == 0 {
            0
        } else {
            unwrap_previous(sequence, previous)
        };
        if sequence - from > EVENT_RING_LEN as i16 {
            from = sequence - EVENT_RING_LEN as i16;
        }
        let mut i = from;
        while i < sequence {
            let slot = (i & 3) as usize;
            out.push(self.events[slot] as i32, self.event_parms[slot]);
            i += 1;
        }
        out
    }
}

/// One half of `BG_PlayerToEntityProcessEvents`: copy a player ring's
/// new events to the entity, leaving out the server-only and
/// single-client ones, and mark them copied.
fn copy_ring_to_entity(
    es: &mut EntityState,
    sequence: i16,
    sequence_old: &mut i16,
    events: &[i32; EVENT_RING_LEN],
    parms: &[u32; EVENT_RING_LEN],
) {
    let mut i = unwrap_previous(sequence, *sequence_old);
    while i < sequence {
        let slot = (i & 3) as usize;
        let ev = events[slot];
        if !is_listed(&SERVER_ONLY_EVENTS, ev) && !is_listed(&SINGLE_CLIENT_EVENTS, ev) {
            // BO2 asserts on a bad event here; it never reaches a ring.
            let _ = add_event(
                &mut es.event_sequence,
                &mut es.events,
                &mut es.event_parms,
                ev,
                parms[slot],
            );
        }
        i += 1;
    }
    *sequence_old = sequence;
}

/// `BG_PlayerToEntityProcessEvents`: copy the player's new predictable,
/// then unpredictable, events to its entity, then clear `event_parm`.
pub fn player_to_entity_events(ps: &mut PlayerState, es: &mut EntityState) {
    copy_ring_to_entity(
        es,
        ps.predictable_event_sequence,
        &mut ps.predictable_event_sequence_old,
        &ps.predictable_events,
        &ps.predictable_event_parms,
    );
    copy_ring_to_entity(
        es,
        ps.unpredictable_event_sequence,
        &mut ps.unpredictable_event_sequence_old,
        &ps.unpredictable_events,
        &ps.unpredictable_event_parms,
    );
    es.event_parm = 0;
}

/// `CG_CheckPlayerstateEvents`: the events the local player plays going
/// from `old` to `new`.
///
/// Predictable events: each of the last four slots past `old`'s
/// sequence plays; a slot `old` already played plays again only when the
/// server changed it. Unpredictable events: each one past `old`'s
/// sequence.
#[must_use]
pub fn player_state_events(old: &PlayerState, new: &PlayerState) -> EventList {
    let mut out = EventList::default();
    let seq = new.predictable_event_sequence;
    let old_seq = unwrap_previous(seq, old.predictable_event_sequence);
    let mut i = seq - EVENT_RING_LEN as i16;
    while i < seq {
        let slot = (i & 3) as usize;
        let ev = new.predictable_events[slot];
        if i >= old_seq
            || (i > old_seq - EVENT_RING_LEN as i16 && ev != old.predictable_events[slot])
        {
            out.push(ev, new.predictable_event_parms[slot]);
        }
        i += 1;
    }
    let useq = new.unpredictable_event_sequence;
    let mut i = unwrap_previous(useq, old.unpredictable_event_sequence);
    while i < useq {
        let slot = (i & 3) as usize;
        out.push(new.unpredictable_events[slot], new.unpredictable_event_parms[slot]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_bo2s() {
        assert_eq!(SERVER_ONLY_EVENTS, [40, 23, 22, 29]);
        assert_eq!(SINGLE_CLIENT_EVENTS, [8, 9, 10, 234, 49, 16, 17, 49, 55, 67, 68]);
    }

    #[test]
    fn adding_fills_slots_and_wraps_at_256() {
        let mut es = EntityState { event_sequence: 254, ..Default::default() };
        es.add_event(event::JUMP, 7).unwrap();
        es.add_event(event::LANDING_FIRST, 0).unwrap();
        assert_eq!(es.event_sequence, 0);
        assert_eq!(es.events[2], event::JUMP);
        assert_eq!(es.event_parms[2], 7);
        assert_eq!(es.events[3], event::LANDING_FIRST);
        es.add_event(event::NONE, 0).unwrap();
        assert_eq!(es.event_sequence, 0);
        assert!(es.add_event(event::JUMP, EVENT_PARM_MAX + 1).is_err());
    }

    #[test]
    fn playback_keeps_the_last_four_and_survives_the_wrap() {
        let mut es = EntityState { event_sequence: 250, ..Default::default() };
        for k in 0..10u32 {
            es.add_event(event::JUMP, k).unwrap();
        }
        assert_eq!(es.event_sequence, 4);
        let got = es.new_events(250);
        assert_eq!(got.as_slice(), &[(event::JUMP as i32, 6), (event::JUMP as i32, 7), (event::JUMP as i32, 8), (event::JUMP as i32, 9)]);
        assert_eq!(es.new_events(3).as_slice(), &[(event::JUMP as i32, 9)]);
        assert!(es.new_events(4).is_empty());
    }

    #[test]
    fn copy_to_entity_leaves_out_private_events() {
        let mut ps = PlayerState::default();
        for ev in [event::JUMP, event::RESET_ADS, event::FIRE_MELEE, event::LANDING_FIRST] {
            add_event(
                &mut ps.predictable_event_sequence,
                &mut ps.predictable_events,
                &mut ps.predictable_event_parms,
                ev as i32,
                3,
            )
            .unwrap();
        }
        let mut es = EntityState { event_parm: 99, ..Default::default() };
        player_to_entity_events(&mut ps, &mut es);
        assert_eq!(es.event_sequence, 2);
        assert_eq!(&es.events[..2], &[event::JUMP, event::LANDING_FIRST]);
        assert_eq!(ps.predictable_event_sequence_old, 4);
        assert_eq!(es.event_parm, 0);
        player_to_entity_events(&mut ps, &mut es);
        assert_eq!(es.event_sequence, 2);
    }

    #[test]
    fn local_player_replays_changed_predictions_only() {
        let mut old = PlayerState::default();
        for _ in 0..2 {
            add_event(
                &mut old.predictable_event_sequence,
                &mut old.predictable_events,
                &mut old.predictable_event_parms,
                event::JUMP as i32,
                0,
            )
            .unwrap();
        }
        let mut new = old.clone();
        add_event(
            &mut new.predictable_event_sequence,
            &mut new.predictable_events,
            &mut new.predictable_event_parms,
            event::LANDING_FIRST as i32,
            5,
        )
        .unwrap();
        assert_eq!(player_state_events(&old, &new).as_slice(), &[(event::LANDING_FIRST as i32, 5)]);
        new.predictable_events[1] = event::MANTLE as i32;
        assert_eq!(
            player_state_events(&old, &new).as_slice(),
            &[(event::MANTLE as i32, 0), (event::LANDING_FIRST as i32, 5)]
        );
        add_event(
            &mut new.unpredictable_event_sequence,
            &mut new.unpredictable_events,
            &mut new.unpredictable_event_parms,
            event::OBITUARY as i32,
            1,
        )
        .unwrap();
        assert_eq!(player_state_events(&old, &new).len(), 3);
    }
}
