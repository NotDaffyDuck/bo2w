//! Session and match state machines for both sides of a connection. Everything here is pure: events
//! go in, effects and log lines come out.

mod client;
mod match_host_books;
mod deadlines;
mod world_rules;
mod session_state;
mod transition;

pub use client::PlayerRoundCore;
pub use match_host_books::{
    HostRoundInstall, HostRoundCore, HostRoundConsequence, HostRoundOccurrence, GameRoundStage, PeerEntryGate,
    PlayerJoinStage,
};
pub use deadlines::{
    AbortStep, LOAD_TIMEOUT_MILLIS, PEER_ENTRY_TIMEOUT_MILLIS, LoadStageKind, LoadStageTimer,
    ACTIVE_QUIET_LIMIT_MILLIS, SessionAbortRecord,
};
pub use world_rules::{
    HostMapLoaded, loadout_pick_permitted, acknowledge_tagged_scene_loaded, level_ready_agrees_with_host,
    teardown_hits_epoch, teardown_hits_key,
};
pub use session_state::{SessionInstallParams, SessionStateCore, GameSessionNotice, SessionSnapshotView, supersedes_applied};
pub(crate) use transition::describe_session_change;
