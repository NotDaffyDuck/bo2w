//! The ring of predictable events a player state carries, and the client
//! side that plays each one exactly once.

use entity_t6::event_ring::EVENT_SLOTS;
use entity_t6::event_ring::EVENT_SEQUENCE_WRAP;
use entity_t6::event_ring::unwrap_previous;
use playerstate_t6::game::AvatarState;

/// Event sequence numbers wrap like BO2's ring (`crate::event_ring`): the mask is one under the wrap.
const SEQUENCE_MASK: i32 = EVENT_SEQUENCE_WRAP as i32 - 1;
/// How many events the ring holds before the oldest is overwritten.
const RING_LEN: i32 = EVENT_SLOTS as i32;

/// Record `event` with `parm` in the next slot of the ring and advance the
/// sequence. Event 0 means "nothing" and is not recorded.
pub fn queue_forecast_happening(avatar: &mut AvatarState, event: i32, parm: i32) {
    if event == 0 {
        return;
    }
    let slot = (avatar.predictable_event_sequence & 3) as usize;
    avatar.predictable_events[slot] = event;
    avatar.predictable_event_parms[slot] = parm as u32;
    avatar.predictable_event_sequence =
        avatar.predictable_event_sequence.wrapping_add(1) & SEQUENCE_MASK as i16;
}

/// Record an event that carries no parameter.
pub fn add_event(avatar: &mut AvatarState, ev: i32) {
    queue_forecast_happening(avatar, ev, 0);
}

/// One event to play, tagged with the sequence it was recorded under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderedHappening {
    pub seq_no: i32,
    pub event: i32,
    pub event_arg: i32,
}

/// Hand every event recorded since `consumed_sequence` to `dispatch`, oldest
/// first, and move `consumed_sequence` up to the newest. Events that fell out
/// of the ring are lost; a sequence that wrapped is unwrapped first.
pub fn drain_player_happenings(
    avatar: &AvatarState,
    seen_seq: &mut i32,
    mut dispatch: impl FnMut(OrderedHappening),
) {
    let newest = i32::from(avatar.predictable_event_sequence);
    if *seen_seq == newest {
        return;
    }
    if newest == 0 {
        *seen_seq = 0;
        return;
    }

    let mut next = i32::from(unwrap_previous(avatar.predictable_event_sequence, *seen_seq as i16));
    if newest - next > RING_LEN {
        next = newest - RING_LEN;
    }
    for sequence in next..newest {
        let slot = (sequence as u32 & 3) as usize;
        let event = avatar.predictable_events[slot];
        if event != 0 {
            dispatch(OrderedHappening {
                seq_no: sequence & SEQUENCE_MASK,
                event,
                event_arg: avatar.predictable_event_parms[slot] as i32,
            });
        }
    }
    *seen_seq = newest;
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn heard(avatar: &AvatarState, consumed: &mut i32) -> Vec<OrderedHappening> {
        let mut out = Vec::new();
        drain_player_happenings(avatar, consumed, |heard| out.push(heard));
        out
    }

    #[test]
    fn events_are_recorded_in_order_and_zero_is_ignored() {
        let mut avatar = AvatarState::ZERO;
        queue_forecast_happening(&mut avatar, 0, 9);
        assert_eq!(avatar.predictable_event_sequence, 0);
        queue_forecast_happening(&mut avatar, 130, 7);
        add_event(&mut avatar, 136);
        assert_eq!(avatar.predictable_event_sequence, 2);
        assert_eq!(avatar.predictable_events[..2], [130, 136]);
        assert_eq!(avatar.predictable_event_parms[..2], [7, 0]);
    }

    #[test]
    fn a_listener_hears_each_event_once() {
        let mut avatar = AvatarState::ZERO;
        queue_forecast_happening(&mut avatar, 130, 7);
        add_event(&mut avatar, 136);
        let mut seen = 0;
        let first = heard(&avatar, &mut seen);
        assert_eq!(
            first,
            [
                OrderedHappening {
                    seq_no: 0_i32,
                    event: 130,
                    event_arg: 7
                },
                OrderedHappening {
                    seq_no: 1,
                    event: 136,
                    event_arg: 0
                },
            ]
        );
        assert_eq!(seen, 2);
        assert!(heard(&avatar, &mut seen).is_empty());
    }

    #[test]
    fn a_listener_that_fell_behind_only_gets_what_the_ring_still_holds() {
        let mut avatar = AvatarState::ZERO;
        for event in 1..=6 {
            add_event(&mut avatar, event);
        }
        let mut consumed = 0;
        let sequences: Vec<i32> = heard(&avatar, &mut consumed).iter().map(|e| e.event).collect();
        assert_eq!(sequences, [3, 4, 5, 6]);
    }

    #[test]
    fn the_sequence_wraps_at_eight_bits() {
        let mut avatar = AvatarState::ZERO;
        avatar.predictable_event_sequence = 0xff;
        add_event(&mut avatar, 5);
        assert_eq!(avatar.predictable_event_sequence, 0);
        let mut seen = 0xff;
        assert_eq!(heard(&avatar, &mut seen), []);
        assert_eq!(seen, 0);
    }

    #[test]
    fn a_listener_across_the_wrap_still_hears_the_events_before_it() {
        let mut avatar = AvatarState::ZERO;
        avatar.predictable_event_sequence = 0xfe;
        add_event(&mut avatar, 7);
        add_event(&mut avatar, 8);
        add_event(&mut avatar, 9);
        assert_eq!(avatar.predictable_event_sequence, 1);
        let mut seen = 0xfe;
        let heard_now = heard(&avatar, &mut seen);
        let events: Vec<i32> = heard_now.iter().map(|e| e.event).collect();
        let sequences: Vec<i32> = heard_now.iter().map(|e| e.seq_no).collect();
        assert_eq!(events, [7, 8, 9]);
        assert_eq!(sequences, [0xfe, 0xff, 0]);
        assert_eq!(seen, 1);
    }
}
