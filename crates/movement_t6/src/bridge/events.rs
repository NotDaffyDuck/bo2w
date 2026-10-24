//! The ring of predictable events a player state carries, and the client
//! side that plays each one exactly once.

use playerstate_t6::game::PlayerState;

/// Event sequence numbers wrap at this many values minus one (11 bits).
const SEQUENCE_MASK: i32 = 0x7ff;
/// A sequence this far behind the newest is read as having wrapped round.
const WRAP_WINDOW: i32 = 0x200;
/// How many events the ring holds before the oldest is overwritten.
const RING_LEN: i32 = 4;

/// Record `event` with `parm` in the next slot of the ring and advance the
/// sequence. Event 0 means "nothing" and is not recorded.
pub fn add_predictable_event(ps: &mut PlayerState, event: i32, parm: i32) {
    if event == 0 {
        return;
    }
    let slot = (ps.predictable_event_sequence & 3) as usize;
    ps.predictable_events[slot] = event;
    ps.predictable_event_parms[slot] = parm as u32;
    ps.predictable_event_sequence =
        ps.predictable_event_sequence.wrapping_add(1) & SEQUENCE_MASK as i16;
}

/// Record an event that carries no parameter.
pub fn add_event(ps: &mut PlayerState, event: i32) {
    add_predictable_event(ps, event, 0);
}

/// One event to play, tagged with the sequence it was recorded under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequencedPlayerEvent {
    pub sequence: i32,
    pub event: i32,
    pub event_parm: i32,
}

/// Hand every event recorded since `consumed_sequence` to `dispatch`, oldest
/// first, and move `consumed_sequence` up to the newest. Events that fell out
/// of the ring are lost; a sequence that wrapped is unwrapped first.
pub fn consume_player_events(
    ps: &PlayerState,
    consumed_sequence: &mut i32,
    mut dispatch: impl FnMut(SequencedPlayerEvent),
) {
    let newest = i32::from(ps.predictable_event_sequence);
    if *consumed_sequence == newest {
        return;
    }
    if newest == 0 {
        *consumed_sequence = 0;
        return;
    }

    let mut next = *consumed_sequence;
    if newest + WRAP_WINDOW < next {
        next -= SEQUENCE_MASK + 1;
    }
    if newest - next > RING_LEN {
        next = newest - RING_LEN;
    }
    for sequence in next..newest {
        let slot = (sequence as u32 & 3) as usize;
        let event = ps.predictable_events[slot];
        if event != 0 {
            dispatch(SequencedPlayerEvent {
                sequence: sequence & SEQUENCE_MASK,
                event,
                event_parm: ps.predictable_event_parms[slot] as i32,
            });
        }
    }
    *consumed_sequence = newest;
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn heard(ps: &PlayerState, consumed: &mut i32) -> Vec<SequencedPlayerEvent> {
        let mut out = Vec::new();
        consume_player_events(ps, consumed, |heard| out.push(heard));
        out
    }

    #[test]
    fn events_are_recorded_in_order_and_zero_is_ignored() {
        let mut ps = PlayerState::ZERO;
        add_predictable_event(&mut ps, 0, 9);
        assert_eq!(ps.predictable_event_sequence, 0);
        add_predictable_event(&mut ps, 130, 7);
        add_event(&mut ps, 136);
        assert_eq!(ps.predictable_event_sequence, 2);
        assert_eq!(ps.predictable_events[..2], [130, 136]);
        assert_eq!(ps.predictable_event_parms[..2], [7, 0]);
    }

    #[test]
    fn a_listener_hears_each_event_once() {
        let mut ps = PlayerState::ZERO;
        add_predictable_event(&mut ps, 130, 7);
        add_event(&mut ps, 136);
        let mut consumed = 0;
        let first = heard(&ps, &mut consumed);
        assert_eq!(
            first,
            [
                SequencedPlayerEvent {
                    sequence: 0,
                    event: 130,
                    event_parm: 7
                },
                SequencedPlayerEvent {
                    sequence: 1,
                    event: 136,
                    event_parm: 0
                },
            ]
        );
        assert_eq!(consumed, 2);
        assert!(heard(&ps, &mut consumed).is_empty());
    }

    #[test]
    fn a_listener_that_fell_behind_only_gets_what_the_ring_still_holds() {
        let mut ps = PlayerState::ZERO;
        for event in 1..=6 {
            add_event(&mut ps, event);
        }
        let mut consumed = 0;
        let sequences: Vec<i32> = heard(&ps, &mut consumed).iter().map(|e| e.event).collect();
        assert_eq!(sequences, [3, 4, 5, 6]);
    }

    #[test]
    fn the_sequence_wraps_at_eleven_bits() {
        let mut ps = PlayerState::ZERO;
        ps.predictable_event_sequence = 0x7ff;
        add_event(&mut ps, 5);
        assert_eq!(ps.predictable_event_sequence, 0);
        let mut consumed = 0x7ff;
        assert_eq!(heard(&ps, &mut consumed), []);
        assert_eq!(consumed, 0);
    }
}
