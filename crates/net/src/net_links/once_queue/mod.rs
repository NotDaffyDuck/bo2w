//! Rows the server must get to a client exactly once and in order: one queue per client, held
//! until the client's confirm_receipt covers them, and the bytes they travel as.

mod codec;
mod hub;
mod queue;
mod row;

pub use codec::{unpack_guaranteed_body, pack_guaranteed_body};
pub use hub::GuaranteedEventRouter;
pub use queue::{GUARANTEED_BACKLOG_LIMIT, GuaranteedEventBacklog, GuaranteedEnvelope};
pub(crate) use queue::is_newer_seq;
pub use row::{ActionRulingNotice, GuaranteedEntry};
