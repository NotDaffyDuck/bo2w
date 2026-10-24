//! Who a datagram claims to come from: connection ids, the header every packet carries, and
//! the authority's table of which connection belongs to which client.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConnectionId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PacketHeader {
    pub connection: ConnectionId,
    pub sequence: u32,

    /// The newest sequence the sender has seen from the other side.
    pub ack: u32,

    /// The match the sender believes it is in.
    pub epoch: u32,
}

impl PacketHeader {
    pub fn applies_to_epoch(self, live: u32) -> bool {
        self.epoch == live
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityError {
    UnknownConnection,
}

impl core::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownConnection => write!(f, "connection id is not registered"),
        }
    }
}

/// Hands out connection ids and client ids in order, and remembers which belongs to which.
#[derive(Debug, Default)]
pub struct ConnectionTable {
    last_connection: u64,
    next_client: u32,
    clients: HashMap<ConnectionId, u32>,
}

impl ConnectionTable {
    pub fn starting_at(first_client: u32) -> Self {
        Self {
            last_connection: 0,
            next_client: first_client,
            clients: HashMap::new(),
        }
    }

    pub fn accept_new(&mut self) -> (ConnectionId, u32) {
        self.last_connection = self.last_connection.wrapping_add(1);
        let connection = ConnectionId(self.last_connection);
        let client = self.next_client;
        self.next_client = client.wrapping_add(1);
        self.clients.insert(connection, client);
        (connection, client)
    }

    /// The client behind `connection`. A client id a packet claims for itself is never trusted.
    pub fn resolve(
        &self,
        connection: ConnectionId,
        _claimed_client: u32,
    ) -> Result<u32, IdentityError> {
        self.client_of(connection)
            .ok_or(IdentityError::UnknownConnection)
    }

    pub fn retire(&mut self, connection: ConnectionId) -> Option<u32> {
        self.clients.remove(&connection)
    }

    pub fn client_of(&self, connection: ConnectionId) -> Option<u32> {
        self.clients.get(&connection).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_and_clients_count_up_from_their_starts() {
        let mut table = ConnectionTable::starting_at(7);
        assert_eq!(table.accept_new(), (ConnectionId(1), 7));
        assert_eq!(table.accept_new(), (ConnectionId(2), 8));
    }

    #[test]
    fn a_claimed_client_id_does_not_matter() {
        let mut table = ConnectionTable::starting_at(1);
        let (connection, client) = table.accept_new();
        assert_eq!(table.resolve(connection, 99), Ok(client));
        assert_eq!(table.resolve(connection, client), Ok(client));
    }

    #[test]
    fn an_unknown_or_retired_connection_does_not_resolve() {
        let mut table = ConnectionTable::starting_at(1);
        assert_eq!(
            table.resolve(ConnectionId(5), 0),
            Err(IdentityError::UnknownConnection)
        );
        let (connection, client) = table.accept_new();
        assert_eq!(table.retire(connection), Some(client));
        assert_eq!(table.client_of(connection), None);
        assert_eq!(table.retire(connection), None);
        assert_eq!(
            table.resolve(connection, client),
            Err(IdentityError::UnknownConnection)
        );
    }

    #[test]
    fn a_retired_connection_id_is_not_handed_out_again() {
        let mut table = ConnectionTable::starting_at(1);
        let (first, _) = table.accept_new();
        table.retire(first);
        assert_ne!(table.accept_new().0, first);
    }

    #[test]
    fn a_header_applies_only_to_its_own_epoch() {
        let header = PacketHeader {
            epoch: 4,
            ..PacketHeader::default()
        };
        assert!(header.applies_to_epoch(4));
        assert!(!header.applies_to_epoch(5));
        assert_eq!(
            IdentityError::UnknownConnection.to_string(),
            "connection id is not registered"
        );
    }
}
