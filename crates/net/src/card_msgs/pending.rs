//! Player-card messages waiting to go out, each addressed to one client.

use bevy::prelude::*;
use sim::{ClientId, QueuedCallingCardKind, SimWorld};

use super::{HostCardSlotMsg, HostScreenFlash, HostMenuOpenMsg};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Queued {
    CardSlot {
        sender: ClientId,
        slot: i32,
    },
    OpenMenu {
        config_slot_number: i32,
    },
    Splash {
        key: String,
        slot: i32,
        optional: i32,
    },
}

#[derive(Resource, Debug, Default)]
pub struct StagedCardUpdate {
    queue: Vec<(ClientId, Queued)>,
    /// Messages dropped because their frame was never sent.
    pub unclaimed: u64,
}

impl StagedCardUpdate {
    pub fn queue_card_slot(&mut self, addressee: ClientId, sender: ClientId, slot: i32) {
        self.queue.push((addressee, Queued::CardSlot { sender, slot }));
    }

    pub fn queue_menu(&mut self, addressee: ClientId, config_slot_number: i32) {
        self.queue.push((addressee, Queued::OpenMenu { config_slot_number }));
    }

    /// Take everything the simulation has queued since the last call.
    pub fn absorb_sim_queue(&mut self, game: &mut SimWorld) {
        for event in game.take_pending_player_cards() {
            let queued = match event.kind {
                QueuedCallingCardKind::SetSlot { source: from, slot } => Queued::CardSlot { sender: from, slot },
                QueuedCallingCardKind::OpenMenu { config_slot_number } => Queued::OpenMenu { config_slot_number },
                QueuedCallingCardKind::Splash {
                    card_key: key,
                    slot,
                    spare: optional,
                } => Queued::Splash {
                    key,
                    slot,
                    optional,
                },
            };
            self.queue.push((event.recipient, queued));
        }
    }

    /// Remove and return what is queued for `listener`, as card slots, menus and splashes, each in
    /// the order queued. What is queued for others stays.
    pub fn hand_to(
        &mut self,
        listener: ClientId,
    ) -> (Vec<HostCardSlotMsg>, Vec<HostMenuOpenMsg>, Vec<HostScreenFlash>) {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|(recipient, _)| *recipient == listener);
        self.queue = others;

        let (mut card_rows, mut menu_rows, mut splash_rows) = (Vec::new(), Vec::new(), Vec::new());
        for (_, queued) in mine {
            match queued {
                Queued::CardSlot { sender, slot } => card_rows.push(HostCardSlotMsg {
                    client: sender.0,
                    card_slot: slot,
                }),
                Queued::OpenMenu { config_slot_number } => menu_rows.push(HostMenuOpenMsg { config_slot_number }),
                Queued::Splash {
                    key,
                    slot,
                    optional,
                } => splash_rows.push(HostScreenFlash {
                    key,
                    card_slot: slot,
                    optional,
                }),
            }
        }
        (card_rows, menu_rows, splash_rows)
    }

    /// Drop everything still queued, counting it as stranded; returns how many were dropped.
    pub fn flush_orphaned(&mut self) -> usize {
        let dropped = self.queue.len();
        self.queue.clear();
        self.unclaimed = self.unclaimed.saturating_add(dropped as u64);
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued() -> StagedCardUpdate {
        let mut pending = StagedCardUpdate::default();
        pending.queue_card_slot(ClientId(1), ClientId(7), 2);
        pending.queue_menu(ClientId(2), 40);
        pending.queue.push((
            ClientId(1),
            Queued::Splash {
                key: "WIN".into(),
                slot: 3,
                optional: -1,
            },
        ));
        pending.queue_card_slot(ClientId(1), ClientId(8), 4);
        pending
    }

    #[test]
    fn a_client_takes_only_its_own_in_the_order_queued() {
        let mut pending = queued();
        let (slots, menus, splashes) = pending.hand_to(ClientId(1));
        assert_eq!(
            slots,
            [
                HostCardSlotMsg { client: 7, card_slot: 2 },
                HostCardSlotMsg { client: 8, card_slot: 4 }
            ]
        );
        assert!(menus.is_empty());
        assert_eq!(
            splashes,
            [HostScreenFlash {
                key: "WIN".into(),
                card_slot: 3,
                optional: -1
            }]
        );
        // The other client's menu is still waiting, and taking again yields nothing.
        assert_eq!(pending.hand_to(ClientId(1)), (vec![], vec![], vec![]));
        let (_, menus, _) = pending.hand_to(ClientId(2));
        assert_eq!(menus, [HostMenuOpenMsg { config_slot_number: 40 }]);
    }

    #[test]
    fn a_client_with_nothing_queued_takes_nothing_and_changes_nothing() {
        let mut pending = queued();
        assert_eq!(pending.hand_to(ClientId(9)), (vec![], vec![], vec![]));
        assert_eq!(pending.queue.len(), 4);
    }

    #[test]
    fn stranded_messages_are_counted_and_cleared() {
        let mut pending = queued();
        assert_eq!(pending.flush_orphaned(), 4);
        assert_eq!(pending.unclaimed, 4);
        assert_eq!(pending.flush_orphaned(), 0);
        pending.queue_menu(ClientId(0), 1);
        assert_eq!(pending.flush_orphaned(), 1);
        assert_eq!(pending.unclaimed, 5);
    }
}
