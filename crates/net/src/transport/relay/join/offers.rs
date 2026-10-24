//! Bootstrap offers: the first snapshot a joiner gets arrives through the bootstrap lane, and
//! the joiner answers through it once the snapshot is applied.

use sim::ClientId;

use super::UdpClientLink;
use super::receive::Harvest;
use crate::transport::bootstrap::{
    BootstrapMessage, decode_bootstrap, encode_bootstrap, epoch_applies,
};

impl UdpClientLink {
    /// Reads the offers waiting on the bootstrap lane (plus any held back earlier) and applies
    /// the snapshot each one carries.
    pub(super) fn drain_bootstrap_offers(&mut self, harvest: &mut Harvest) -> Result<(), String> {
        let Some(lane) = self.bootstrap.clone() else {
            return Ok(());
        };
        let live = lane.epoch();
        let mut waiting = std::mem::take(&mut self.held_offers);
        waiting.extend(lane.take_from_worker().into_iter().map(|ingress| ingress.bytes));

        for bytes in waiting {
            match decode_bootstrap(&bytes) {
                Ok(Some(offer @ BootstrapMessage::Offer { .. })) => {
                    self.take_offer(offer, live, harvest);
                }
                Ok(Some(BootstrapMessage::Applied { .. })) => {
                    diag::warn!(Net, "client ignored a bootstrap applied on the offer lane");
                }
                Ok(None) => {
                    diag::warn!(Net, "client ignored non-bootstrap bytes on the offer lane");
                }
                Err(error) => {
                    diag::warn!(Net, "client ignored malformed bootstrap offer: {error}");
                }
            }
        }
        Ok(())
    }

    fn take_offer(&mut self, offer: BootstrapMessage, live: u32, harvest: &mut Harvest) {
        let BootstrapMessage::Offer {
            epoch,
            bootstrap_id,
            tick_b,
            snapshot_seq,
            connection,
            packet,
        } = offer
        else {
            return;
        };
        if !epoch_applies(epoch, live) {
            diag::warn!(
                Net,
                "client ignored bootstrap offer for epoch {epoch} (live {live})"
            );
            return;
        }
        diag::info!(
            Net,
            "bootstrap-io session=join event=bootstrap offer epoch={epoch} live={live} bootstrap_id={bootstrap_id} snapshot_seq={snapshot_seq} bytes={}",
            packet.len()
        );
        self.connection.get_or_insert(connection);

        let confirm = BootstrapMessage::Applied {
            epoch,
            bootstrap_id,
            tick_b,
            snapshot_seq,
            connection,
        };
        match self.apply_bytes(&packet, harvest) {
            Ok(true) => {
                self.last_applied_offer = Some((bootstrap_id, snapshot_seq));
                self.unsent_applied.push(confirm);
            }
            Ok(false) => {
                // Offers are repeated until confirmed; a repeat of one already applied is
                // confirmed again in case the first confirmation was lost.
                if self.last_applied_offer == Some((bootstrap_id, snapshot_seq)) {
                    self.unsent_applied.push(confirm);
                }
            }
            Err(error) => {
                diag::warn!(
                    Net,
                    "bootstrap offer named the connection; snapshot not applied: {error}"
                );
            }
        }
    }

    /// Once the game thread has entered the bootstrap this client applied, the client id it
    /// was given there replaces the one from the handshake.
    pub(super) fn adopt_entered_client(&mut self) {
        let Some(lane) = &self.bootstrap else {
            return;
        };
        let entered = lane.entered_bootstrap();
        if entered == 0 || self.applied_bootstrap_id != Some(entered) {
            return;
        }
        let client = ClientId(lane.entered_client());
        if client.0 != 0 {
            self.assigned_client = Some(client);
        }
    }

    /// Hands the applied confirmations to the bootstrap lane, in order. The first one the lane
    /// refuses stays queued, with everything after it, for the next call.
    pub fn flush_applied_after_adopt(&mut self) {
        let Some(lane) = self.bootstrap.clone() else {
            return;
        };
        let queued = std::mem::take(&mut self.unsent_applied);
        let mut sent = 0;
        for message in &queued {
            if let BootstrapMessage::Applied { bootstrap_id, .. } = message {
                self.applied_bootstrap_id = Some(*bootstrap_id);
            }
            let outcome = encode_bootstrap(message)
                .map_err(|error| format!("bootstrap applied encode failed: {error}"))
                .and_then(|bytes| {
                    lane.push_to_worker(None, bytes)
                        .map_err(|error| format!("bootstrap applied not queued: {error}"))
                });
            if let Err(why) = outcome {
                diag::warn!(Net, "{why}");
                break;
            }
            sent += 1;
        }
        self.unsent_applied = queued.into_iter().skip(sent).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::bootstrap::BootstrapLane;
    use crate::transport::protocol::{ConnectionId, HandshakeHello};
    use crate::transport::relay::mailbox::RelayMailbox;

    fn link_with_lane(epoch: u32) -> (UdpClientLink, std::sync::Arc<BootstrapLane>) {
        let mut link = UdpClientLink::relay(
            HandshakeHello::current(Default::default()),
            RelayMailbox::with_default_cap(),
        );
        let lane = BootstrapLane::new();
        lane.set_epoch(epoch);
        link.attach_bootstrap(lane.clone());
        (link, lane)
    }

    fn offer(epoch: u32, bootstrap_id: u32, packet: Vec<u8>) -> Vec<u8> {
        encode_bootstrap(&BootstrapMessage::Offer {
            epoch,
            bootstrap_id,
            tick_b: 40,
            snapshot_seq: 1,
            connection: ConnectionId(6),
            packet,
        })
        .unwrap()
    }

    fn applied(bootstrap_id: u32) -> BootstrapMessage {
        BootstrapMessage::Applied {
            epoch: 3,
            bootstrap_id,
            tick_b: 40,
            snapshot_seq: 1,
            connection: ConnectionId(6),
        }
    }

    #[test]
    fn without_a_lane_there_is_nothing_to_drain() {
        let mut link = UdpClientLink::relay(
            HandshakeHello::current(Default::default()),
            RelayMailbox::with_default_cap(),
        );
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        link.flush_applied_after_adopt();
        assert!(link.unsent_applied.is_empty());
    }

    #[test]
    fn an_offer_for_another_epoch_is_ignored_and_names_no_connection() {
        let (mut link, lane) = link_with_lane(3);
        lane.push_from_worker(None, offer(4, 1, vec![1])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert!(link.connection.is_none());
        assert!(link.unsent_applied.is_empty());
    }

    #[test]
    fn an_offer_names_the_connection_even_when_its_snapshot_does_not_decode() {
        let (mut link, lane) = link_with_lane(3);
        lane.push_from_worker(None, offer(3, 1, vec![0xee, 0xee])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert_eq!(link.connection, Some(ConnectionId(6)));
        assert!(link.unsent_applied.is_empty(), "nothing was applied");
    }

    #[test]
    fn an_offer_never_replaces_a_connection_already_named() {
        let (mut link, lane) = link_with_lane(3);
        link.note_accept(ConnectionId(2), ClientId(1));
        lane.push_from_worker(None, offer(3, 1, vec![1])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert_eq!(link.connection, Some(ConnectionId(2)));
    }

    #[test]
    fn junk_on_the_offer_lane_is_ignored() {
        let (mut link, lane) = link_with_lane(3);
        lane.push_from_worker(None, vec![0, 1, 2]).unwrap();
        let confirm = encode_bootstrap(&applied(1)).unwrap();
        lane.push_from_worker(None, confirm).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert!(link.connection.is_none());
        assert!(link.unsent_applied.is_empty());
    }

    #[test]
    fn flushing_hands_the_confirmations_to_the_lane_and_notes_the_bootstrap() {
        let (mut link, lane) = link_with_lane(3);
        link.unsent_applied.push(applied(5));
        link.unsent_applied.push(applied(6));
        link.flush_applied_after_adopt();
        assert!(link.unsent_applied.is_empty());
        assert_eq!(link.admission_bootstrap_id(), Some(6));
        assert_eq!(lane.take_to_worker().len(), 2);
    }

    #[test]
    fn entering_the_applied_bootstrap_adopts_the_client_id_from_it() {
        let (mut link, lane) = link_with_lane(3);
        link.note_accept(ConnectionId(6), ClientId(1));
        link.note_applied_bootstrap(5);

        lane.note_entered(4, 9);
        link.adopt_entered_client();
        assert_eq!(link.assigned_client, Some(ClientId(1)), "a different bootstrap");

        lane.note_entered(5, 9);
        link.adopt_entered_client();
        assert_eq!(link.assigned_client, Some(ClientId(9)));
        assert!(link.has_entered_match());
    }

    #[test]
    fn a_zero_client_id_is_never_adopted() {
        let (mut link, lane) = link_with_lane(3);
        link.note_accept(ConnectionId(6), ClientId(1));
        link.note_applied_bootstrap(5);
        lane.note_entered(5, 0);
        link.adopt_entered_client();
        assert_eq!(link.assigned_client, Some(ClientId(1)));
    }
}
