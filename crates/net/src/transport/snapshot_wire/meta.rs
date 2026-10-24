//! The whole per-snapshot message a client reads next to the entity delta: match state, every
//! client's own row, what happened, name tables, model objects, entities, the entity kernel, item
//! tables, the collision grid, objectives and the world-object sync bytes, in that order.

use gamemode_t6::{GameModeKind, PrematchStep};
use sim::{MatchPhase, SnapshotMeta};

use super::codec::{
    Decoded, Wire, put_flagged, put_list, refuse, take_blob, take_flagged, take_list,
};
use super::feel::{put_feel, take_feel};
use super::names::{put_hud_strings, put_slot_names, take_hud_strings, take_slot_names};
use super::world_sync::WorldObjectSyncDecoder;
use crate::transport::wire::{WireReader, WireWriter};

/// How many bytes each part of a snapshot message took, for the network overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotMetaSectionBytes {
    pub match_header: usize,
    pub events: usize,
    pub aliases: usize,
    pub entity_dobjs: usize,
    pub corpses: usize,
    pub entities: usize,
    pub script_movers: usize,
    pub entity_kernel: usize,
    pub item_tables: usize,
    pub area_entities: usize,
    pub objectives: usize,
    pub world_objects: usize,
}

impl SnapshotMetaSectionBytes {
    pub fn total(self) -> usize {
        let Self {
            match_header,
            events,
            aliases,
            entity_dobjs,
            corpses,
            entities,
            script_movers,
            entity_kernel,
            item_tables,
            area_entities,
            objectives,
            world_objects,
        } = self;
        [
            match_header,
            events,
            aliases,
            entity_dobjs,
            corpses,
            entities,
            script_movers,
            entity_kernel,
            item_tables,
            area_entities,
            objectives,
            world_objects,
        ]
        .iter()
        .sum()
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

/// Hands out the size of each section as the writer passes the section's end.
struct SectionMeter {
    since: usize,
}

impl SectionMeter {
    fn start(out: &WireWriter) -> Self {
        Self { since: out.len() }
    }

    fn close(&mut self, out: &WireWriter) -> usize {
        let size = out.len() - self.since;
        self.since = out.len();
        size
    }
}

fn put_prematch(out: &mut WireWriter, step: &PrematchStep) {
    let (tag, elapsed_ms) = match *step {
        PrematchStep::Waiting { elapsed_ms } => (0u8, elapsed_ms),
        PrematchStep::Starting { elapsed_ms } => (1, elapsed_ms),
        PrematchStep::Done => (2, 0),
    };
    tag.put(out);
    elapsed_ms.put(out);
}

fn take_prematch(input: &mut WireReader<'_>) -> Decoded<PrematchStep> {
    let tag = u8::take(input)?;
    let elapsed_ms = u32::take(input)?;
    match tag {
        0 => Ok(PrematchStep::Waiting { elapsed_ms }),
        1 => Ok(PrematchStep::Starting { elapsed_ms }),
        2 => Ok(PrematchStep::Done),
        _ => refuse("unknown prematch tag"),
    }
}

pub fn encode_snapshot_meta(out: &mut WireWriter, meta: &SnapshotMeta, world_objects_wire: &[u8]) {
    encode_snapshot_meta_sections(out, meta, world_objects_wire);
}

pub fn encode_snapshot_meta_sections(
    out: &mut WireWriter,
    meta: &SnapshotMeta,
    world_objects_wire: &[u8],
) -> SnapshotMetaSectionBytes {
    let mut meter = SectionMeter::start(out);

    meta.phase.put(out);
    meta.match_elapsed_ms.put(out);
    put_prematch(out, &meta.prematch);
    meta.score_limit.put(out);
    meta.time_limit_ms.put(out);
    meta.kind.wire_tag().put(out);
    put_list(out, &meta.clients);
    let match_header = meter.close(out);

    put_list(out, &meta.journal);
    put_list(out, &meta.entity_events);
    put_list(out, &meta.pellet_fx);
    put_list(out, &meta.debris);
    put_list(out, &meta.oriented_blockers);
    put_feel(out, &meta.feel);
    let events = meter.close(out);

    put_slot_names(out, &meta.sound_aliases);
    put_slot_names(out, &meta.effect_names);
    put_slot_names(out, &meta.hud_materials);
    put_hud_strings(out, &meta.hud_strings);
    meta.rng.put(out);
    let aliases = meter.close(out);

    put_list(out, &meta.entity_dobjs);
    let entity_dobjs = meter.close(out);

    meta.corpses.put(out);
    let corpses = meter.close(out);

    put_list(out, &meta.entities);
    let entities = meter.close(out);

    put_list(out, &meta.script_movers);
    let script_movers = meter.close(out);

    meta.entity_kernel.put(out);
    let entity_kernel = meter.close(out);

    put_list(out, &meta.item_ammo);
    put_list(out, &meta.item_pickups);
    let item_tables = meter.close(out);

    put_flagged(out, &meta.area_entities);
    let area_entities = meter.close(out);

    meta.objectives.put(out);
    let objectives = meter.close(out);

    debug_assert!(world_objects_wire.len() <= usize::from(u16::MAX));
    out.put_u16(world_objects_wire.len() as u16);
    out.put_bytes(world_objects_wire);
    let world_objects = meter.close(out);

    SnapshotMetaSectionBytes {
        match_header,
        events,
        aliases,
        entity_dobjs,
        corpses,
        entities,
        script_movers,
        entity_kernel,
        item_tables,
        area_entities,
        objectives,
        world_objects,
    }
}

/// Reads one snapshot message. The world-object bytes are applied to `world_decoder` and also
/// handed back untouched, so the caller can pass them on.
pub fn decode_snapshot_meta(
    input: &mut WireReader<'_>,
    world_decoder: &mut WorldObjectSyncDecoder,
) -> Decoded<(SnapshotMeta, Vec<u8>)> {
    let phase = MatchPhase::take(input)?;
    let match_elapsed_ms = u32::take(input)?;
    let prematch = take_prematch(input)?;
    let score_limit = i32::take(input)?;
    let time_limit_ms = u32::take(input)?;
    let Some(kind) = GameModeKind::from_wire_tag(u8::take(input)?) else {
        return refuse("unknown GameModeKind tag");
    };
    let clients = take_list(input, CLIENT_RESERVE)?;

    let journal = take_list(input, JOURNAL_RESERVE)?;
    let entity_events = take_list(input, ENTITY_EVENT_RESERVE)?;
    let pellet_fx = take_list(input, PELLET_RESERVE)?;
    let debris = take_list(input, DEBRIS_RESERVE)?;
    let oriented_blockers = take_list(input, BLOCKER_RESERVE)?;
    let feel = take_feel(input)?;

    let sound_aliases = take_slot_names(input)?;
    let effect_names = take_slot_names(input)?;
    let hud_materials = take_slot_names(input)?;
    let hud_strings = take_hud_strings(input)?;
    let rng = Wire::take(input)?;

    let entity_dobjs = take_list(input, DOBJ_RESERVE)?;
    let corpses = Wire::take(input)?;
    let entities = take_list(input, ENTITY_RESERVE)?;
    let script_movers = take_list(input, MOVER_RESERVE)?;
    let entity_kernel = Wire::take(input)?;
    let item_ammo = take_list(input, ITEM_RESERVE)?;
    let item_pickups = take_list(input, ITEM_RESERVE)?;
    let area_entities = take_flagged(input, "unknown CM area-sector snapshot tag")?;
    let objectives = Wire::take(input)?;

    let wire_len = usize::from(input.get_u16()?);
    let wire = take_blob(input, wire_len)?;
    let world_objects = world_decoder.apply_wire(&wire)?;

    let meta = SnapshotMeta {
        phase,
        match_elapsed_ms,
        prematch,
        score_limit,
        time_limit_ms,
        kind,
        clients,
        journal,
        entity_events,
        pellet_fx,
        debris,
        oriented_blockers,
        feel,
        // The server swaps each viewer's models itself, so none travel.
        enemy_models: Vec::new(),
        sound_aliases,
        effect_names,
        hud_materials,
        hud_strings,
        rng,
        world_objects,
        area_entities,
        objectives,
        entity_dobjs,
        entities,
        script_movers,
        entity_kernel,
        corpses,
        item_ammo,
        item_pickups,
    };
    Ok((meta, wire))
}
