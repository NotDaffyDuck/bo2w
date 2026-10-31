//! The host's memory of each client's action requests.

use std::collections::{BTreeMap, HashMap};

use bevy::prelude::Resource;
use sim::{IntentSerial, PlayerIntent, ClientId, intent_serial};

use crate::net_links::once_queue::ActionRulingNotice;

/// How many answered requests the host keeps per client before forgetting the oldest.
pub const RECALLED_COMMANDS_PER_RECEIVER_CAP: usize = 256;

/// What the host makes of a request that has just arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionAcceptanceResult {
    /// Never seen: run it.
    Fresh,
    /// A resend of a request still being worked on.
    Pending,
    /// A resend of one already answered: send the same verdict again.
    Repeat(ActionRulingNotice),
    /// The id was used before for a different action.
    DuplicateIdDifferentBody,
    /// Too old: its answer has been forgotten.
    Expired,
}

#[derive(Clone, Debug, Default)]
struct PlayerActionTrace {
    /// Taken in, no verdict yet.
    in_flight: BTreeMap<IntentSerial, PlayerIntent>,
    /// Answered.
    resolved: BTreeMap<IntentSerial, (PlayerIntent, ActionRulingNotice)>,
    /// Every id up to and including this one has had its answer forgotten.
    forgotten_through: Option<IntentSerial>,
}

impl PlayerActionTrace {
    fn forget_oldest_over_the_limit(&mut self) {
        while self.resolved.len() > RECALLED_COMMANDS_PER_RECEIVER_CAP {
            let Some((oldest, _)) = self.resolved.pop_first() else {
                break;
            };
            self.forgotten_through = Some(self.forgotten_through.map_or(oldest, |t| t.max(oldest)));
        }
    }
}

/// Every client's requests, for the current match.
#[derive(Resource, Debug, Default)]
pub struct PlayerActionBook {
    match_stamp: u64,
    per_client: HashMap<ClientId, PlayerActionTrace>,
}

impl PlayerActionBook {
    /// Changes every time the ledger is cleared, so a verdict can be tied to the match it
    /// belongs to.
    pub fn match_stamp(&self) -> u64 {
        self.match_stamp
    }

    /// Forget every client's requests and begin a new scope.
    pub fn begin_timed_region(&mut self) {
        self.match_stamp = self.match_stamp.wrapping_add(1);
        self.per_client.clear();
    }

    /// Forget one client, who has left.
    pub fn drop_player_record(&mut self, leaver: ClientId) {
        self.per_client.remove(&leaver);
    }

    /// How many of the client's answered requests are still remembered.
    pub fn answered_count(&self, peer: ClientId) -> usize {
        self.per_client.get(&peer).map_or(0, |trace| trace.resolved.len())
    }

    /// The verdict given to the client's request, if still remembered.
    pub fn ruling_for(&self, peer: ClientId, request_id: IntentSerial) -> Option<ActionRulingNotice> {
        let (_, ruling) = self.per_client.get(&peer)?.resolved.get(&request_id)?;
        Some(*ruling)
    }

    /// Classify a request as it arrives; a fresh one is taken in as pending.
    pub fn weigh(&mut self, peer: ClientId, action: &PlayerIntent) -> ActionAcceptanceResult {
        let request_id = intent_serial(action);
        let trace = self.per_client.entry(peer).or_default();
        if let Some(pending) = trace.in_flight.get(&request_id) {
            return if pending == action {
                ActionAcceptanceResult::Pending
            } else {
                ActionAcceptanceResult::DuplicateIdDifferentBody
            };
        }
        if let Some((answered, ruling)) = trace.resolved.get(&request_id) {
            return if answered == action {
                ActionAcceptanceResult::Repeat(*ruling)
            } else {
                ActionAcceptanceResult::DuplicateIdDifferentBody
            };
        }
        if trace.forgotten_through.is_some_and(|through| request_id <= through) {
            return ActionAcceptanceResult::Expired;
        }
        trace.in_flight.insert(request_id, *action);
        ActionAcceptanceResult::Fresh
    }

    /// Note the verdict for a request, forgetting the oldest answers beyond the limit.
    pub fn log_ruling(&mut self, peer: ClientId, action: PlayerIntent, ruling: ActionRulingNotice) {
        let request_id = intent_serial(&action);
        let trace = self.per_client.entry(peer).or_default();
        trace.in_flight.remove(&request_id);
        trace.resolved.insert(request_id, (action, ruling));
        trace.forget_oldest_over_the_limit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: ClientId = ClientId(3);

    fn give(serial: IntentSerial, gun: u32) -> PlayerIntent {
        PlayerIntent::HandOutGun { request_id: serial, gun }
    }

    #[test]
    fn a_new_request_is_fresh_and_then_pending_when_resent() {
        let mut ledger = PlayerActionBook::default();
        assert_eq!(ledger.weigh(CLIENT, &give(1, 5)), ActionAcceptanceResult::Fresh);
        assert_eq!(ledger.weigh(CLIENT, &give(1, 5)), ActionAcceptanceResult::Pending);
        assert_eq!(ledger.answered_count(CLIENT), 0, "pending requests are not answered yet");
    }

    #[test]
    fn a_reused_id_with_other_contents_is_a_mismatch() {
        let mut ledger = PlayerActionBook::default();
        ledger.weigh(CLIENT, &give(1, 5));
        assert_eq!(ledger.weigh(CLIENT, &give(1, 6)), ActionAcceptanceResult::DuplicateIdDifferentBody);

        ledger.log_ruling(CLIENT, give(1, 5), ActionRulingNotice::TakenIntoEffect);
        assert_eq!(ledger.weigh(CLIENT, &give(1, 6)), ActionAcceptanceResult::DuplicateIdDifferentBody);
    }

    #[test]
    fn an_answered_request_repeats_its_verdict() {
        let mut ledger = PlayerActionBook::default();
        ledger.weigh(CLIENT, &give(1, 5));
        ledger.log_ruling(CLIENT, give(1, 5), ActionRulingNotice::Declined);
        assert_eq!(ledger.answered_count(CLIENT), 1);
        assert_eq!(ledger.ruling_for(CLIENT, 1), Some(ActionRulingNotice::Declined));
        assert_eq!(ledger.weigh(CLIENT, &give(1, 5)), ActionAcceptanceResult::Repeat(ActionRulingNotice::Declined));
        assert_eq!(ledger.ruling_for(CLIENT, 2), None);
        assert_eq!(ledger.ruling_for(ClientId(9), 1), None);
    }

    #[test]
    fn clients_are_remembered_separately() {
        let mut ledger = PlayerActionBook::default();
        ledger.weigh(CLIENT, &give(1, 5));
        assert_eq!(ledger.weigh(ClientId(4), &give(1, 6)), ActionAcceptanceResult::Fresh);
    }

    #[test]
    fn answers_past_the_limit_are_forgotten_oldest_first_and_then_expire() {
        let mut ledger = PlayerActionBook::default();
        let total = RECALLED_COMMANDS_PER_RECEIVER_CAP as u32 + 3;
        for id in 1..=total {
            ledger.weigh(CLIENT, &give(id, 5));
            ledger.log_ruling(CLIENT, give(id, 5), ActionRulingNotice::TakenIntoEffect);
        }
        assert_eq!(ledger.answered_count(CLIENT), RECALLED_COMMANDS_PER_RECEIVER_CAP);
        assert_eq!(ledger.ruling_for(CLIENT, 3), None);
        assert_eq!(ledger.ruling_for(CLIENT, 4), Some(ActionRulingNotice::TakenIntoEffect));
        for id in 1..=3 {
            assert_eq!(ledger.weigh(CLIENT, &give(id, 5)), ActionAcceptanceResult::Expired, "id {id}");
        }
        assert_eq!(ledger.weigh(CLIENT, &give(total + 1, 5)), ActionAcceptanceResult::Fresh);
        assert_eq!(ledger.weigh(CLIENT, &give(4, 5)), ActionAcceptanceResult::Repeat(ActionRulingNotice::TakenIntoEffect));
    }

    #[test]
    fn opening_a_scope_forgets_everyone_and_changes_the_scope() {
        let mut ledger = PlayerActionBook::default();
        let before = ledger.match_stamp();
        ledger.log_ruling(CLIENT, give(1, 5), ActionRulingNotice::TakenIntoEffect);
        ledger.begin_timed_region();
        assert_ne!(ledger.match_stamp(), before);
        assert_eq!(ledger.answered_count(CLIENT), 0);
        assert_eq!(ledger.weigh(CLIENT, &give(1, 5)), ActionAcceptanceResult::Fresh);
    }

    #[test]
    fn retiring_a_client_forgets_only_that_client() {
        let mut ledger = PlayerActionBook::default();
        ledger.log_ruling(CLIENT, give(1, 5), ActionRulingNotice::TakenIntoEffect);
        ledger.log_ruling(ClientId(4), give(1, 5), ActionRulingNotice::TakenIntoEffect);
        ledger.drop_player_record(CLIENT);
        assert_eq!(ledger.answered_count(CLIENT), 0);
        assert_eq!(ledger.answered_count(ClientId(4)), 1);
    }
}
