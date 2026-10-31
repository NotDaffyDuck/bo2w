//! The joining side of the relay: one `PlayerUdpSession` per client. It reads the authority's
//! snapshots out of the mailbox (`receive`), keeps what it has applied (`ledger`), answers
//! bootstrap offers (`offers`) and sends commands back (`send`).

mod ledger;
mod offers;
mod receive;
mod send;

use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::Resource;
use master_protocol::RoomOccupantId;
use sim::ClientId;

use self::ledger::Ledger;
use super::mailbox::{ForwardingInbox, send_refused};
use crate::net_links::match_handoff::{TransferChannelState, InitialStateTransferMsg};
use crate::net_links::datagrams::{LinkNumber, JoinGreeting, JoinRefusal, NegotiatedCaps};
use crate::net_links::send_fault::UdpTransmitFailure;

/// The authority is the only peer a client talks to; its member id is all zeroes.
const AUTHORITY: RoomOccupantId = RoomOccupantId([0; 16]);

/// What a client learns while it is in one match. Leaving the match drops all of it at once.
#[derive(Default)]
struct Round {
    /// Snapshots received and applied, kept as delta bases.
    ledger: Ledger,
    /// Sequence of the newest packet this client sent.
    last_sent: u32,
    /// Sequence of the newest authority packet taken in; echoed back as the confirm_receipt.
    last_heard: u32,
    /// The bootstrap this client applied last; zero never counts.
    bootstrap_applied: Option<u32>,
    /// (bootstrap id, snapshot seq) of the last offer whose snapshot was applied.
    offer_applied: Option<(u32, u32)>,
    /// Confirmations waiting for the bootstrap lane to take them.
    confirms_due: Vec<InitialStateTransferMsg>,
    /// Offers read from the lane that could not be handled yet.
    offers_waiting: Vec<Vec<u8>>,
    /// Why the authority turned this client away, once it has.
    rejected: Option<JoinRefusal>,
    /// Reliable rows from the authority not yet taken by the game.
    controls: Vec<crate::GuaranteedEnvelope>,
    /// Action requests already put on the wire.
    actions_sent: HashSet<sim::IntentSerial>,
}

#[derive(Resource)]
pub struct PlayerUdpSession {
    mailbox: ForwardingInbox,
    pub hello: JoinGreeting,
    pub negotiated: NegotiatedCaps,
    pub connection: Option<LinkNumber>,
    pub granted_player_slot: Option<ClientId>,
    offer_lane: Option<Arc<TransferChannelState>>,
    round: Round,
}

impl PlayerUdpSession {
    pub fn relay(hello: JoinGreeting, mailbox: ForwardingInbox) -> Self {
        Self {
            negotiated: hello.limits,
            hello,
            mailbox,
            connection: Default::default(),
            granted_player_slot: Default::default(),
            offer_lane: Default::default(),
            round: Round::default(),
        }
    }

    pub fn mailbox(&self) -> ForwardingInbox {
        ForwardingInbox::clone(&self.mailbox)
    }

    pub fn bind_offer_lane(&mut self, chan: Arc<TransferChannelState>) {
        self.offer_lane.replace(chan);
    }

    pub fn greeting_refusal(&self) -> Option<JoinRefusal> {
        self.round.rejected
    }

    pub fn took_in_world_state(&self) -> bool {
        self.round.ledger.newest().is_some()
    }

    /// The match epoch packets are stamped with; zero when there is no bootstrap lane.
    pub fn game_generation(&self) -> u32 {
        self.offer_lane.as_deref().map(TransferChannelState::era).unwrap_or_default()
    }

    /// Without a bootstrap lane a client is in from the start. With one, it is in once the
    /// bootstrap it applied is the one the game thread has entered.
    pub fn is_inside_match(&self) -> bool {
        match (&self.offer_lane, self.entry_setup_token()) {
            (None, _) => true,
            (Some(lane), Some(applied)) => applied == lane.joined_setup_stage(),
            (Some(_), None) => false,
        }
    }

    pub fn entry_setup_token(&self) -> Option<u32> {
        match self.round.bootstrap_applied {
            Some(0) | None => None,
            some => some,
        }
    }

    pub fn mark_setup_applied(&mut self, setup_id: u32) {
        self.round.bootstrap_applied.replace(setup_id);
    }

    pub fn record_refusal(&mut self, reason: JoinRefusal) {
        self.round.rejected.replace(reason);
    }

    /// An accept after a reject is ignored: the reject stands.
    pub fn note_admission(&mut self, link_id: LinkNumber, slot: ClientId) {
        if self.round.rejected.is_some() {
            return;
        }
        self.connection.replace(link_id);
        self.granted_player_slot.replace(slot);
    }

    pub fn note_snapshot_applied(&mut self, snap_no: u32) {
        self.round.ledger.note_applied(snap_no);
    }

    /// The reliable rows the authority sent since the last call.
    pub fn take_controls(&mut self) -> Vec<crate::GuaranteedEnvelope> {
        self.round.controls.drain(..).collect()
    }

    /// Back to the state of a client that has not met the authority yet.
    pub fn clear_for_new_game(&mut self) {
        self.round = Round::default();
        self.connection.take();
        self.granted_player_slot.take();
        if let Some(lane) = self.offer_lane.as_deref() {
            lane.assign_generation(0);
        }
    }

    /// Everything a client sends goes to the one peer it has, the authority.
    fn send_to_authority(&self, raw: Vec<u8>) -> Result<(), UdpTransmitFailure> {
        let pushed = self.mailbox.queue_departure(AUTHORITY, raw);
        pushed.map_err(send_refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> PlayerUdpSession {
        PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        )
    }

    fn empty_rows(confirmed_up_to: u16) -> crate::GuaranteedEnvelope {
        crate::GuaranteedEnvelope {
            confirmed_up_to,
            entries: vec![],
            evicted_stale_count: 0,
        }
    }

    #[test]
    fn a_fresh_link_knows_nobody() {
        let l = fresh();
        assert_eq!((l.connection, l.granted_player_slot), (None, None));
        assert!(!l.took_in_world_state());
        assert_eq!(l.game_generation(), 0);
        assert_eq!(l.greeting_refusal(), None);
    }

    #[test]
    fn no_bootstrap_lane_means_already_entered() {
        assert!(fresh().is_inside_match());
    }

    #[test]
    fn an_accept_stores_connection_and_client() {
        let mut l = fresh();
        l.note_admission(LinkNumber(4), ClientId(2));
        assert_eq!(
            (l.connection, l.granted_player_slot),
            (Some(LinkNumber(4)), Some(ClientId(2)))
        );
    }

    #[test]
    fn an_accept_after_a_reject_changes_nothing() {
        let mut l = fresh();
        l.record_refusal(JoinRefusal::Limits);
        l.note_admission(LinkNumber(4), ClientId(2));
        assert_eq!(l.connection, None);
        assert!(l.greeting_refusal().is_some());
    }

    #[test]
    fn bootstrap_zero_never_admits() {
        let mut l = fresh();
        let mut seen = vec![l.entry_setup_token()];
        for id in [0, 6] {
            l.mark_setup_applied(id);
            seen.push(l.entry_setup_token());
        }
        assert_eq!(seen, [None, None, Some(6)]);
    }

    #[test]
    fn an_applied_snapshot_sticks() {
        let mut l = fresh();
        l.note_snapshot_applied(3);
        assert!(l.took_in_world_state());
    }

    #[test]
    fn reset_match_clears_the_round() {
        let mut l = fresh();
        l.note_admission(LinkNumber(4), ClientId(2));
        l.note_snapshot_applied(3);
        l.mark_setup_applied(6);
        l.round.last_sent = 9;
        l.round.last_heard = 8;
        l.round.controls.push(empty_rows(0));

        l.clear_for_new_game();

        assert_eq!(l.connection, None);
        assert!(!l.took_in_world_state());
        assert_eq!(l.entry_setup_token(), None);
        assert_eq!([l.round.last_sent, l.round.last_heard], [0, 0]);
        assert_eq!(l.take_controls().len(), 0);
    }

    #[test]
    fn controls_are_handed_over_once() {
        let mut l = fresh();
        l.round.controls.push(empty_rows(1));
        assert_eq!(l.take_controls().len(), 1);
        assert_eq!(l.take_controls().len(), 0);
    }

    #[test]
    fn sends_are_queued_for_the_authority() {
        let l = fresh();
        assert!(l.send_to_authority(vec![1, 2]).is_ok());
        assert_eq!(l.mailbox().take_outbound(), [(AUTHORITY, vec![1, 2])]);
    }
}
