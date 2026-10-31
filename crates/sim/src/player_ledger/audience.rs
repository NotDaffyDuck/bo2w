//! Who an event is meant for.

use crate::sim_parts::SeatNo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRecipients {
    All,
    AllExcept(SeatNo),
    Client(SeatNo),
    Clients(Vec<SeatNo>),
}

impl EventRecipients {
    /// Whether `id` is among the listeners.
    pub fn is_visible_to(&self, id: SeatNo) -> bool {
        match self {
            Self::AllExcept(skipped) => *skipped != id,
            Self::Client(only) => *only == id,
            Self::Clients(listeners) => listeners.contains(&id),
            _ => true,
        }
    }

    /// Two people, or just one when they are the same client.
    pub fn pair(first: SeatNo, second: SeatNo) -> Self {
        if first == second {
            Self::Client(first)
        } else {
            Self::Clients(vec![first, second])
        }
    }
}
