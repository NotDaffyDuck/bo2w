//! Gameplay actions (join, leave, pick a class, ...) waiting for the authority.
//!
//! A client's own actions sit in `open_actions` until the authority resolves them by request id. Actions
//! that arrive from remote peers go to `remote_arrivals` and are handed over once.

use bevy::prelude::Resource;
use sim::{IntentSerial, PlayerIntent, ClientId, intent_serial};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// The most unresolved actions one client may hold.
pub const PLAYER_ACTION_QUEUE_LIMIT: usize = 32;

/// How long a dispatched action may stay unresolved before `deadline_passed` reports it.
const RESOLVE_DEADLINE: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionQueueRefusal {
    IntakeSaturated { pending: usize },

    RepeatedRequestNumber { request_id: IntentSerial },
}

impl core::fmt::Display for ActionQueueRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IntakeSaturated { pending } => {
                write!(f, "action queue full ({pending} unresolved requests)")
            }
            Self::RepeatedRequestNumber { request_id } => {
                write!(
                    f,
                    "request_id {request_id} is already pending with another payload"
                )
            }
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct PlayerActionIntake {
    open_actions: Vec<(ClientId, PlayerIntent)>,
    dispatched_at: HashMap<(ClientId, IntentSerial), Instant>,

    remote_arrivals: Vec<(ClientId, PlayerIntent)>,
    flooded: HashSet<ClientId>,
}

/// Is `entry` the action `id` sent under `request_id`?
fn is_request(entry: &(ClientId, PlayerIntent), asker: ClientId, serial: IntentSerial) -> bool {
    entry.0 == asker && intent_serial(&entry.1) == serial
}

impl PlayerActionIntake {
    /// Queue an action from `id`. The same action twice is accepted as one; a different action
    /// under a request id that is still pending is refused, and so is anything past the limit.
    pub fn push(&mut self, asker: ClientId, action: PlayerIntent) -> Result<(), ActionQueueRefusal> {
        let request_id = intent_serial(&action);
        let same_id = self.awaiting_from(asker).find(|queued| intent_serial(queued) == request_id);
        if let Some(queued) = same_id {
            return match queued == action {
                true => Ok(()),
                false => Err(ActionQueueRefusal::RepeatedRequestNumber { request_id }),
            };
        }
        let held = self.awaiting_from(asker).count();
        if held >= PLAYER_ACTION_QUEUE_LIMIT {
            return Err(ActionQueueRefusal::IntakeSaturated { pending: held });
        }
        self.open_actions.push((asker, action));
        Ok(())
    }

    /// Start the resolve clock for a pending action, once.
    pub fn begin_clock(&mut self, asker: ClientId, serial: IntentSerial) {
        if self.is_pending(asker, serial) {
            let clock = (asker, serial);
            self.dispatched_at.entry(clock).or_insert_with(Instant::now);
        }
    }

    /// Queue an action that arrived from a remote peer. Repeats are dropped; a peer over the limit
    /// is remembered as having overflowed.
    pub fn accept_remote(&mut self, asker: ClientId, action: PlayerIntent) {
        let entry = (asker, action);
        if self.remote_arrivals.contains(&entry) {
            return;
        }
        let held = self.remote_arrivals.iter().filter(|(sender, _)| *sender == asker).count();
        if held < PLAYER_ACTION_QUEUE_LIMIT {
            self.remote_arrivals.push(entry);
        } else {
            self.flooded.insert(asker);
        }
    }

    pub fn drain_flooded(&mut self) -> impl Iterator<Item = ClientId> + '_ {
        self.flooded.drain()
    }

    /// Has any dispatched action gone unresolved for too long?
    pub fn deadline_passed(&self) -> bool {
        let late = |since: &Instant| since.elapsed() > RESOLVE_DEADLINE;
        self.dispatched_at.values().any(late)
    }

    pub fn waiting_count(&self) -> usize {
        self.open_actions.len()
    }

    pub fn pending(&self) -> &[(ClientId, PlayerIntent)] {
        &self.open_actions
    }

    pub fn awaiting_from(&self, asker: ClientId) -> impl Iterator<Item = PlayerIntent> + '_ {
        let owns = move |entry: &&(ClientId, PlayerIntent)| entry.0 == asker;
        self.open_actions.iter().filter(owns).map(|entry| entry.1)
    }

    /// Drop the resolved action; says whether it was pending. A client holds at most one action
    /// per request id, so there is never a second one to find.
    pub fn resolve(&mut self, asker: ClientId, serial: IntentSerial) -> bool {
        self.dispatched_at.remove(&(asker, serial));
        let found = self.open_actions.iter().position(|entry| is_request(entry, asker, serial));
        found.map(|at| self.open_actions.remove(at)).is_some()
    }

    pub fn drop_player_record(&mut self, asker: ClientId) {
        let keep = |entry: &(ClientId, PlayerIntent)| entry.0 != asker;
        self.open_actions.retain(keep);
        self.remote_arrivals.retain(keep);
        self.dispatched_at.retain(|clock, _| clock.0 != asker);
        self.flooded.remove(&asker);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Everything to act on this frame: all pending actions (which stay pending until retired, and
    /// are marked dispatched) followed by the ones received from peers (which are handed over).
    pub fn collect_due(&mut self) -> Vec<(ClientId, PlayerIntent)> {
        for (id, action) in &self.open_actions {
            let clock = (*id, intent_serial(action));
            self.dispatched_at.entry(clock).or_insert_with(Instant::now);
        }
        self.open_actions.iter().copied().chain(self.remote_arrivals.drain(..)).collect()
    }

    fn is_pending(&self, asker: ClientId, serial: IntentSerial) -> bool {
        self.open_actions.iter().any(|entry| is_request(entry, asker, serial))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: ClientId = ClientId(1);
    const OTHER: ClientId = ClientId(2);

    fn join(request_id: u32) -> PlayerIntent {
        PlayerIntent::JoinMatch { request_id }
    }

    fn leave(request_id: u32) -> PlayerIntent {
        PlayerIntent::LeaveMatch { request_id }
    }

    #[test]
    fn the_same_action_twice_is_one_pending_action() {
        let mut queue = PlayerActionIntake::default();
        queue.push(ME, join(5)).unwrap();
        queue.push(ME, join(5)).unwrap();
        assert_eq!(queue.waiting_count(), 1);
    }

    #[test]
    fn another_payload_under_a_pending_request_id_is_refused() {
        let mut queue = PlayerActionIntake::default();
        queue.push(ME, join(5)).unwrap();
        assert_eq!(
            queue.push(ME, leave(5)),
            Err(ActionQueueRefusal::RepeatedRequestNumber { request_id: 5 })
        );
        // Another client may use the same id.
        queue.push(OTHER, leave(5)).unwrap();
    }

    #[test]
    fn a_client_over_the_limit_is_refused_with_its_count() {
        let mut queue = PlayerActionIntake::default();
        for id in 0..PLAYER_ACTION_QUEUE_LIMIT as u32 {
            queue.push(ME, join(id)).unwrap();
        }
        assert_eq!(
            queue.push(ME, join(1000)),
            Err(ActionQueueRefusal::IntakeSaturated {
                pending: PLAYER_ACTION_QUEUE_LIMIT
            })
        );
        queue.push(OTHER, join(1000)).unwrap();
    }

    #[test]
    fn retiring_removes_the_action_and_says_so() {
        let mut queue = PlayerActionIntake::default();
        queue.push(ME, join(5)).unwrap();
        assert!(!queue.resolve(OTHER, 5));
        assert!(queue.resolve(ME, 5));
        assert!(!queue.resolve(ME, 5));
        assert_eq!(queue.waiting_count(), 0);
    }

    #[test]
    fn collect_dueing_marks_pending_dispatched_and_hands_over_received_once() {
        let mut queue = PlayerActionIntake::default();
        queue.push(ME, join(5)).unwrap();
        queue.accept_remote(OTHER, leave(9));
        let first = queue.collect_due();
        assert_eq!(first, vec![(ME, join(5)), (OTHER, leave(9))]);
        assert!(queue.dispatched_at.contains_key(&(ME, 5)));
        // The pending action stays; the received one was handed over.
        assert_eq!(queue.collect_due(), vec![(ME, join(5))]);
        assert!(!queue.deadline_passed());
    }

    #[test]
    fn only_pending_actions_can_be_marked_dispatched() {
        let mut queue = PlayerActionIntake::default();
        queue.begin_clock(ME, 5);
        assert!(queue.dispatched_at.is_empty());
    }

    #[test]
    fn a_peer_that_floods_is_remembered_once_and_repeats_are_ignored() {
        let mut queue = PlayerActionIntake::default();
        queue.accept_remote(OTHER, join(0));
        queue.accept_remote(OTHER, join(0));
        for id in 1..PLAYER_ACTION_QUEUE_LIMIT as u32 + 4 {
            queue.accept_remote(OTHER, join(id));
        }
        assert_eq!(queue.drain_flooded().collect::<Vec<_>>(), vec![OTHER]);
        assert_eq!(queue.drain_flooded().count(), 0);
        assert_eq!(queue.collect_due().len(), PLAYER_ACTION_QUEUE_LIMIT);
    }

    #[test]
    fn retiring_a_client_drops_everything_it_owned() {
        let mut queue = PlayerActionIntake::default();
        queue.push(ME, join(1)).unwrap();
        queue.push(OTHER, join(2)).unwrap();
        queue.accept_remote(ME, leave(3));
        queue.collect_due();
        queue.drop_player_record(ME);
        assert_eq!(queue.awaiting_from(ME).count(), 0);
        assert_eq!(queue.awaiting_from(OTHER).collect::<Vec<_>>(), vec![join(2)]);
        assert_eq!(queue.dispatched_at.len(), 1);
        queue.clear();
        assert_eq!(queue.waiting_count(), 0);
        assert!(queue.collect_due().is_empty());
    }
}
