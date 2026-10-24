//! Session and match state machines for both sides of a connection. Everything here is pure: events
//! go in, effects and log lines come out.

mod client;
mod host;
mod progress;
mod rules;
mod session;
mod transition;

pub use client::ClientMatchCore;
pub use host::{
    HostMatchApply, HostMatchCore, HostMatchEffect, HostMatchEvent, MatchPhase, PeerAdmission,
    PeerPhase,
};
pub use progress::{
    FailStage, LOADING_DEADLINE_MS, PEER_ADMISSION_DEADLINE_MS, ProgressPhase, ProgressWatch,
    RUNNING_SILENCE_MS, SessionFail,
};
pub use rules::{
    HostWorldReady, class_select_allowed, confirm_keyed_world_ready, map_loaded_matches_host,
    match_boundary_applies, match_key_boundary_applies,
};
pub use session::{SessionApply, SessionCore, SessionEvent, SessionView, state_is_newer};
pub(crate) use transition::format_session_transition;
