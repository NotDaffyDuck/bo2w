//! Who an event is meant for.

use crate::world::ClientId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventAudience {
    All,
    AllExcept(ClientId),
    Client(ClientId),
    Clients(Vec<ClientId>),
}

impl EventAudience {
    /// Whether `id` is among the listeners.
    pub fn projects_to(&self, id: ClientId) -> bool {
        match self {
            Self::All => true,
            Self::AllExcept(skipped) => *skipped != id,
            Self::Client(only) => *only == id,
            Self::Clients(listeners) => listeners.contains(&id),
        }
    }

    /// Two people, or just one when they are the same client.
    pub fn pair(a: ClientId, b: ClientId) -> Self {
        match a == b {
            true => Self::Client(a),
            false => Self::Clients(vec![a, b]),
        }
    }
}
