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
mod objective_wire;
mod records;
mod screen;
mod sim_events;
mod world_sync;

#[cfg(test)]
mod golden;
#[cfg(test)]
mod tests;

pub(crate) use actions::{read_intent, pack_command};
pub use actions::{read_intent_list, write_intent_list};
pub use meta::{
    MetaSectionSizes, parse_state_header, encode_snapshot_meta,
    pack_state_header_parts,
};
pub(crate) use sim_events::{read_occurrence, pack_occurrence};
pub use world_sync::{
    SCENE_RESYNC_INTERVAL_TICKS, WorldPropStreamDecoder, WorldPropStreamEncoder,
    parse_scene_prop_resync, pack_scene_prop_resync,
};
