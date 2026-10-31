//! The authority side of the relay: one `HostUdpSwitchboard` per hosted match. It keeps a `Peer`
//! for every remote member, reads their packets (`ingress`) and answers with snapshots
//! (`fanout`). The packets travel through a `ForwardingInbox`, never over a socket of its own.

mod fanout;
mod ingress;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::prelude::Resource;
use master_protocol::RoomOccupantId;
use sim::ClientId;

use super::entry_gate::FinalizedEntry;
use super::mailbox::{ForwardingInbox, send_refused};
use super::peer::Peer;
use crate::net_links::match_handoff::TransferChannelState;
use crate::net_links::datagrams::{
    LinkNumber, LinkRegistry, JoinGreeting, DatagramPreamble, NegotiatedCaps, HostToPlayerDatagram,
};
use crate::net_links::send_fault::UdpTransmitFailure;

#[derive(Resource)]
pub struct HostUdpSwitchboard {
    mailbox: ForwardingInbox,
    pub hello: JoinGreeting,
    pub negotiated: NegotiatedCaps,
    pub links: LinkRegistry,
    remotes: HashMap<LinkNumber, Peer>,

    /// Present when joiners are admitted through bootstrap offers.
    offer_lane: Option<Arc<TransferChannelState>>,
    finalized: Vec<FinalizedEntry>,

    /// Members that must not be enrolled again until they are retired for good.
    banned_members: HashSet<RoomOccupantId>,
}

impl HostUdpSwitchboard {
    pub fn relay(hello: JoinGreeting, lead_receiver: u32, mailbox: ForwardingInbox) -> Self {
        let negotiated = hello.limits;
        Self {
            mailbox,
            hello,
            negotiated,
            links: LinkRegistry::starting_at(lead_receiver),
            remotes: HashMap::new(),
            offer_lane: None,
            finalized: Vec::new(),
            banned_members: HashSet::new(),
        }
    }

    pub fn mailbox(&self) -> ForwardingInbox {
        self.mailbox.clone()
    }

    pub fn bind_offer_lane(&mut self, chan: Arc<TransferChannelState>) {
        self.offer_lane = Some(chan);
    }

    pub fn sync_room_members(&mut self, members: &[RoomOccupantId], local: RoomOccupantId) {
        for &member in members {
            if member != local && !self.banned_members.contains(&member) {
                self.enroll(member);
            }
        }
    }

    /// The members whose admission was confirmed since the last call.
    pub fn take_committed_admissions(&mut self) -> Vec<FinalizedEntry> {
        std::mem::take(&mut self.finalized)
    }

    pub fn seat_for_occupant(&self, member: RoomOccupantId) -> Option<ClientId> {
        let conn = self.connection_of(member)?;
        self.links.receiver_for(conn).map(ClientId)
    }

    /// Retires the member and keeps it out until `release_occupant` clears the ban.
    pub fn refuse_occupant(&mut self, member: RoomOccupantId) -> Option<ClientId> {
        let client = self.release_occupant(member);
        self.banned_members.insert(member);
        client
    }

    pub fn release_occupant(&mut self, member: RoomOccupantId) -> Option<ClientId> {
        self.banned_members.remove(&member);
        let conn = self.connection_of(member)?;
        self.drop_link_entry(conn)
    }

    pub fn drop_player_record(&mut self, seat: ClientId) -> bool {
        self.drop_seat(seat, "ConnectionRetired")
    }

    /// Tells the client why it is going (as a reliable failure row), bans the member and
    /// drops the connection.
    pub fn drop_seat(&mut self, seat: ClientId, cause_note: &str) -> bool {
        let Some((link, member)) = self.remotes.iter().find_map(|(&link, peer)| {
            (self.links.trusted_slot(link, 0) == Ok(seat.0)).then_some((link, peer.member))
        }) else {
            return false;
        };
        let farewell = HostToPlayerDatagram::OwedBatch {
            header: DatagramPreamble {
                connection: link,
                packet_no: 0,
                confirm_receipt: 0,
                round_era: self.running_generation(),
            },
            payload: crate::GuaranteedEnvelope {
                confirmed_up_to: 0,
                entries: vec![(1, crate::GuaranteedEntry::RefusalText(cause_note.to_owned()))],
                evicted_stale_count: 0,
            },
        };
        if let Err(error) = self.mailbox.enqueue_control_outgoing(member, farewell.wire_bytes()) {
            diag::warn!(WireLink, "farewell control lane refused: {error}");
        }
        self.banned_members.insert(member);
        self.drop_link_entry(link).is_some()
    }

    pub fn drop_link_entry(&mut self, link: LinkNumber) -> Option<ClientId> {
        let freed = self.links.forget_link(link).map(ClientId);
        self.remotes.remove(&link);
        freed
    }

    /// Starts a new match: every peer goes back to the state of a fresh arrival.
    pub fn clear_for_new_game(&mut self) {
        self.finalized.clear();
        self.banned_members.clear();
        for peer in self.remotes.values_mut() {
            peer.restart();
        }
        if let Some(lane) = &self.offer_lane {
            lane.assign_generation(0);
        }
    }

    fn enroll(&mut self, member: RoomOccupantId) -> LinkNumber {
        if let Some(conn) = self.connection_of(member) {
            return conn;
        }
        let (conn, _) = self.links.admit_arrivals();
        self.remotes.insert(conn, Peer::new(member));
        conn
    }

    fn connection_of(&self, member: RoomOccupantId) -> Option<LinkNumber> {
        self.remotes
            .iter()
            .find_map(|(&conn, peer)| (peer.member == member).then_some(conn))
    }

    fn member_on(&self, conn: LinkNumber) -> Option<RoomOccupantId> {
        self.remotes.get(&conn).map(|peer| peer.member)
    }

    /// The match epoch packets are stamped with; zero when there is no bootstrap lane.
    fn running_generation(&self) -> u32 {
        self.offer_lane.as_ref().map_or(0, |lane| lane.era())
    }

    fn send_datagram(&self, member: RoomOccupantId, payload: Vec<u8>) -> Result<(), UdpTransmitFailure> {
        self.mailbox
            .queue_departure(member, payload)
            .map_err(send_refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(byte: u8) -> RoomOccupantId {
        RoomOccupantId([byte; 16])
    }

    fn hub() -> HostUdpSwitchboard {
        let hello = JoinGreeting::current(Default::default());
        HostUdpSwitchboard::relay(hello, 1, ForwardingInbox::standard_capacity())
    }

    #[test]
    fn enrolling_the_same_member_twice_keeps_one_connection() {
        let mut hub = hub();
        let first = hub.enroll(member(1));
        let again = hub.enroll(member(1));
        assert_eq!(first, again);
        assert_eq!(hub.remotes.len(), 1);
    }

    #[test]
    fn reconciling_skips_the_local_member_and_the_banned() {
        let mut hub = hub();
        hub.banned_members.insert(member(3));
        hub.sync_room_members(&[member(1), member(2), member(3)], member(1));
        assert!(hub.connection_of(member(1)).is_none());
        assert!(hub.connection_of(member(2)).is_some());
        assert!(hub.connection_of(member(3)).is_none());
    }

    #[test]
    fn every_member_gets_its_own_client_id() {
        let mut hub = hub();
        hub.sync_room_members(&[member(2), member(3)], member(1));
        let a = hub.seat_for_occupant(member(2)).unwrap();
        let b = hub.seat_for_occupant(member(3)).unwrap();
        assert_ne!(a, b);
        assert!(hub.seat_for_occupant(member(9)).is_none());
    }

    #[test]
    fn retiring_a_member_forgets_the_peer_and_frees_the_client() {
        let mut hub = hub();
        hub.sync_room_members(&[member(2)], member(1));
        let client = hub.seat_for_occupant(member(2)).unwrap();
        assert_eq!(hub.release_occupant(member(2)), Some(client));
        assert!(hub.connection_of(member(2)).is_none());
        assert!(hub.seat_for_occupant(member(2)).is_none());
        assert_eq!(hub.release_occupant(member(2)), None);
    }

    #[test]
    fn a_denied_member_is_not_enrolled_again_until_retired() {
        let mut hub = hub();
        hub.sync_room_members(&[member(2)], member(1));
        hub.refuse_occupant(member(2));
        hub.sync_room_members(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_none());

        hub.release_occupant(member(2));
        hub.sync_room_members(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_some());
    }

    #[test]
    fn retiring_a_client_sends_it_a_failure_row_and_bans_it() {
        let mut hub = hub();
        hub.sync_room_members(&[member(2)], member(1));
        let client = hub.seat_for_occupant(member(2)).unwrap();

        assert!(hub.drop_seat(client, "Kicked"));

        let farewell = hub.mailbox().take_control_outbound();
        assert_eq!(farewell.len(), 1);
        assert_eq!(farewell[0].0, member(2));
        match crate::net_links::datagrams::parse_host_datagram(&farewell[0].1, &hub.negotiated)
            .expect("the farewell decodes")
        {
            HostToPlayerDatagram::OwedBatch { payload: envelope, .. } => match &envelope.entries[..] {
                [(1, crate::GuaranteedEntry::RefusalText(reason))] => assert_eq!(reason, "Kicked"),
                other => panic!("unexpected rows {other:?}"),
            },
            other => panic!("expected a control packet, got {other:?}"),
        }
        hub.sync_room_members(&[member(2)], member(1));
        assert!(hub.connection_of(member(2)).is_none(), "a retired client stays out");
    }

    #[test]
    fn retiring_an_unknown_client_changes_nothing() {
        let mut hub = hub();
        assert!(!hub.drop_player_record(ClientId(99)));
        assert!(hub.mailbox().take_control_outbound().is_empty());
    }

    #[test]
    fn resetting_a_match_keeps_the_peers_but_clears_their_progress() {
        let mut hub = hub();
        hub.sync_room_members(&[member(2)], member(1));
        let conn = hub.connection_of(member(2)).unwrap();
        hub.remotes.get_mut(&conn).unwrap().feed.next_outgoing_number = 12;
        hub.banned_members.insert(member(5));

        hub.clear_for_new_game();

        assert_eq!(hub.remotes[&conn].feed.next_outgoing_number, 0);
        assert!(hub.banned_members.is_empty());
        assert!(hub.take_committed_admissions().is_empty());
    }

    #[test]
    fn without_a_bootstrap_lane_the_epoch_is_zero() {
        assert_eq!(hub().running_generation(), 0);
    }

    #[test]
    fn a_full_outbound_lane_surfaces_as_a_send_error() {
        let hello = JoinGreeting::current(Default::default());
        let hub = HostUdpSwitchboard::relay(hello, 1, ForwardingInbox::new(1));
        hub.send_datagram(member(2), vec![1]).unwrap();
        assert!(hub.send_datagram(member(2), vec![2]).is_err());
    }
}
