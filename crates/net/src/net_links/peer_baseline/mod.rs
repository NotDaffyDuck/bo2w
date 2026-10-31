//! What the host keeps per peer so it can send snapshots as changes against one the peer
//! has confirmed receiving.

mod flaky;
mod table;

pub use flaky::LossySimulatedQueue;
pub use table::ConfirmedReferenceTable;
