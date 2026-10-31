//! Snapshot delta streams. The encoder writes each snapshot as the difference from what the peer
//! already holds; the decoder rebuilds the snapshot from that difference. Both sides then treat the
//! snapshot as the new consensus_reached state.

use playerstate_t6::game::AvatarState;
use sim::{ClientId, FlyingProjectile, Snapshot, Tick};

use super::baseline::Baseline;
use super::player::{parse_participant_state, encode_player};
use super::projectile::{ProjectileChangeRecord, decode_projectiles, encode_projectiles};
use crate::net_links::snapshot_wire::{WorldPropStreamDecoder, WorldPropStreamEncoder};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Framing ahead of the payload: tick, then payload length (u32 little-endian each).
const FRAME_HEAD: usize = 8;

/// What a decoder reports when bytes remain once every section has been read.
const LEFTOVER: &str = "trailing bytes after the projectile entity delta";

/// One encoded frame: its tick and the delta bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDiff {
    pub at_tick: Tick,

    bytes: Vec<u8>,
}

impl FrameDiff {
    pub fn pack(&self, sink: &mut ByteSinkWriter) {
        let mut head = [0u8; FRAME_HEAD];
        let (tick, len) = head.split_at_mut(4);
        tick.copy_from_slice(&self.at_tick.0.to_le_bytes());
        len.copy_from_slice(&(self.bytes.len() as u32).to_le_bytes());
        sink.append_raw_slice(&head);
        sink.append_raw_slice(&self.bytes);
    }

    pub fn decode(cursor: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let tick = cursor.read_word32().map(Tick)?;
        let mut bytes = cursor.read_word32().map(|len| vec![0u8; len as usize])?;
        cursor.copy_out(&mut bytes)?;
        Ok(Self { at_tick: tick, bytes })
    }

    pub fn body_length(&self) -> usize {
        self.bytes.len()
    }
}

/// The world-object half of a side, which follows the same adopt/forget life as the baseline.
trait WorldHalf {
    fn hold(&mut self, objects: sim::WorldObjectSnapshot);
    fn forget(&mut self);
}

impl WorldHalf for WorldPropStreamEncoder {
    fn hold(&mut self, objects: sim::WorldObjectSnapshot) {
        WorldPropStreamEncoder::accept_reference_frame(self, objects);
    }
    fn forget(&mut self) {
        WorldPropStreamEncoder::start_over(self);
    }
}

impl WorldHalf for WorldPropStreamDecoder {
    fn hold(&mut self, objects: sim::WorldObjectSnapshot) {
        WorldPropStreamDecoder::accept_reference_frame(self, objects);
    }
    fn forget(&mut self) {
        WorldPropStreamDecoder::start_over(self);
    }
}

/// Everything one end of the stream remembers about the other.
#[derive(Debug, Default)]
struct Side<W> {
    consensus_reached: Baseline,
    world: W,
}

impl<W: WorldHalf> Side<W> {
    fn hold(&mut self, frame: &Snapshot) {
        self.consensus_reached.adopt(frame);
        self.world.hold(frame.meta.world_objects.clone());
    }

    fn forget(&mut self) {
        self.consensus_reached.clear();
        self.world.forget();
    }
}

/// Player section: a u16 count, then per player its peer id and its field diff.
fn write_players(sink: &mut ByteSinkWriter, consensus_reached: &Baseline, roster: &[(ClientId, AvatarState)]) {
    debug_assert!(
        u16::try_from(roster.len()).is_ok(),
        "player count exceeds the wire width"
    );
    sink.put_u16(roster.len() as u16);
    for &(peer, ref now) in roster {
        let was = consensus_reached.player(peer);
        sink.put_u32(peer.0);
        encode_player(sink, &was, now);
    }
}

fn read_players(
    cursor: &mut ByteCursorReader<'_>,
    consensus_reached: &Baseline,
) -> Result<Vec<(ClientId, AvatarState)>, PacketDecodeFault> {
    let count = usize::from(cursor.get_u16()?);
    (0..count)
        .map(|_| {
            let peer = cursor.read_word32().map(ClientId)?;
            let mut state = consensus_reached.player(peer);
            parse_participant_state(cursor, &mut state).map(|()| (peer, state))
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct FrameDiffWriter {
    side: Side<WorldPropStreamEncoder>,
    world_prop_bytes: Vec<u8>,
}

impl FrameDiffWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_projectile_delta(&self) -> &ProjectileChangeRecord {
        &self.side.consensus_reached.last_projectile_delta
    }

    pub fn pack_scene_props(
        &mut self,
        tick: Tick,
        now_snap: &sim::WorldObjectSnapshot,
    ) -> &[u8] {
        let wire = self.side.world.emit_props(tick, now_snap);
        self.world_prop_bytes = wire;
        self.world_prop_bytes.as_slice()
    }

    /// Encode `snapshot` against the baseline, which then becomes `snapshot`.
    pub fn pack(&mut self, frame: &Snapshot) -> FrameDiff {
        let room = frame.players.len().max(1) * 64;
        let mut sink = ByteSinkWriter::with_capacity(room);
        let consensus_reached = &mut self.side.consensus_reached;
        write_players(&mut sink, consensus_reached, &frame.players);
        let sent = encode_projectiles(&mut sink, &consensus_reached.projectiles, &frame.projectiles);
        consensus_reached.adopt(frame);
        consensus_reached.last_projectile_delta = sent;
        FrameDiff { at_tick: frame.tick, bytes: sink.into_bytes() }
    }

    pub fn accept_reference_frame(&mut self, frame: &Snapshot) {
        self.side.hold(frame);
    }

    pub fn start_over(&mut self) {
        self.side.forget();
        self.world_prop_bytes.clear();
    }
}

#[derive(Debug, Default)]
pub struct FrameDiffReader {
    side: Side<WorldPropStreamDecoder>,
}

impl FrameDiffReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last_projectile_delta(&self) -> &ProjectileChangeRecord {
        &self.side.consensus_reached.last_projectile_delta
    }

    pub fn install_scene_props_message(
        &mut self,
        wire: &[u8],
    ) -> Result<sim::WorldObjectSnapshot, PacketDecodeFault> {
        self.side.world.read_props(wire)
    }

    /// Rebuild the frame `diff` describes from the baseline, which then becomes that frame.
    /// The projectile part of the baseline moves even when the delta later proves to have trailing
    /// bytes; the players only move on success.
    pub fn decode(&mut self, diff: &FrameDiff) -> Result<Snapshot, PacketDecodeFault> {
        let consensus_reached = &mut self.side.consensus_reached;
        let mut cursor = ByteCursorReader::new(&diff.bytes);
        let players = read_players(&mut cursor, consensus_reached)?;
        consensus_reached.last_projectile_delta = decode_projectiles(&mut cursor, &mut consensus_reached.projectiles)?;
        if cursor.unread_len() != 0 {
            return Err(PacketDecodeFault::GarbledRequest(LEFTOVER));
        }

        let mut projectiles: Vec<FlyingProjectile> = consensus_reached.projectiles.values().copied().collect();
        projectiles.sort_unstable_by_key(|p| p.id.0);
        consensus_reached.players.clone_from(&players);
        Ok(Snapshot {
            tick: diff.at_tick,
            players,
            projectiles,
            meta: Default::default(),
        })
    }

    pub fn start_over(&mut self) {
        self.side.forget();
    }

    pub fn accept_reference_frame(&mut self, frame: &Snapshot) {
        self.side.hold(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot at `tick` whose players stand at the given x positions.
    fn at(tick: u32, xs: &[(u32, f32)]) -> Snapshot {
        let mut snap = Snapshot {
            tick: Tick(tick),
            players: Vec::with_capacity(xs.len()),
            projectiles: Vec::new(),
            meta: Default::default(),
        };
        for &(id, x) in xs {
            let mut ps = AvatarState::ZERO;
            ps.origin = [x, 0.0, 0.0];
            ps.last_cmd_ms = 50 * tick as i32;
            snap.players.push((ClientId(id), ps));
        }
        snap
    }

    fn ends() -> (FrameDiffWriter, FrameDiffReader) {
        (FrameDiffWriter::new(), FrameDiffReader::new())
    }

    #[test]
    fn framing_survives_a_round_trip() {
        let diff = FrameDiffWriter::new().pack(&at(3, &[(1, 10.0)]));
        let mut w = ByteSinkWriter::new();
        diff.pack(&mut w);
        let framed = w.into_bytes();
        assert_eq!(framed.len(), FRAME_HEAD + diff.body_length());
        assert_eq!(FrameDiff::decode(&mut ByteCursorReader::new(&framed)), Ok(diff));
    }

    #[test]
    fn decoder_tracks_encoder_over_several_ticks() {
        let (mut enc, mut dec) = ends();
        for t in 1..=4u32 {
            let f = t as f32;
            let want = at(t, &[(1, f), (2, -f)]);
            let back = dec.decode(&enc.pack(&want)).expect("decodes");
            assert_eq!((back.tick, back.players), (want.tick, want.players));
        }
    }

    #[test]
    fn repeating_a_snapshot_costs_less() {
        let mut enc = FrameDiffWriter::new();
        let sizes: Vec<usize> = (0..2)
            .map(|_| enc.pack(&at(1, &[(1, 5.0)])).body_length())
            .collect();
        assert!(sizes[1] < sizes[0]);
    }

    #[test]
    fn a_shared_adopted_baseline_lets_the_next_delta_decode() {
        let (mut enc, mut dec) = ends();
        let base = at(5, &[(1, 1.0)]);
        enc.accept_reference_frame(&base);
        dec.accept_reference_frame(&base);
        let want = at(6, &[(1, 2.0)]);
        assert_eq!(dec.decode(&enc.pack(&want)).unwrap().players, want.players);
    }

    #[test]
    fn after_reset_both_ends_start_over() {
        let (mut enc, mut dec) = ends();
        assert!(dec.decode(&enc.pack(&at(1, &[(1, 9.0)]))).is_ok());
        enc.start_over();
        dec.start_over();
        let want = at(2, &[(1, 9.0)]);
        assert_eq!(dec.decode(&enc.pack(&want)).unwrap().players, want.players);
    }

    #[test]
    fn leftover_bytes_are_rejected() {
        let mut diff = FrameDiffWriter::new().pack(&at(1, &[]));
        diff.bytes.push(0);
        assert!(matches!(
            FrameDiffReader::new().decode(&diff),
            Err(PacketDecodeFault::GarbledRequest("trailing bytes after the projectile entity delta"))
        ));
    }

    #[test]
    fn the_encoder_remembers_its_last_projectile_delta_until_reset() {
        let mut enc = FrameDiffWriter::new();
        let mut snap = at(1, &[]);
        snap.projectiles = vec![crate::net_links::snapshot_diff::projectile::sample(1)];
        enc.pack(&snap);
        assert_eq!(enc.last_projectile_delta().changed.len(), 1);
        enc.start_over();
        assert_eq!(enc.last_projectile_delta().changed.len(), 0);
    }
}
