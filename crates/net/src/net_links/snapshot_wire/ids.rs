//! Identifiers and small shared enums that several sections of the snapshot wire carry.

use sim::{
    ClassId, ClientId, PlayerSessionStage, EntHandle, EventSequence, LifeSequence, GameRoundStage,
    ScriptModelId, Tick,
};

use super::codec::{Wire, refuse};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

wire_newtype! {
    ClassId => u32,
    ClientId => u32,
    EventSequence => u32,
    LifeSequence => u32,
    Tick => u32,
}

impl Wire for ScriptModelId {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.encode_for_net().put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        Ok(Self::decode_net(u32::take(rd)?))
    }
}

wire_tags! {
    GameRoundStage, "unknown MatchPhase tag", {
        GameRoundStage::Warmup => 0,
        GameRoundStage::Playing => 1,
        GameRoundStage::Intermission => 2,
        GameRoundStage::PostGame => 3,
    }
}

wire_tags! {
    PlayerSessionStage, "unknown ClientLifecycle tag", {
        PlayerSessionStage::Connecting => 0,
        PlayerSessionStage::PickingLoadout => 1,
        PlayerSessionStage::Alive => 2,
        PlayerSessionStage::Dead => 3,
        PlayerSessionStage::Spectating => 4,
        PlayerSessionStage::AwaitingFirstLife => 5,
        PlayerSessionStage::AwaitingNextLife => 6,
        PlayerSessionStage::Intermission => 7,
    }
}

/// A reference to another entity: its slot number and how many times that slot has been reused.
impl Wire for EntHandle {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.slot().put(wr);
        self.generation().put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let number = i32::take(rd)?;
        let generation = u32::take(rd)?;
        match Self::from_parts(number, generation) {
            Ok(entity) => Ok(entity),
            Err(_) => refuse("EntityKernel relation number is out of range"),
        }
    }
}
