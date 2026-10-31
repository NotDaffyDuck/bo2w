//! Snapshots on the wire as deltas against the last one the other side applied.
//!
//! A delta holds, for every player, only the fields that changed from the baseline, then the
//! projectiles that changed or went away. Encoder and decoder keep matching baselines, so what one
//! writes the other can read back.

mod baseline;
mod player;
mod projectile;
mod diff_codec;
mod move_cmd_wire;

pub use projectile::ProjectileChangeRecord;
pub use diff_codec::{FrameDiffReader, FrameDiff, FrameDiffWriter};
pub(crate) use move_cmd_wire::{unpack_input_record, pack_input_record};
