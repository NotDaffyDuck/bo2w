//! Shootable and destructible things in the map (loop sounds on broken props, glass panes): sent
//! whole every `WORLD_SYNC_PERIOD_TICKS`, as a difference in between, and as one byte when nothing
//! moved.

use sim::{DestructibleLoopSound, Tick, WorldObjectSnapshot};

use super::codec::{Decoded, put_list, refuse, take_list};
use crate::transport::wire::{WireError, WireReader, WireWriter};

pub const WORLD_SYNC_PERIOD_TICKS: u32 = 200;

const SAME_AS_BEFORE: u8 = 0;
const CHANGES_ONLY: u8 = 1;
const EVERYTHING: u8 = 2;

wire_fields!(DestructibleLoopSound { owner, alias_index, origin, volume, pitch });

#[derive(Debug, Default)]
pub struct WorldObjectSyncEncoder {
    baseline: WorldObjectSnapshot,
    force_full: bool,
}

impl WorldObjectSyncEncoder {
    pub fn reset(&mut self) {
        self.baseline = WorldObjectSnapshot::default();
        self.force_full = true;
    }

    pub fn adopt_baseline(&mut self, baseline: WorldObjectSnapshot) {
        self.baseline = baseline;
        self.force_full = false;
    }

    pub fn encode(&mut self, tick: Tick, current: &WorldObjectSnapshot) -> Vec<u8> {
        let mut out = WireWriter::with_capacity(64);
        out.put_i32(current.as_of_ms);
        out.put_u32(current.map_round_epoch);
        let due = self.force_full || tick.0 % WORLD_SYNC_PERIOD_TICKS == 0;
        let unchanged = self.baseline.destructible_loop_sounds == current.destructible_loop_sounds
            && self.baseline.t6_glass == current.t6_glass;
        if due {
            out.put_u8(EVERYTHING);
            put_list(&mut out, &current.destructible_loop_sounds);
            put_glass(
                &mut out,
                current.t6_glass.len(),
                &glass_t6::GlassMsg::Full(current.t6_glass.clone()),
            );
            self.baseline = current.clone();
            self.force_full = false;
        } else if unchanged {
            out.put_u8(SAME_AS_BEFORE);
            self.baseline.as_of_ms = current.as_of_ms;
            self.baseline.map_round_epoch = current.map_round_epoch;
        } else {
            out.put_u8(CHANGES_ONLY);
            put_list(&mut out, &current.destructible_loop_sounds);
            let changes = glass_t6::net::diff(&self.baseline.t6_glass, &current.t6_glass);
            put_glass(&mut out, current.t6_glass.len(), &changes);
            self.baseline = current.clone();
        }
        out.finish()
    }
}

#[derive(Debug, Default)]
pub struct WorldObjectSyncDecoder {
    state: WorldObjectSnapshot,
}

impl WorldObjectSyncDecoder {
    pub fn reset(&mut self) {
        self.state = WorldObjectSnapshot::default();
    }

    pub fn adopt_baseline(&mut self, baseline: WorldObjectSnapshot) {
        self.state = baseline;
    }

    pub fn state(&self) -> &WorldObjectSnapshot {
        &self.state
    }

    pub fn apply_wire(&mut self, wire: &[u8]) -> Result<WorldObjectSnapshot, WireError> {
        let mut input = WireReader::new(wire);
        self.apply_from(&mut input)?;
        if !input.is_empty() {
            return refuse("trailing bytes after world object sync");
        }
        Ok(self.state.clone())
    }

    fn apply_from(&mut self, input: &mut WireReader<'_>) -> Decoded<()> {
        self.state.as_of_ms = input.get_i32()?;
        self.state.map_round_epoch = input.get_u32()?;
        match input.get_u8()? {
            SAME_AS_BEFORE => {}
            CHANGES_ONLY => {
                let sounds = take_list(input, 256)?;
                take_glass(input, &mut self.state.t6_glass)?;
                self.state.destructible_loop_sounds = sounds;
            }
            EVERYTHING => {
                let sounds = take_list(input, 256)?;
                let mut panes = Vec::new();
                take_glass(input, &mut panes)?;
                self.state = WorldObjectSnapshot {
                    as_of_ms: self.state.as_of_ms,
                    map_round_epoch: self.state.map_round_epoch,
                    t6_glass: panes,
                    destructible_loop_sounds: sounds,
                };
            }
            _ => return refuse("unknown world object sync tag"),
        }
        Ok(())
    }
}

pub fn encode_world_object_sync(
    encoder: &mut WorldObjectSyncEncoder,
    tick: Tick,
    current: &WorldObjectSnapshot,
) -> Vec<u8> {
    encoder.encode(tick, current)
}

pub fn decode_world_object_sync_wire(
    decoder: &mut WorldObjectSyncDecoder,
    wire: &[u8],
) -> Result<WorldObjectSnapshot, WireError> {
    decoder.apply_wire(wire)
}

/// The pane count the message is read against, then the glass message itself behind a length.
fn put_glass(out: &mut WireWriter, panes: usize, msg: &glass_t6::GlassMsg) {
    let bytes = glass_t6::net::encode(msg);
    debug_assert!(panes <= usize::from(u16::MAX) && bytes.len() <= usize::from(u16::MAX));
    out.put_u16(panes as u16);
    out.put_u16(bytes.len() as u16);
    out.put_bytes(&bytes);
}

fn take_glass(input: &mut WireReader<'_>, panes: &mut Vec<glass_t6::PaneUpdate>) -> Decoded<()> {
    let expected = usize::from(input.get_u16()?);
    let len = usize::from(input.get_u16()?);
    let bytes = super::codec::take_blob(input, len)?;
    let Ok(msg) = glass_t6::net::decode(&bytes, expected) else {
        return refuse("bad t6 glass message");
    };
    if !matches!(msg, glass_t6::GlassMsg::Full(_)) && panes.len() != expected {
        return refuse("t6 glass delta for another pane count");
    }
    glass_t6::net::apply(panes, msg);
    Ok(())
}
