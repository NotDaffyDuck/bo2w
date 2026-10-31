//! Who a datagram claims to come from: connection ids, the header every packet carries, and
//! the authority's table of which connection belongs to which client.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LinkNumber(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DatagramPreamble {
    pub connection: LinkNumber,
    pub packet_no: u32,

    /// The newest sequence the sender has seen from the other side.
    pub confirm_receipt: u32,

    /// The match the sender believes it is in.
    pub round_era: u32,
}

impl DatagramPreamble {
    pub fn valid_for_generation(self, current_gen: u32) -> bool {
        self.round_era == current_gen
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIdentityFault {
    NoSuchLink,
}

impl core::fmt::Display for LinkIdentityFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSuchLink => write!(f, "connection id is not registered"),
        }
    }
}

/// Hands out connection ids and client ids in order, and remembers which belongs to which.
#[derive(Debug, Default)]
pub struct LinkRegistry {
    last_connection: u64,
    next_player_slot: u32,
    clients: HashMap<LinkNumber, u32>,
}

impl LinkRegistry {
    pub fn starting_at(lead_receiver: u32) -> Self {
        Self {
            last_connection: 0,
            next_player_slot: lead_receiver,
            clients: HashMap::new(),
        }
    }

    pub fn admit_arrivals(&mut self) -> (LinkNumber, u32) {
        self.last_connection = self.last_connection.wrapping_add(1);
        let link = LinkNumber(self.last_connection);
        let client = self.next_player_slot;
        self.next_player_slot = client.wrapping_add(1);
        self.clients.insert(link, client);
        (link, client)
    }

    /// The client behind `connection`. A client id a packet claims for itself is never trusted.
    pub fn trusted_slot(
        &self,
        link: LinkNumber,
        _said_slot: u32,
    ) -> Result<u32, LinkIdentityFault> {
        self.receiver_for(link)
            .ok_or(LinkIdentityFault::NoSuchLink)
    }

    pub fn forget_link(&mut self, link: LinkNumber) -> Option<u32> {
        self.clients.remove(&link)
    }

    pub fn receiver_for(&self, link: LinkNumber) -> Option<u32> {
        self.clients.get(&link).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_and_clients_count_up_from_their_starts() {
        let mut table = LinkRegistry::starting_at(7);
        assert_eq!(table.admit_arrivals(), (LinkNumber(1), 7));
        assert_eq!(table.admit_arrivals(), (LinkNumber(2), 8));
    }

    #[test]
    fn a_claimed_client_id_does_not_matter() {
        let mut table = LinkRegistry::starting_at(1);
        let (link, client) = table.admit_arrivals();
        assert_eq!(table.trusted_slot(link, 99), Ok(client));
        assert_eq!(table.trusted_slot(link, client), Ok(client));
    }

    #[test]
    fn an_unknown_or_retired_connection_does_not_resolve() {
        let mut table = LinkRegistry::starting_at(1);
        assert_eq!(
            table.trusted_slot(LinkNumber(5), 0),
            Err(LinkIdentityFault::NoSuchLink)
        );
        let (link, client) = table.admit_arrivals();
        assert_eq!(table.forget_link(link), Some(client));
        assert_eq!(table.receiver_for(link), None);
        assert_eq!(table.forget_link(link), None);
        assert_eq!(
            table.trusted_slot(link, client),
            Err(LinkIdentityFault::NoSuchLink)
        );
    }

    #[test]
    fn a_retired_connection_id_is_not_handed_out_again() {
        let mut table = LinkRegistry::starting_at(1);
        let (first, _) = table.admit_arrivals();
        table.forget_link(first);
        assert_ne!(table.admit_arrivals().0, first);
    }

    #[test]
    fn a_header_applies_only_to_its_own_epoch() {
        let header = DatagramPreamble {
            round_era: 4,
            ..DatagramPreamble::default()
        };
        assert!(header.valid_for_generation(4));
        assert!(!header.valid_for_generation(5));
        assert_eq!(
            LinkIdentityFault::NoSuchLink.to_string(),
            "connection id is not registered"
        );
    }
}
