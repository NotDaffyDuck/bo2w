//! Client-side prediction: the commands a client has run ahead of the authority, and how a
//! snapshot from the authority is reconciled with them.
//!
//! * [`history`] holds the unacknowledged commands and matches acknowledgements to them.
//! * [`metrics`] counts what reconciliation did, for the log line and tests.
//! * [`ground`] answers small questions about what a snapshot says about the local player.
//! * [`state`] owns [`ClientPrediction`] and the forward-prediction step.
//! * [`reconcile`] adopts snapshots, retires acknowledgements and replays what is left.

mod ground;
mod history;
mod metrics;
mod reconcile;
mod state;

pub use ground::snapshot_ground_e_type;
pub use history::{AckMatch, CmdSeq, DEFAULT_HISTORY_CAP, MoveHistory, MoveRecord};
pub use metrics::{PredictionMetrics, ReconcileOutcome};
pub use reconcile::MAX_UNACKED_SNAPSHOTS;
pub use state::ClientPrediction;
