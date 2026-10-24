//! The joining side of the relay: one `UdpClientLink` per client. It reads the authority's
//! snapshots out of the mailbox (`receive`), keeps what it has applied (`ledger`), answers
//! bootstrap offers (`offers`) and sends commands back (`send`).

mod ledger;
mod offers;
mod receive;
mod send;

use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::Resource;
use master_protocol::MemberId;
use sim::ClientId;

use self::ledger::Ledger;
use super::mailbox::{RelayMailbox, send_refused};
use crate::transport::bootstrap::{BootstrapLane, BootstrapMessage};
use crate::transport::protocol::{ConnectionId, HandshakeHello, HandshakeReject, ProtocolLimits};
use crate::transport::udp_socket::UdpSendError;

#[derive(Resource)]
pub struct UdpClientLink {
    mailbox: RelayMailbox,
    pub hello: HandshakeHello,
    pub limits: ProtocolLimits,
    pub connection: Option<ConnectionId>,
    pub assigned_client: Option<ClientId>,

    ledger: Ledger,
    out_seq: u32,
    in_ack: u32,

    /// The bootstrap this client applied last; zero never counts.
    applied_bootstrap_id: Option<u32>,

    /// (bootstrap id, snapshot seq) of the last offer whose snapshot was applied.
    last_applied_offer: Option<(u32, u32)>,

    /// Confirmations waiting for the bootstrap lane to take them.
    unsent_applied: Vec<BootstrapMessage>,

    /// Offers read from the lane that could not be handled yet.
    held_offers: Vec<Vec<u8>>,

    failed: Option<HandshakeReject>,
    bootstrap: Option<Arc<BootstrapLane>>,
    controls: Vec<crate::ReliablePayload>,
    sent_actions: HashSet<sim::ActionRequestId>,
}

impl UdpClientLink {
    pub fn relay(hello: HandshakeHello, mailbox: RelayMailbox) -> Self {
        let limits = hello.limits;
        Self {
            mailbox,
            hello,
            limits,
            connection: None,
            assigned_client: None,
            ledger: Ledger::default(),
            out_seq: 0,
            in_ack: 0,
            applied_bootstrap_id: None,
            last_applied_offer: None,
            unsent_applied: Vec::new(),
            held_offers: Vec::new(),
            failed: None,
            bootstrap: None,
            controls: Vec::new(),
            sent_actions: HashSet::new(),
        }
    }

    pub fn mailbox(&self) -> RelayMailbox {
        self.mailbox.clone()
    }

    pub fn attach_bootstrap(&mut self, lane: Arc<BootstrapLane>) {
        self.bootstrap = Some(lane);
    }

    pub fn handshake_reject(&self) -> Option<HandshakeReject> {
        self.failed
    }

    pub fn has_applied_snapshot(&self) -> bool {
        self.ledger.newest().is_some()
    }

    /// The match epoch packets are stamped with; zero when there is no bootstrap lane.
    pub fn match_epoch(&self) -> u32 {
        self.bootstrap.as_ref().map_or(0, |lane| lane.epoch())
    }

    /// Without a bootstrap lane a client is in from the start. With one, it is in once the
    /// bootstrap it applied is the one the game thread has entered.
    pub fn has_entered_match(&self) -> bool {
        let Some(lane) = &self.bootstrap else {
            return true;
        };
        self.applied_bootstrap_id
            .is_some_and(|applied| applied != 0 && applied == lane.entered_bootstrap())
    }

    pub fn admission_bootstrap_id(&self) -> Option<u32> {
        self.applied_bootstrap_id.filter(|id| *id != 0)
    }

    pub fn note_applied_bootstrap(&mut self, bootstrap_id: u32) {
        self.applied_bootstrap_id = Some(bootstrap_id);
    }

    pub fn note_reject(&mut self, reason: HandshakeReject) {
        self.failed = Some(reason);
    }

    pub fn note_accept(&mut self, connection: ConnectionId, client: ClientId) {
        if self.failed.is_none() {
            self.connection = Some(connection);
            self.assigned_client = Some(client);
        }
    }

    pub fn note_applied_snapshot(&mut self, snapshot_seq: u32) {
        self.ledger.note_applied(snapshot_seq);
    }

    /// The reliable rows the authority sent since the last call.
    pub fn take_controls(&mut self) -> Vec<crate::ReliablePayload> {
        std::mem::take(&mut self.controls)
    }

    /// Back to the state of a client that has not met the authority yet.
    pub fn reset_match(&mut self) {
        self.connection = None;
        self.assigned_client = None;
        self.ledger = Ledger::default();
        self.out_seq = 0;
        self.in_ack = 0;
        self.applied_bootstrap_id = None;
        self.last_applied_offer = None;
        self.unsent_applied.clear();
        self.held_offers.clear();
        self.controls.clear();
        self.sent_actions.clear();
        self.failed = None;
        if let Some(lane) = &self.bootstrap {
            lane.set_epoch(0);
        }
    }

    /// Everything a client sends goes to the one peer it has, the authority.
    fn send_to_authority(&self, bytes: Vec<u8>) -> Result<(), UdpSendError> {
        self.mailbox
            .push_outbound(MemberId([0; 16]), bytes)
            .map_err(send_refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link() -> UdpClientLink {
        let hello = HandshakeHello::current(Default::default());
        UdpClientLink::relay(hello, RelayMailbox::with_default_cap())
    }

    #[test]
    fn a_new_link_has_met_no_one() {
        let link = link();
        assert!(link.connection.is_none());
        assert!(link.assigned_client.is_none());
        assert!(!link.has_applied_snapshot());
        assert_eq!(link.match_epoch(), 0);
        assert!(link.handshake_reject().is_none());
    }

    #[test]
    fn without_a_bootstrap_lane_the_client_counts_as_entered() {
        assert!(link().has_entered_match());
    }

    #[test]
    fn accepting_records_the_connection_and_client() {
        let mut link = link();
        link.note_accept(ConnectionId(4), ClientId(2));
        assert_eq!(link.connection, Some(ConnectionId(4)));
        assert_eq!(link.assigned_client, Some(ClientId(2)));
    }

    #[test]
    fn a_rejected_link_ignores_a_later_accept() {
        let mut link = link();
        link.note_reject(HandshakeReject::Limits);
        link.note_accept(ConnectionId(4), ClientId(2));
        assert!(link.connection.is_none());
        assert!(link.handshake_reject().is_some());
    }

    #[test]
    fn zero_is_not_an_admission_id() {
        let mut link = link();
        assert_eq!(link.admission_bootstrap_id(), None);
        link.note_applied_bootstrap(0);
        assert_eq!(link.admission_bootstrap_id(), None);
        link.note_applied_bootstrap(6);
        assert_eq!(link.admission_bootstrap_id(), Some(6));
    }

    #[test]
    fn applying_a_snapshot_is_remembered() {
        let mut link = link();
        link.note_applied_snapshot(3);
        assert!(link.has_applied_snapshot());
    }

    #[test]
    fn resetting_forgets_everything() {
        let mut link = link();
        link.note_accept(ConnectionId(4), ClientId(2));
        link.note_applied_snapshot(3);
        link.note_applied_bootstrap(6);
        link.out_seq = 9;
        link.in_ack = 8;
        link.controls.push(crate::ReliablePayload {
            ack_through: 0,
            rows: Vec::new(),
            dropped_oldest: 0,
        });

        link.reset_match();

        assert!(link.connection.is_none());
        assert!(!link.has_applied_snapshot());
        assert_eq!(link.admission_bootstrap_id(), None);
        assert_eq!((link.out_seq, link.in_ack), (0, 0));
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn taking_controls_empties_them() {
        let mut link = link();
        link.controls.push(crate::ReliablePayload {
            ack_through: 1,
            rows: Vec::new(),
            dropped_oldest: 0,
        });
        assert_eq!(link.take_controls().len(), 1);
        assert!(link.take_controls().is_empty());
    }

    #[test]
    fn what_a_client_sends_lands_in_the_outbound_lane_for_the_authority() {
        let link = link();
        link.send_to_authority(vec![1, 2]).unwrap();
        assert_eq!(
            link.mailbox().take_outbound(),
            vec![(MemberId([0; 16]), vec![1, 2])]
        );
    }
}
