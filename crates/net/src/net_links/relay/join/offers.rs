//! Bootstrap offers: the first snapshot a joiner gets arrives through the bootstrap lane, and
//! the joiner answers through it once the snapshot is applied.

use sim::ClientId;

use super::PlayerUdpSession;
use super::receive::Harvest;
use crate::net_links::match_handoff::{
    InitialStateTransferMsg, parse_setup_message, pack_bootstrap, same_generation,
};

impl PlayerUdpSession {
    /// Reads the offers waiting on the bootstrap lane (plus any held back earlier) and applies
    /// the snapshot each one carries.
    pub(super) fn drain_bootstrap_offers(&mut self, harvest: &mut Harvest) -> Result<(), String> {
        let Some(chan) = self.offer_lane.clone() else {
            return Ok(());
        };
        let current_era = chan.era();
        let mut waiting = std::mem::take(&mut self.round.offers_waiting);
        waiting.extend(chan.take_from_worker().into_iter().map(|arrival| arrival.coded_bytes));

        for raw in waiting {
            match parse_setup_message(&raw) {
                Ok(Some(offer @ InitialStateTransferMsg::Offer { .. })) => {
                    self.take_offer(offer, current_era, harvest);
                }
                Ok(Some(InitialStateTransferMsg::TakenIntoEffect { .. })) => {
                    diag::warn!(WireLink, "joiner dropped an applied-notice that arrived on its offer lane");
                }
                Ok(None) => {
                    diag::warn!(WireLink, "joiner dropped bytes on its offer lane that are not a setup message");
                }
                Err(error) => {
                    diag::warn!(WireLink, "joiner dropped an unreadable setup offer: {error}");
                }
            }
        }
        Ok(())
    }

    fn take_offer(&mut self, offer: InitialStateTransferMsg, current_era: u32, harvest: &mut Harvest) {
        let InitialStateTransferMsg::Offer {
            era,
            handoff_id: setup_id,
            step_marker_b: marker_b,
            frame_number_on_wire: snap_no,
            link: link_id,
            chunk: wire,
        } = offer
        else {
            return;
        };
        if !same_generation(era, current_era) {
            diag::warn!(
                WireLink,
                "joiner skipped a setup offer from era {era} (current {current_era})"
            );
            return;
        }
        diag::info!(
            WireLink,
            "bootstrap-io session=join event=bootstrap offer era={era} current={current_era} setup_id={setup_id} snap_no={snap_no} len={}",
            wire.len()
        );
        self.connection.get_or_insert(link_id);

        let confirm = InitialStateTransferMsg::TakenIntoEffect {
            era,
            handoff_id: setup_id,
            step_marker_b: marker_b,
            frame_number_on_wire: snap_no,
            link: link_id,
        };
        match self.apply_bytes(&wire, harvest) {
            Ok(true) => {
                self.round.offer_applied = Some((setup_id, snap_no));
                self.round.confirms_due.push(confirm);
            }
            Ok(false) => {
                // Offers are repeated until confirmed; a repeat of one already applied is
                // confirmed again in case the first confirmation was lost.
                if self.round.offer_applied == Some((setup_id, snap_no)) {
                    self.round.confirms_due.push(confirm);
                }
            }
            Err(error) => {
                diag::warn!(
                    WireLink,
                    "setup offer carried a link id but its snapshot was rejected: {error}"
                );
            }
        }
    }

    /// Once the game thread has entered the bootstrap this client applied, the client id it
    /// was given there replaces the one from the handshake.
    pub(super) fn take_over_joined_receiver(&mut self) {
        let Some(chan) = &self.offer_lane else {
            return;
        };
        let entered = chan.joined_setup_stage();
        if entered == 0 || self.round.bootstrap_applied != Some(entered) {
            return;
        }
        let given = ClientId(chan.joined_receiver_role());
        if given.0 != 0 {
            self.granted_player_slot = Some(given);
        }
    }

    /// Hands the applied confirmations to the bootstrap lane, in order. The first one the lane
    /// refuses stays queued, with everything after it, for the next call.
    pub fn emit_installed_notices(&mut self) {
        let Some(chan) = self.offer_lane.clone() else {
            return;
        };
        let queued = std::mem::take(&mut self.round.confirms_due);
        let mut sent = 0;
        for message in &queued {
            if let InitialStateTransferMsg::TakenIntoEffect { handoff_id: setup_id, .. } = message {
                self.round.bootstrap_applied = Some(*setup_id);
            }
            let outcome = pack_bootstrap(message)
                .map_err(|error| format!("applied-notice could not be encoded: {error}"))
                .and_then(|encoded| {
                    chan.hand_to_worker(None, encoded)
                        .map_err(|error| format!("applied-notice could not be queued: {error}"))
                });
            if let Err(why) = outcome {
                diag::warn!(WireLink, "{why}");
                break;
            }
            sent += 1;
        }
        self.round.confirms_due = queued.into_iter().skip(sent).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net_links::match_handoff::TransferChannelState;
    use crate::net_links::datagrams::{LinkNumber, JoinGreeting};
    use crate::net_links::relay::mailbox::ForwardingInbox;

    fn link_with_lane(epoch: u32) -> (PlayerUdpSession, std::sync::Arc<TransferChannelState>) {
        let mut link = PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        );
        let lane = TransferChannelState::new();
        lane.assign_generation(epoch);
        link.bind_offer_lane(lane.clone());
        (link, lane)
    }

    fn offer(era: u32, setup_id: u32, packet: Vec<u8>) -> Vec<u8> {
        pack_bootstrap(&InitialStateTransferMsg::Offer {
            era,
            handoff_id: setup_id,
            step_marker_b: 40,
            frame_number_on_wire: 1,
            link: LinkNumber(6),
            chunk: packet,
        })
        .unwrap()
    }

    fn applied(setup_id: u32) -> InitialStateTransferMsg {
        InitialStateTransferMsg::TakenIntoEffect {
            era: 3,
            handoff_id: setup_id,
            step_marker_b: 40,
            frame_number_on_wire: 1,
            link: LinkNumber(6),
        }
    }

    #[test]
    fn without_a_lane_there_is_nothing_to_drain() {
        let mut link = PlayerUdpSession::relay(
            JoinGreeting::current(Default::default()),
            ForwardingInbox::standard_capacity(),
        );
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        link.emit_installed_notices();
        assert!(link.round.confirms_due.is_empty());
    }

    #[test]
    fn an_offer_for_another_epoch_is_ignored_and_names_no_connection() {
        let (mut link, lane) = link_with_lane(3);
        lane.enqueue_from_helper(None, offer(4, 1, vec![1])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert!(link.connection.is_none());
        assert!(link.round.confirms_due.is_empty());
    }

    #[test]
    fn an_offer_names_the_connection_even_when_its_snapshot_does_not_decode() {
        let (mut link, lane) = link_with_lane(3);
        lane.enqueue_from_helper(None, offer(3, 1, vec![0xee, 0xee])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert_eq!(link.connection, Some(LinkNumber(6)));
        assert!(link.round.confirms_due.is_empty(), "nothing was applied");
    }

    #[test]
    fn an_offer_never_replaces_a_connection_already_named() {
        let (mut link, lane) = link_with_lane(3);
        link.note_admission(LinkNumber(2), ClientId(1));
        lane.enqueue_from_helper(None, offer(3, 1, vec![1])).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert_eq!(link.connection, Some(LinkNumber(2)));
    }

    #[test]
    fn junk_on_the_offer_lane_is_ignored() {
        let (mut link, lane) = link_with_lane(3);
        lane.enqueue_from_helper(None, vec![0, 1, 2]).unwrap();
        let confirm = pack_bootstrap(&applied(1)).unwrap();
        lane.enqueue_from_helper(None, confirm).unwrap();
        link.drain_bootstrap_offers(&mut Harvest::default()).unwrap();
        assert!(link.connection.is_none());
        assert!(link.round.confirms_due.is_empty());
    }

    #[test]
    fn flushing_hands_the_confirmations_to_the_lane_and_notes_the_bootstrap() {
        let (mut link, lane) = link_with_lane(3);
        link.round.confirms_due.push(applied(5));
        link.round.confirms_due.push(applied(6));
        link.emit_installed_notices();
        assert!(link.round.confirms_due.is_empty());
        assert_eq!(link.entry_setup_token(), Some(6));
        assert_eq!(lane.take_to_worker().len(), 2);
    }

    #[test]
    fn entering_the_applied_bootstrap_adopts_the_client_id_from_it() {
        let (mut link, lane) = link_with_lane(3);
        link.note_admission(LinkNumber(6), ClientId(1));
        link.mark_setup_applied(5);

        lane.record_entry(4, 9);
        link.take_over_joined_receiver();
        assert_eq!(link.granted_player_slot, Some(ClientId(1)), "a different bootstrap");

        lane.record_entry(5, 9);
        link.take_over_joined_receiver();
        assert_eq!(link.granted_player_slot, Some(ClientId(9)));
        assert!(link.is_inside_match());
    }

    #[test]
    fn a_zero_client_id_is_never_adopted() {
        let (mut link, lane) = link_with_lane(3);
        link.note_admission(LinkNumber(6), ClientId(1));
        link.mark_setup_applied(5);
        lane.record_entry(5, 0);
        link.take_over_joined_receiver();
        assert_eq!(link.granted_player_slot, Some(ClientId(1)));
    }
}
