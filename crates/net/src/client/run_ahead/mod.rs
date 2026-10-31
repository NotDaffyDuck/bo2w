//! Client-side prediction: the commands a client has run ahead of the authority, and how a
//! snapshot from the authority is reconciled with them.
//!
//! * [`history`] holds the unacknowledged commands and matches acknowledgements to them.
//! * [`metrics`] counts what reconciliation did, for the log line and tests.
//! * [`ground`] answers small questions about what a snapshot says about the local player.
//! * [`state`] owns [`LocalForecast`] and the forward-prediction step.
//! * [`reconcile`] adopts snapshots, retires acknowledgements and replays what is left.

mod ground;
mod history;
mod metrics;
mod reconcile;
mod state;

pub use ground::ground_entity_kind;
pub use history::{ReceiptLookup, InputCommandNumber, DEFAULT_MOVE_LOG_LIMIT, ForecastMoveLog, ForecastMoveEntry};
pub use metrics::{ForecastCounters, CorrectionResult};
pub use reconcile::UNCONFIRMED_STATE_CAP;
pub use state::LocalForecast;
