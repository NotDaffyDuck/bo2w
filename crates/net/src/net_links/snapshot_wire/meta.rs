//! The whole per-snapshot message a client reads next to the entity delta: match state, every
//! client's own row, what happened, name tables, model objects, entities, the entity kernel, item
//! tables, the collision grid, objectives and the world-object sync bytes, in that order.

use gamemode_t6::{RulesetKind, WarmupStage};
use sim::{GameRoundStage, SnapshotMeta};

use super::codec::{
    Wire, put_flagged, put_list, refuse, take_blob, take_flagged, take_list,
};
use super::feel::{put_feel, take_feel};
use super::names::{put_hud_strings, put_slot_names, take_hud_strings, take_slot_names};
use super::world_sync::WorldPropStreamDecoder;
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// How many bytes each part of a snapshot message took, for the network overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetaSectionSizes {
    pub header_len: usize,
    pub event_len: usize,
    pub alias_len: usize,
    pub dobj_len: usize,
    pub corpse_len: usize,
    pub entity_len: usize,
    pub mover_len: usize,
    pub kernel_len: usize,
    pub item_len: usize,
    pub area_len: usize,
    pub objective_len: usize,
    pub world_len: usize,
}

impl MetaSectionSizes {
    /// The whole message, every section added up.
    pub fn sum_bytes(self) -> usize {
        let front = self.header_len + self.event_len + self.alias_len + self.dobj_len;
        let bodies = self.corpse_len + self.entity_len + self.mover_len + self.kernel_len;
        let tail = self.item_len + self.area_len + self.objective_len + self.world_len;
        front + bodies + tail
    }
}

/// Reserves for the lists a snapshot carries: the count on the wire may be bigger, the vector
/// only grows to what the bytes behind the count really hold.
const CLIENT_RESERVE: usize = 64;
const JOURNAL_RESERVE: usize = 64;
const ENTITY_EVENT_RESERVE: usize = 64;
const PELLET_RESERVE: usize = 256;
const DEBRIS_RESERVE: usize = 64;
const BLOCKER_RESERVE: usize = 64;
const DOBJ_RESERVE: usize = 256;
const ENTITY_RESERVE: usize = 256;
const MOVER_RESERVE: usize = 256;
const ITEM_RESERVE: usize = 16;

/// Reports the size of each section as the writer passes the section's end.
struct SectionMeter {
    since: usize,
}

impl SectionMeter {
    fn start(wr: &ByteSinkWriter) -> Self {
        Self { since: wr.len() }
    }

    fn close(&mut self, wr: &ByteSinkWriter) -> usize {
        let size = wr.len() - self.since;
        self.since = wr.len();
        size
    }
}

fn put_prematch(wr: &mut ByteSinkWriter, step: &WarmupStage) {
    let (tag, elapsed_ms) = match *step {
        WarmupStage::Waiting { spent_ms: elapsed_ms } => (0u8, elapsed_ms),
        WarmupStage::Starting { spent_ms: elapsed_ms } => (1, elapsed_ms),
        WarmupStage::Done => (2, 0),
    };
    tag.put(wr);
    elapsed_ms.put(wr);
}

fn take_prematch(rd: &mut ByteCursorReader<'_>) -> Result<WarmupStage, PacketDecodeFault> {
    let tag = u8::take(rd)?;
    let elapsed_ms = u32::take(rd)?;
    match tag {
        0 => Ok(WarmupStage::Waiting { spent_ms: elapsed_ms }),
        1 => Ok(WarmupStage::Starting { spent_ms: elapsed_ms }),
        2 => Ok(WarmupStage::Done),
        _ => refuse("unknown prematch tag"),
    }
}

pub fn encode_snapshot_meta(wr: &mut ByteSinkWriter, snap: &SnapshotMeta, world_prop_bytes: &[u8]) {
    pack_state_header_parts(wr, snap, world_prop_bytes);
}

/// Writes one snapshot message and says how many bytes each section of it took.
pub fn pack_state_header_parts(
    wr: &mut ByteSinkWriter,
    snapshot: &SnapshotMeta,
    world_prop_bytes: &[u8],
) -> MetaSectionSizes {
    let mut meter = SectionMeter::start(wr);
    let mut sizes = MetaSectionSizes::default();

    snapshot.phase.put(wr);
    snapshot.match_clock_ms.put(wr);
    put_prematch(wr, &snapshot.prematch);
    snapshot.score_limit.put(wr);
    snapshot.time_limit_ms.put(wr);
    snapshot.kind.net_code().put(wr);
    put_list(wr, &snapshot.clients);
    sizes.header_len = meter.close(wr);

    put_list(wr, &snapshot.journal);
    put_list(wr, &snapshot.entity_events);
    put_list(wr, &snapshot.pellet_fx);
    put_list(wr, &snapshot.debris);
    put_list(wr, &snapshot.oriented_blockers);
    put_feel(wr, &snapshot.feel);
    sizes.event_len = meter.close(wr);

    put_slot_names(wr, &snapshot.sound_aliases);
    put_slot_names(wr, &snapshot.effect_names);
    put_slot_names(wr, &snapshot.hud_materials);
    put_hud_strings(wr, &snapshot.hud_strings);
    snapshot.rng.put(wr);
    sizes.alias_len = meter.close(wr);

    put_list(wr, &snapshot.entity_dobjs);
    sizes.dobj_len = meter.close(wr);
    snapshot.corpses.put(wr);
    sizes.corpse_len = meter.close(wr);
    put_list(wr, &snapshot.entities);
    sizes.entity_len = meter.close(wr);
    put_list(wr, &snapshot.script_movers);
    sizes.mover_len = meter.close(wr);
    snapshot.entity_kernel.put(wr);
    sizes.kernel_len = meter.close(wr);

    put_list(wr, &snapshot.item_ammo);
    put_list(wr, &snapshot.item_pickups);
    sizes.item_len = meter.close(wr);

    put_flagged(wr, &snapshot.area_entities);
    sizes.area_len = meter.close(wr);
    snapshot.objectives.put(wr);
    sizes.objective_len = meter.close(wr);

    debug_assert!(world_prop_bytes.len() <= usize::from(u16::MAX));
    wr.put_u16(world_prop_bytes.len() as u16);
    wr.append_raw_slice(world_prop_bytes);
    sizes.world_len = meter.close(wr);
    sizes
}

/// Reads one snapshot message. The world-object bytes are applied to `world_decoder` and also
/// handed back untouched, so the caller can pass them on.
pub fn parse_state_header(
    rd: &mut ByteCursorReader<'_>,
    world_decoder: &mut WorldPropStreamDecoder,
) -> Result<(SnapshotMeta, Vec<u8>), PacketDecodeFault> {
    // A struct literal runs its fields top to bottom, so they are listed in wire order.
    let world_wire;
    let snapshot = SnapshotMeta {
        phase: GameRoundStage::take(rd)?,
        match_clock_ms: u32::take(rd)?,
        prematch: take_prematch(rd)?,
        score_limit: i32::take(rd)?,
        time_limit_ms: u32::take(rd)?,
        kind: match RulesetKind::from_net_code(u8::take(rd)?) {
            Some(kind) => kind,
            None => return refuse("unknown GameModeKind tag"),
        },
        clients: take_list(rd, CLIENT_RESERVE)?,

        journal: take_list(rd, JOURNAL_RESERVE)?,
        entity_events: take_list(rd, ENTITY_EVENT_RESERVE)?,
        pellet_fx: take_list(rd, PELLET_RESERVE)?,
        debris: take_list(rd, DEBRIS_RESERVE)?,
        oriented_blockers: take_list(rd, BLOCKER_RESERVE)?,
        feel: take_feel(rd)?,

        sound_aliases: take_slot_names(rd)?,
        effect_names: take_slot_names(rd)?,
        hud_materials: take_slot_names(rd)?,
        hud_strings: take_hud_strings(rd)?,
        rng: Wire::take(rd)?,

        entity_dobjs: take_list(rd, DOBJ_RESERVE)?,
        corpses: Wire::take(rd)?,
        entities: take_list(rd, ENTITY_RESERVE)?,
        script_movers: take_list(rd, MOVER_RESERVE)?,
        entity_kernel: Wire::take(rd)?,
        item_ammo: take_list(rd, ITEM_RESERVE)?,
        item_pickups: take_list(rd, ITEM_RESERVE)?,
        area_entities: take_flagged(rd, "unknown CM area-sector snapshot tag")?,
        objectives: Wire::take(rd)?,

        world_objects: {
            let len = usize::from(rd.get_u16()?);
            world_wire = take_blob(rd, len)?;
            world_decoder.read_props(&world_wire)?
        },
        // The server swaps each viewer's models itself, so none travel.
        enemy_models: Vec::new(),
        // Only the host's own clients follow each other.
        follows: Vec::new(),
    };
    Ok((snapshot, world_wire))
}
