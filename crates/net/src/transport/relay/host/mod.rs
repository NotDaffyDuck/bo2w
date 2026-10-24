//! The authority side of the relay: one `UdpAuthorityHub` per hosted match. It keeps a `Peer`
//! for every remote member, reads their packets (`ingress`) and answers with snapshots
//! (`fanout`). The packets travel through a `RelayMailbox`, never over a socket of its own.

mod fanout;
mod ingress;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::Resource;
use master_protocol::MemberId;
use sim::ClientId;

use super::admission::CommittedAdmission;
use super::mailbox::{RelayMailbox, send_refused};
use super::peer::Peer;
use crate::transport::bootstrap::BootstrapLane;
use crate::transport::protocol::{
    ConnectionId, ConnectionTable, HandshakeHello, PacketHeader, ProtocolLimits, ServerPacket,
};
use crate::transport::udp_socket::UdpSendError;

#[derive(Resource)]
pub struct UdpAuthorityHub {
    mailbox: RelayMailbox,
    pub hello: HandshakeHello,
    pub limits: ProtocolLimits,
    pub connections: ConnectionTable,
    peers: HashMap<ConnectionId, Peer>,

    /// Present when joiners are admitted through bootstrap offers.
    bootstrap: Option<Arc<BootstrapLane>>,
    committed: Vec<CommittedAdmission>,

    /// Members that must not be enrolled again until they are retired for good.
    denied: HashSet<MemberId>,
}

impl UdpAuthorityHub {
    pub fn relay(hello: HandshakeHello, first_client: u32, mailbox: RelayMailbox) -> Self {
        let limits = hello.limits;
        Self {
            mailbox,
            hello,
            limits,
            connections: ConnectionTable::starting_at(first_client),
            peers: HashMap::new(),
            bootstrap: None,
            committed: Vec::new(),
            denied: HashSet::new(),
        }
    }

    pub fn mailbox(&self) -> RelayMailbox {
        self.mailbox.clone()
    }

    pub fn attach_bootstrap(&mut self, lane: Arc<BootstrapLane>) {
        self.bootstrap = Some(lane);
    }

    pub fn reconcile_relay_membership(&mut self, members: &[MemberId], local: MemberId) {
        for &member in members {
            if member != local && !self.denied.contains(&member) {
                self.enroll(member);
            }
        }
    }

    /// The members whose admission was confirmed since the last call.
    pub fn take_committed_admissions(&mut self) -> Vec<CommittedAdmission> {
        std::mem::take(&mut self.committed)
    }

    pub fn client_of_member(&self, member: MemberId) -> Option<ClientId> {
        let conn = self.connection_of(member)?;
        self.connections.client_of(conn).map(ClientId)
    }

    /// Retires the member and keeps it out until `retire_member` clears the ban.
    pub fn deny_member(&mut self, member: MemberId) -> Option<ClientId> {
        let client = self.retire_member(member);
        self.denied.insert(member);
        client
    }

    pub fn retire_member(&mut self, member: MemberId) -> Option<ClientId> {
        self.denied.remove(&member);
        let conn = self.connection_of(member)?;
        self.retire_connection(conn)
    }

    pub fn retire_client(&mut self, client: ClientId) -> bool {
        self.retire_client_with_reason(client, "ConnectionRetired")
    }

    /// Tells the client why it is going (as a reliable failure row), bans the member and
    /// drops the connection.
    pub fn retire_client_with_reason(&mut self, client: ClientId, reason: &str) -> bool {
        let Some((conn, member)) = self.peers.iter().find_map(|(&conn, peer)| {
            (self.connections.resolve(conn, 0) == Ok(client.0)).then_some((conn, peer.member))
        }) else {
            return false;
        };
        let farewell = ServerPacket::Control {
            header: PacketHeader {
                connection: conn,
                sequence: 0,
                ack: 0,
                epoch: self.live_epoch(),
            },
            payload: crate::ReliablePayload {
                ack_through: 0,
                rows: vec![(1, crate::ReliableRow::Failure(reason.to_owned()))],
                dropped_oldest: 0,
            },
        };
        if let Err(error) = self.mailbox.push_control_outbound(member, farewell.to_bytes()) {
            diag::warn!(Net, "retire control: {error}");
        }
        self.denied.insert(member);
        self.retire_connection(conn).is_some()
    }

    pub fn retire_connection(&mut self, conn: ConnectionId) -> Option<ClientId> {
        let client = self.connections.retire(conn).map(ClientId);
        self.peers.remove(&conn);
        client
    }

    /// Starts a new match: every peer goes back to the state of a fresh arrival.
    pub fn reset_match(&mut self) {
        self.committed.clear();
        self.denied.clear();
        for peer in self.peers.values_mut() {
            peer.restart();
        }
        if let Some(lane) = &self.bootstrap {
            lane.set_epoch(0);
        }
    }

    fn enroll(&mut self, member: MemberId) -> ConnectionId {
        if let Some(conn) = self.connection_of(member) {
            return conn;
        }
        let (conn, _) = self.connections.accept_new();
        self.peers.insert(conn, Peer::new(member));
        conn
    }

    fn connection_of(&self, member: MemberId) -> Option<ConnectionId> {
        self.peers
            .iter()
            .find_map(|(&conn, peer)| (peer.member == member).then_some(conn))
    }

    fn member_on(&self, conn: ConnectionId) -> Option<MemberId> {
        self.peers.get(&conn).map(|peer| peer.member)
    }

    /// The match epoch packets are stamped with; zero when there is no bootstrap lane.
    fn live_epoch(&self) -> u32 {
        self.bootstrap.as_ref().map_or(0, |lane| lane.epoch())
    }

    fn send_datagram(&self, member: MemberId, bytes: Vec<u8>) -> Result<(), UdpSendError> {
        self.mailbox
            .push_outbound(member, bytes)
            .map_err(send_refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(byte: u8) -> MemberId {
        MemberId([byte; 16])
    }

    fn hub() -> UdpAuthorityHub {
        let hello = HandshakeHello::current(Default::default());
        UdpAuthorityHub::relay(hello, 1, RelayMailbox::with_default_cap())
    }

    #[test]
    fn enrolling_the_same_member_twice_keeps_one_connection() {
        let mut hub = hub();
        let first = hub.enroll(member(1));
        let again = hub.enroll(member(1));
        assert_eq!(first, again);
        assert_eq!(hub.peers.len(), 1);
    }

    #[test]
    fn reconciling_skips_the_local_member_and_the_banned() {
        let mut hub = hub();
        hub.denied.insert(member(3));
        hub.reconcile_relay_membership(&[member(1), member(2), member(3)], member(1));
        assert!(hub.connection_of(member(1)).is_none());
        assert!(hub.connection_of(member(2)).is_some());
        assert!(hub.connection_of(member(3)).is_none());
    }

    #[test]
    fn every_member_gets_its_own_client_id() {
        let mut hub = hub();
        hub.reconcile_relay_membership(&[member(2), member(3)], member(1));
        let a = hub.client_of_member(member(2)).unwrap();
        let b = hub.client_of_member(member(3)).unwrap();
        assert_ne!(a, b);
        assert!(hub.client_of_member(member(9)).is_none());
    }

    #[test]
    fn retiring_a_member_forgets_the_peer_and_frees_the_client() {
        let mut hub = hub();
        hub.reconcile_relay_membership(&[member(2)], member(1));
        let client = hub.client_of_member(member(2)).unwrap();
        assert_eq!(hub.retire_member(member(2)), Some(client));
        assert!(hub.connection_of(member(2)).is_none());
        assert!(hub.client_of_member(member(2)).is_none());
        assert_eq!(hub.retire_member(member(2)), None);
    }

    #[test]
    fn a_denied_member_is_not_enrolled_again_until_retired() {
        let mut hub = hub();
        hub.reconcile_relay_membership(&[member(2)], member(1));
        hub.deny_member(member(2));
        hub.reconcile_relay_membership(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_none());

        hub.retire_member(member(2));
        hub.reconcile_relay_membership(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_some());
    }

    #[test]
    fn retiring_a_client_sends_it_a_failure_row_and_bans_it() {
        let mut hub = hub();
        hub.reconcile_relay_membership(&[member(2)], member(1));
        let client = hub.client_of_member(member(2)).unwrap();

        assert!(hub.retire_client_with_reason(client, "Kicked"));

        let farewell = hub.mailbox().take_control_outbound();
        assert_eq!(farewell.len(), 1);
        assert_eq!(farewell[0].0, member(2));
        match crate::transport::protocol::decode_server_packet(&farewell[0].1, &hub.limits)
            .expect("the farewell decodes")
        {
            ServerPacket::Control { payload, .. } => match &payload.rows[..] {
                [(1, crate::ReliableRow::Failure(reason))] => assert_eq!(reason, "Kicked"),
                other => panic!("unexpected rows {other:?}"),
            },
            other => panic!("expected a control packet, got {other:?}"),
        }
        hub.reconcile_relay_membership(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_none(), "a retired client stays out");
    }

    #[test]
    fn retiring_an_unknown_client_changes_nothing() {
        let mut hub = hub();
        assert!(!hub.retire_client(ClientId(99)));
        assert!(hub.mailbox().take_control_outbound().is_empty());
    }

    #[test]
    fn resetting_a_match_keeps_the_peers_but_clears_their_progress() {
        let mut hub = hub();
        hub.reconcile_relay_membership(&[member(2)], member(1));
        let conn = hub.connection_of(member(2)).unwrap();
        hub.peers.get_mut(&conn).unwrap().feed.out_seq = 12;
        hub.denied.insert(member(5));

        hub.reset_match();

        assert_eq!(hub.peers[&conn].feed.out_seq, 0);
        assert!(hub.denied.is_empty());
        assert!(hub.take_committed_admissions().is_empty());
    }

    #[test]
    fn without_a_bootstrap_lane_the_epoch_is_zero() {
        assert_eq!(hub().live_epoch(), 0);
    }

    #[test]
    fn a_full_outbound_lane_surfaces_as_a_send_error() {
        let hello = HandshakeHello::current(Default::default());
        let hub = UdpAuthorityHub::relay(hello, 1, RelayMailbox::new(1));
        hub.send_datagram(member(2), vec![1]).unwrap();
        assert!(hub.send_datagram(member(2), vec![2]).is_err());
    }
}
