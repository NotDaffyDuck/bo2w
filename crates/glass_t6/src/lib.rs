//! Black Ops II windows, written from BO2's own glass system (facts in
//! bo2work/facts/glass-t6.md). The server keeps each pane's health and
//! decides when it cracks or shatters ([`server`]); the states travel in
//! the snapshot ([`net`]); each client applies them ([`client`]) and turns
//! a shattered pane into falling, chipping shards ([`shards`]).
//!
//! No engine, launcher or game-session types: callers hand in the map's
//! panes, hits and a world trace, and read back states and shard meshes.

pub mod client;
pub mod net;
pub mod server;
pub mod shards;

pub use client::{ClientPane, Cue, FxPoint, GlassesClient, shatter_fx_points};
pub use net::{GlassMsg, NetError, PaneUpdate};

pub use server::{
    Cause, GlassesServer, PaneChange, PaneDef, PaneState, ServerPane, Smash, means_from_name,
    means_name, means_shatters,
};
pub use shards::{
    T6_GLASS_LOW_LOD_DIST, T6Glass, T6PaneShape, T6Shard, T6ShardCounts, T6ShardMotion,
    T6ShardVertex,
};

/// Where a falling shard's sweep through the world stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassWorldContact {
    pub fraction: f32,
    pub end: [f32; 3],
    pub normal: [f32; 3],
    pub startsolid: bool,
    pub pane: Option<u32>,
}

/// The world as falling shards see it: a sweep from `start` to `end`.
pub trait GlassWorldTrace {
    fn sweep(&self, start: [f32; 3], end: [f32; 3]) -> Option<GlassWorldContact>;
}
