//! The binary form of everything the host sends a client besides the entity delta, and of what a
//! client sends back: actions, events, and the snapshot message with its sections.
//!
//! Each type that travels says how it is written once (`codec::Wire`) and the reader mirrors the
//! writer field for field. The bytes are part of the protocol a second PC speaks, so the order and
//! width of every field, and every refusal message, stay as they are.

#[macro_use]
mod codec;

mod actions;
mod area;
mod client_meta;
mod dobj;
mod entities;
mod feel;
mod ids;
mod kernel;
mod meta;
mod names;
mod objectives;
mod records;
mod screen;
mod sim_events;
mod world_sync;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;

pub(crate) use actions::{decode_action, encode_action};
pub use actions::{decode_actions, encode_actions};
pub use meta::{
    SnapshotMetaSectionBytes, decode_snapshot_meta, encode_snapshot_meta,
    encode_snapshot_meta_sections,
};
pub(crate) use sim_events::{decode_event, encode_event};
pub use world_sync::{
    WORLD_SYNC_PERIOD_TICKS, WorldObjectSyncDecoder, WorldObjectSyncEncoder,
    decode_world_object_sync_wire, encode_world_object_sync,
};
