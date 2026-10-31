use std::collections::HashMap;

use bevy::prelude::Resource;
use sim::{IntentSerial, ClientId, MatchOccurrence};

use super::queue::{GuaranteedEventBacklog, GuaranteedEnvelope};
use super::row::{ActionRulingNotice, GuaranteedEntry};

/// Every peer's queue.
#[derive(Resource, Clone, Debug, Default)]
pub struct GuaranteedEventRouter {
    backlogs: HashMap<ClientId, GuaranteedEventBacklog>,
}

impl GuaranteedEventRouter {
    pub fn backlog_of(&self, peer: ClientId) -> Option<&GuaranteedEventBacklog> {
        self.backlogs.get(&peer)
    }

    /// The peer's queue, made if they had none.
    pub fn backlog_for_player(&mut self, peer: ClientId) -> &mut GuaranteedEventBacklog {
        self.backlogs
            .entry(peer)
            .or_insert_with(GuaranteedEventBacklog::new)
    }

    pub fn post_event(&mut self, peer: ClientId, event: MatchOccurrence) -> u16 {
        self.backlog_for_player(peer).post_event(event)
    }

    /// Tell `peer` how their request went. The same answer to the same request is only queued
    /// once; asking again returns the sequence number it already has.
    pub fn enqueue_resolution(
        &mut self,
        peer: ClientId,
        request_id: IntentSerial,
        verdict: ActionRulingNotice,
    ) -> u16 {
        let queue = self.backlog_for_player(peer);
        let answer = GuaranteedEntry::CommandResolution {
            request_id,
            verdict,
        };
        match queue.owed_rows().iter().find(|(_, row)| *row == answer) {
            Some((seq, _)) => *seq,
            None => queue.enqueue_resolution(request_id, verdict),
        }
    }

    /// What to send `peer` now; nothing owed if they have no queue.
    pub fn owed_envelope(&self, peer: ClientId) -> GuaranteedEnvelope {
        self.backlogs
            .get(&peer)
            .map_or_else(GuaranteedEnvelope::has_no_debt, GuaranteedEventBacklog::envelope)
    }

    pub fn confirm_receipt(&mut self, peer: ClientId, upto: u16) {
        if let Some(queue) = self.backlogs.get_mut(&peer) {
            queue.confirm_receipt(upto);
        }
    }

    /// Acknowledge everything queued for a peer that has no wire to confirm_receipt over (a bot the host
    /// drives). Left alone, the broadcasts every peer gets would pile past the queue's limit and
    /// the bot would look like a connection that had fallen behind, and be retired as one.
    pub fn settle_all(&mut self, peer: ClientId) {
        let Some(queue) = self.backlogs.get_mut(&peer) else {
            return;
        };
        if let Some(&(last, _)) = queue.owed_rows().last() {
            queue.confirm_receipt(last);
        }
    }

    pub fn drop_backlog(&mut self, peer: ClientId) {
        self.backlogs.remove(&peer);
    }

    pub fn clear(&mut self) {
        self.backlogs.clear();
    }

    /// The clients whose queue has refused a row.
    pub fn overflowed_clients(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.backlogs
            .iter()
            .filter(|(_, queue)| queue.evicted_stale_count != 0)
            .map(|(peer, _)| *peer)
    }

    pub fn len(&self) -> usize {
        self.backlogs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backlogs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use sim::ClassId;

    use super::*;
    use crate::GUARANTEED_BACKLOG_LIMIT;

    const ALICE: ClientId = ClientId(1);
    const BOB: ClientId = ClientId(2);

    fn accepted(request_id: u32) -> MatchOccurrence {
        MatchOccurrence::LoadoutTaken {
            request_id,
            class_id: ClassId(1),
            revision: 1,
        }
    }

    #[test]
    fn queues_are_made_on_first_use_and_kept_apart() {
        let mut hub = GuaranteedEventRouter::default();
        assert!(hub.is_empty());
        assert!(hub.backlog_of(ALICE).is_none());
        assert_eq!(hub.post_event(ALICE, accepted(1)), 1);
        assert_eq!(hub.post_event(ALICE, accepted(2)), 2);
        assert_eq!(hub.post_event(BOB, accepted(3)), 1);
        assert_eq!(hub.len(), 2);
        assert_eq!(hub.backlog_of(ALICE).unwrap().waiting_count(), 2);
        assert_eq!(hub.backlog_of(BOB).unwrap().waiting_count(), 1);
    }

    #[test]
    fn the_same_answer_to_the_same_request_is_queued_once() {
        let mut hub = GuaranteedEventRouter::default();
        let first = hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::TakenIntoEffect);
        let again = hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::TakenIntoEffect);
        assert_eq!(first, again);
        assert_eq!(hub.backlog_of(ALICE).unwrap().waiting_count(), 1);
    }

    #[test]
    fn a_different_verdict_or_request_or_client_is_a_new_answer() {
        let mut hub = GuaranteedEventRouter::default();
        hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::TakenIntoEffect);
        hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::Declined);
        hub.enqueue_resolution(ALICE, 6, ActionRulingNotice::TakenIntoEffect);
        hub.enqueue_resolution(BOB, 5, ActionRulingNotice::TakenIntoEffect);
        assert_eq!(hub.backlog_of(ALICE).unwrap().waiting_count(), 3);
        assert_eq!(hub.backlog_of(BOB).unwrap().waiting_count(), 1);
    }

    #[test]
    fn an_answer_can_be_given_again_once_the_first_was_acked() {
        let mut hub = GuaranteedEventRouter::default();
        let first = hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::TakenIntoEffect);
        hub.confirm_receipt(ALICE, first);
        let second = hub.enqueue_resolution(ALICE, 5, ActionRulingNotice::TakenIntoEffect);
        assert_ne!(first, second);
    }

    #[test]
    fn a_client_with_no_queue_is_owed_nothing_and_an_ack_from_them_is_ignored() {
        let mut hub = GuaranteedEventRouter::default();
        assert_eq!(hub.owed_envelope(ALICE), GuaranteedEnvelope::has_no_debt());
        hub.confirm_receipt(ALICE, 4);
        hub.settle_all(ALICE);
        assert!(hub.is_empty());
    }

    #[test]
    fn the_payload_is_that_clients_queue() {
        let mut hub = GuaranteedEventRouter::default();
        hub.post_event(ALICE, accepted(1));
        hub.post_event(BOB, accepted(2));
        hub.post_event(BOB, accepted(3));
        assert_eq!(hub.owed_envelope(ALICE).entries.len(), 1);
        assert_eq!(hub.owed_envelope(BOB).entries.len(), 2);
    }

    #[test]
    fn ack_all_releases_everything_queued() {
        let mut hub = GuaranteedEventRouter::default();
        for n in 0..5 {
            hub.post_event(ALICE, accepted(n));
        }
        hub.settle_all(ALICE);
        assert_eq!(hub.backlog_of(ALICE).unwrap().waiting_count(), 0);
        assert_eq!(hub.backlog_of(ALICE).unwrap().confirmed_up_to(), 5);
    }

    #[test]
    fn a_client_that_is_never_acked_overflows_and_is_listed() {
        let mut hub = GuaranteedEventRouter::default();
        for n in 0..=GUARANTEED_BACKLOG_LIMIT as u32 {
            hub.post_event(ALICE, accepted(n));
        }
        hub.post_event(BOB, accepted(0));
        assert_eq!(hub.overflowed_clients().collect::<Vec<_>>(), [ALICE]);
    }

    #[test]
    fn a_retired_client_loses_their_queue_and_clearing_drops_all() {
        let mut hub = GuaranteedEventRouter::default();
        hub.post_event(ALICE, accepted(1));
        hub.post_event(BOB, accepted(2));
        hub.drop_backlog(ALICE);
        assert!(hub.backlog_of(ALICE).is_none());
        assert_eq!(hub.len(), 1);
        hub.clear();
        assert!(hub.is_empty());
    }
}
