//! Shootable and destructible things in the map (loop sounds on broken props, glass panes): sent
//! whole every `SCENE_RESYNC_INTERVAL_TICKS`, as a difference in between, and as one byte when nothing
//! moved.

use sim::{DestructibleLoopSound, Tick, WorldObjectSnapshot};

use super::codec::{put_list, refuse, take_list};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub const SCENE_RESYNC_INTERVAL_TICKS: u32 = 200;

const SAME_AS_BEFORE: u8 = 0;
const CHANGES_ONLY: u8 = 1;
const EVERYTHING: u8 = 2;

wire_fields!(DestructibleLoopSound { owner, alias_index, origin, volume, pitch });

/// Remembers what the receiver was last sent, to send only what changed since.
#[derive(Debug, Default)]
pub struct WorldPropStreamEncoder {
    /// What the receiver holds now.
    last_sent: WorldObjectSnapshot,
    /// Send everything next time whatever the tick (after a reset).
    whole_next: bool,
}

impl WorldPropStreamEncoder {
    pub fn start_over(&mut self) {
        self.last_sent = Default::default();
        self.whole_next = true;
    }

    pub fn accept_reference_frame(&mut self, reference: WorldObjectSnapshot) {
        self.last_sent = reference;
        self.whole_next = false;
    }

    pub fn emit_props(&mut self, tick: Tick, now: &WorldObjectSnapshot) -> Vec<u8> {
        let mut w = ByteSinkWriter::with_capacity(64);
        w.put_u32(now.as_of_ms as u32);
        w.put_u32(now.map_round_epoch);
        let resync = self.whole_next || tick.0 % SCENE_RESYNC_INTERVAL_TICKS == 0;
        let same = now.destructible_loop_sounds == self.last_sent.destructible_loop_sounds && now.t6_glass == self.last_sent.t6_glass;
        if !resync && same {
            // Only the clock and the round moved; the receiver already has the rest.
            w.put_u8(SAME_AS_BEFORE);
            (self.last_sent.as_of_ms, self.last_sent.map_round_epoch) = (now.as_of_ms, now.map_round_epoch);
            return w.into_bytes();
        }
        let glass = if resync { glass_t6::GlassMsg::Full(now.t6_glass.clone()) } else { glass_t6::net::diff(&self.last_sent.t6_glass, &now.t6_glass) };
        w.put_u8(if resync { EVERYTHING } else { CHANGES_ONLY });
        // The loop sounds always go whole; only the glass has a difference form.
        put_list(&mut w, &now.destructible_loop_sounds);
        put_glass(&mut w, now.t6_glass.len(), &glass);
        self.last_sent = now.clone();
        self.whole_next = false;
        w.into_bytes()
    }
}

/// Holds what has been received so far; each message is applied on top of it.
#[derive(Debug, Default)]
pub struct WorldPropStreamDecoder {
    held: WorldObjectSnapshot,
}

impl WorldPropStreamDecoder {
    pub fn start_over(&mut self) {
        self.held = Default::default();
    }

    pub fn accept_reference_frame(&mut self, reference: WorldObjectSnapshot) {
        self.held = reference;
    }

    pub fn held_props(&self) -> &WorldObjectSnapshot {
        let Self { held } = self;
        held
    }

    pub fn read_props(&mut self, wire: &[u8]) -> Result<WorldObjectSnapshot, PacketDecodeFault> {
        let mut rd = ByteCursorReader::new(wire);
        self.apply_from(&mut rd)?;
        if !rd.is_empty() {
            return refuse("trailing bytes after world object sync");
        }
        Ok(self.held.clone())
    }

    fn apply_from(&mut self, rd: &mut ByteCursorReader<'_>) -> Result<(), PacketDecodeFault> {
        self.held.as_of_ms = rd.get_i32()?;
        self.held.map_round_epoch = rd.read_word32()?;
        let kind = rd.get_u8()?;
        if kind == SAME_AS_BEFORE {
            return Ok(());
        }
        if kind != CHANGES_ONLY && kind != EVERYTHING {
            return refuse("unknown world object sync tag");
        }
        let sounds = take_list(rd, 256)?;
        // A full message starts the panes over; a difference applies to the ones held.
        let mut panes = if kind == EVERYTHING { Vec::new() } else { std::mem::take(&mut self.held.t6_glass) };
        let read = take_glass(rd, &mut panes);
        if kind == CHANGES_ONLY || read.is_ok() {
            self.held.t6_glass = panes;
        }
        read?;
        self.held.destructible_loop_sounds = sounds;
        Ok(())
    }
}

pub fn pack_scene_prop_resync(stream: &mut WorldPropStreamEncoder, tick: Tick, now: &WorldObjectSnapshot) -> Vec<u8> {
    stream.emit_props(tick, now)
}

pub fn parse_scene_prop_resync(stream: &mut WorldPropStreamDecoder, bytes: &[u8]) -> Result<WorldObjectSnapshot, PacketDecodeFault> {
    stream.read_props(bytes)
}

/// The pane count the message is read against, then the glass message itself behind a length.
fn put_glass(wr: &mut ByteSinkWriter, panes: usize, msg: &glass_t6::GlassMsg) {
    let bytes = glass_t6::net::encode(msg);
    debug_assert!(panes <= usize::from(u16::MAX) && bytes.len() <= usize::from(u16::MAX));
    wr.put_u16(panes as u16);
    wr.put_u16(bytes.len() as u16);
    wr.append_raw_slice(&bytes);
}

fn take_glass(rd: &mut ByteCursorReader<'_>, panes: &mut Vec<glass_t6::PaneUpdate>) -> Result<(), PacketDecodeFault> {
    let expected = usize::from(rd.get_u16()?);
    let len = usize::from(rd.get_u16()?);
    let bytes = super::codec::take_blob(rd, len)?;
    let Ok(msg) = glass_t6::net::decode(&bytes, expected) else {
        return refuse("bad t6 glass message");
    };
    if !matches!(msg, glass_t6::GlassMsg::Full(_)) && panes.len() != expected {
        return refuse("t6 glass delta for another pane count");
    }
    glass_t6::net::apply(panes, msg);
    Ok(())
}
