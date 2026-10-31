use std::io;
use std::sync::{Arc, Mutex};

use master_protocol::RoomOccupantId;

use crate::net_links::send_fault::UdpTransmitFailure;

/// How many parcels each lane of a default mailbox holds before it refuses more.
const DEFAULT_LANE_CAPACITY: usize = 64;

const LANE_FULL: &str = "relay mailbox full";

/// One bounded, shared, first-in first-out lane.
#[derive(Clone, Debug)]
struct Chute {
    queued: Arc<Mutex<Vec<(RoomOccupantId, Vec<u8>)>>>,
    capacity: usize,
}

impl Chute {
    fn new(capacity: usize) -> Self {
        Self {
            queued: Arc::new(Mutex::new(Vec::new())),
            capacity,
        }
    }

    fn hold(&self, member: RoomOccupantId, raw: Vec<u8>) -> Result<(), &'static str> {
        let mut parcels = self.queued.lock().expect("relay mailbox poisoned");
        if parcels.len() >= self.capacity {
            return Err(LANE_FULL);
        }
        parcels.push((member, raw));
        Ok(())
    }

    fn empty_out(&self) -> Vec<(RoomOccupantId, Vec<u8>)> {
        std::mem::take(&mut *self.queued.lock().expect("relay mailbox poisoned"))
    }

    fn has_room(&self) -> bool {
        self.queued.lock().expect("relay mailbox poisoned").len() < self.capacity
    }
}

/// Where the game thread and the network worker leave packets for each other.
///
/// There are two directions, each with a datagram lane and a control lane. The datagram lane
/// may lose packets under pressure; the control lane is ordered and reliable, so its reader
/// waits for room instead (see `wait_control_inbound_capacity`).
#[derive(Clone, Debug)]
pub struct ForwardingInbox {
    inbound: Chute,
    outbound: Chute,
    control_inbound: Chute,
    control_outbound: Chute,
    control_inbound_drained: Arc<tokio::sync::Notify>,
}

impl ForwardingInbox {
    pub fn new(capacity: usize) -> Self {
        Self {
            inbound: Chute::new(capacity),
            outbound: Chute::new(capacity),
            control_inbound: Chute::new(capacity),
            control_outbound: Chute::new(capacity),
            control_inbound_drained: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn standard_capacity() -> Self {
        Self::new(DEFAULT_LANE_CAPACITY)
    }

    pub fn take_arrival(&self, member: RoomOccupantId, raw: Vec<u8>) -> Result<(), &'static str> {
        self.inbound.hold(member, raw)
    }

    pub fn take_inbound(&self) -> Vec<(RoomOccupantId, Vec<u8>)> {
        self.inbound.empty_out()
    }

    pub fn queue_departure(&self, member: RoomOccupantId, raw: Vec<u8>) -> Result<(), &'static str> {
        self.outbound.hold(member, raw)
    }

    pub fn take_outbound(&self) -> Vec<(RoomOccupantId, Vec<u8>)> {
        self.outbound.empty_out()
    }

    pub fn enqueue_control_incoming(
        &self,
        member: RoomOccupantId,
        raw: Vec<u8>,
    ) -> Result<(), &'static str> {
        self.control_inbound.hold(member, raw)
    }

    /// Emptying the control inbox also wakes the control reader if it is waiting for room.
    pub fn take_control_inbound(&self) -> Vec<(RoomOccupantId, Vec<u8>)> {
        let parcels = self.control_inbound.empty_out();
        self.control_inbound_drained.notify_one();
        parcels
    }

    pub fn enqueue_control_outgoing(
        &self,
        member: RoomOccupantId,
        raw: Vec<u8>,
    ) -> Result<(), &'static str> {
        self.control_outbound.hold(member, raw)
    }

    pub fn take_control_outbound(&self) -> Vec<(RoomOccupantId, Vec<u8>)> {
        self.control_outbound.empty_out()
    }

    /// Resolves once the control inbox has room for one more parcel.
    ///
    /// The control reader is the only producer. It waits here from inside its own select loop,
    /// so cancellation and local commands keep running while the game thread catches up.
    pub(crate) async fn wait_control_inbound_capacity(&self) {
        loop {
            // Register for the wake-up before looking, so a drain between the look and the
            // wait cannot be missed.
            let drained = self.control_inbound_drained.notified();
            if self.control_inbound.has_room() {
                return;
            }
            drained.await;
        }
    }
}

/// The error a full mailbox turns into when a send was supposed to go through it.
pub(super) fn send_refused(why: &'static str) -> UdpTransmitFailure {
    UdpTransmitFailure::OsSend(io::Error::new(io::ErrorKind::WouldBlock, why))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn member(byte: u8) -> RoomOccupantId {
        RoomOccupantId([byte; 16])
    }

    #[test]
    fn parcels_come_back_in_the_order_they_went_in() {
        let mail = ForwardingInbox::standard_capacity();
        mail.take_arrival(member(1), vec![10]).unwrap();
        mail.take_arrival(member(2), vec![20]).unwrap();
        assert_eq!(
            mail.take_inbound(),
            vec![(member(1), vec![10]), (member(2), vec![20])]
        );
    }

    #[test]
    fn taking_empties_the_lane() {
        let mail = ForwardingInbox::standard_capacity();
        mail.queue_departure(member(1), vec![1]).unwrap();
        assert_eq!(mail.take_outbound().len(), 1);
        assert!(mail.take_outbound().is_empty());
    }

    #[test]
    fn a_full_lane_refuses_and_says_so() {
        let mail = ForwardingInbox::new(2);
        mail.take_arrival(member(1), vec![]).unwrap();
        mail.take_arrival(member(1), vec![]).unwrap();
        assert_eq!(
            mail.take_arrival(member(1), vec![]),
            Err("relay mailbox full")
        );
        mail.take_inbound();
        assert_eq!(mail.take_arrival(member(1), vec![]), Ok(()));
    }

    #[test]
    fn each_lane_fills_on_its_own() {
        let mail = ForwardingInbox::new(1);
        mail.take_arrival(member(1), vec![]).unwrap();
        assert!(mail.queue_departure(member(1), vec![]).is_ok());
        assert!(mail.enqueue_control_incoming(member(1), vec![]).is_ok());
        assert!(mail.enqueue_control_outgoing(member(1), vec![]).is_ok());
        assert!(mail.take_arrival(member(1), vec![]).is_err());
        assert!(mail.queue_departure(member(1), vec![]).is_err());
    }

    #[test]
    fn the_lanes_hold_different_parcels() {
        let mail = ForwardingInbox::standard_capacity();
        mail.take_arrival(member(1), vec![1]).unwrap();
        mail.enqueue_control_incoming(member(1), vec![2]).unwrap();
        mail.queue_departure(member(1), vec![3]).unwrap();
        mail.enqueue_control_outgoing(member(1), vec![4]).unwrap();
        assert_eq!(mail.take_control_outbound()[0].1, vec![4]);
        assert_eq!(mail.take_outbound()[0].1, vec![3]);
        assert_eq!(mail.take_control_inbound()[0].1, vec![2]);
        assert_eq!(mail.take_inbound()[0].1, vec![1]);
    }

    #[test]
    fn a_clone_is_another_handle_on_the_same_lanes() {
        let mail = ForwardingInbox::standard_capacity();
        let other = mail.clone();
        mail.take_arrival(member(7), vec![7]).unwrap();
        assert_eq!(other.take_inbound(), vec![(member(7), vec![7])]);
    }

    #[test]
    fn a_refused_send_looks_like_a_would_block_error() {
        match send_refused("relay mailbox full") {
            UdpTransmitFailure::OsSend(error) => {
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                assert_eq!(error.to_string(), "relay mailbox full");
            }
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn waiting_with_room_to_spare_returns_at_once() {
        let mail = ForwardingInbox::new(2);
        mail.enqueue_control_incoming(member(1), vec![]).unwrap();
        tokio::time::timeout(Duration::from_secs(5), mail.wait_control_inbound_capacity())
            .await
            .expect("there was room, so the wait should end");
    }

    #[tokio::test]
    async fn a_full_control_inbox_holds_the_reader_until_it_is_drained() {
        let mail = ForwardingInbox::new(1);
        mail.enqueue_control_incoming(member(1), vec![]).unwrap();
        let waiter = {
            let mail = mail.clone();
            tokio::spawn(async move { mail.wait_control_inbound_capacity().await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        mail.take_control_inbound();
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("draining should wake the reader")
            .expect("the waiting task should not panic");
    }
}
