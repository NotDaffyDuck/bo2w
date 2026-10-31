//! Sign-on: how a client walks from "no match" to "may pick a class", and the host-side reports that
//! go with it.
//!
//! * `phase` - the phases and the pure rules that derive one from the link and the epoch
//! * `state` - the `JoinProgressRecord` resource that records the current phase
//! * `defer` - the test knobs that hold the host's world-ready and bootstrap reports back
//! * `entry_gate` - the client's admission resource and the system feeding it match facts
//! * `progress` - the system that advances `JoinProgressRecord` every frame
//! * `map_loaded` - the system that tells the master the local world is installed
//! * `boundary` - the system that resets both ends of the link when a match is torn down

mod entry_gate;
mod boundary;
mod defer;
mod map_loaded;
mod join_phase;
mod progress;
mod state;

pub use entry_gate::{PlayerEntryTicket, process_entry_ticket_events, current_round_key};
pub use boundary::step_match_edge_phase;
pub use defer::{HoldTransferInstalled, HoldMapLoadedSignal, world_ready_held_back};
pub use map_loaded::step_map_ready_phase;
pub use join_phase::{GameRoundSerial, JoinFailureCause, JoinSequenceStep, join_stage_now, stage_of_link};
pub use progress::step_join_handshake;
pub use state::JoinProgressRecord;
