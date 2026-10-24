//! Identifiers and small shared enums that several sections of the snapshot wire carry.

use sim::{
    ClassId, ClientId, ClientLifecycle, EntityRef, EventSequence, LifeSequence, MatchPhase,
    ScriptModelId, Tick,
};

use super::codec::{Decoded, Wire, refuse};
use crate::transport::wire::{WireReader, WireWriter};

wire_newtype! {
    ClassId => u32,
    ClientId => u32,
    EventSequence => u32,
    LifeSequence => u32,
    Tick => u32,
}

impl Wire for ScriptModelId {
    fn put(&self, out: &mut WireWriter) {
        self.to_wire().put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        Ok(Self::from_wire(u32::take(input)?))
    }
}

wire_tags! {
    MatchPhase, "unknown MatchPhase tag", {
        MatchPhase::Warmup => 0,
        MatchPhase::Playing => 1,
        MatchPhase::Intermission => 2,
        MatchPhase::PostGame => 3,
    }
}

wire_tags! {
    ClientLifecycle, "unknown ClientLifecycle tag", {
        ClientLifecycle::Connecting => 0,
        ClientLifecycle::ChoosingClass => 1,
        ClientLifecycle::Alive => 2,
        ClientLifecycle::Dead => 3,
        ClientLifecycle::Spectating => 4,
        ClientLifecycle::SpawnPending => 5,
        ClientLifecycle::RespawnPending => 6,
        ClientLifecycle::Intermission => 7,
    }
}

/// A reference to another entity: its slot number and how many times that slot has been reused.
impl Wire for EntityRef {
    fn put(&self, out: &mut WireWriter) {
        self.number().put(out);
        self.generation().put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let number = i32::take(input)?;
        let generation = u32::take(input)?;
        match Self::from_parts(number, generation) {
            Ok(entity) => Ok(entity),
            Err(_) => refuse("EntityKernel relation number out of range"),
        }
    }
}
