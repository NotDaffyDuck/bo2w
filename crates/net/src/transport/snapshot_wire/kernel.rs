//! The entity kernel: which of the world's entity slots are alive, who owns, carries or stands on
//! whom, and which slots are next in line to be reused.

use sim::{
    EntityKernelOccupiedSnapshot, EntityKernelSlotSnapshot, EntityKernelSnapshot, EntityRelations,
    EntityRunKind,
};

use super::codec::{
    Decoded, Wire, put_flagged, put_list, refuse, take_counted, take_flagged,
};
use crate::transport::wire::{WireReader, WireWriter};

/// Room reserved up front for slots; the count on the wire may be larger but is read lazily.
const SLOT_RESERVE: usize = 1024;

wire_tags! {
    EntityRunKind, "unknown EntityRunKind tag", {
        EntityRunKind::ScriptMover => 0,
        EntityRunKind::Item => 1,
        EntityRunKind::Missile => 2,
        EntityRunKind::TempEvent => 3,
        EntityRunKind::PrimaryLight => 4,
        EntityRunKind::General => 5,
        EntityRunKind::PlayerCorpse => 6,
    }
}

wire_fields!(EntityRelations {
    owner: flag "bad EntityKernel relation tag",
    parent: flag "bad EntityKernel relation tag",
    ground: flag "bad EntityKernel relation tag",
    parent_tag,
    parent_link_axis,
    parent_link_origin,
});

wire_fields!(EntityKernelOccupiedSnapshot {
    kind,
    linked,
    relations,
    next_think_ms: flag "bad optional i32 tag",
    transient_event_time_ms: flag "bad optional i32 tag",
});

/// A slot opens with its reuse count and the time it was freed, then says whether it is in use.
impl Wire for EntityKernelSlotSnapshot {
    fn put(&self, out: &mut WireWriter) {
        self.generation.put(out);
        self.freed_at_ms.put(out);
        put_flagged(out, &self.occupied);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let generation = u32::take(input)?;
        let freed_at_ms = i32::take(input)?;
        let occupied = take_flagged(input, "bad EntityKernel occupancy tag")?;
        Ok(Self { generation, occupied, freed_at_ms })
    }
}

impl Wire for EntityKernelSnapshot {
    fn put(&self, out: &mut WireWriter) {
        debug_assert!(self.validate().is_ok());
        self.high_water.put(out);
        self.level_time_ms.put(out);
        self.frame_serial.put(out);
        put_list(out, &self.slots);
        put_list(out, &self.free_fifo);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let high_water = i32::take(input)?;
        let level_time_ms = i32::take(input)?;
        let frame_serial = u32::take(input)?;
        let slot_count = usize::from(input.get_u16()?);
        let slots = take_counted(input, slot_count, SLOT_RESERVE)?;
        let free_count = usize::from(input.get_u16()?);
        if free_count > slot_count {
            return refuse("EntityKernel free FIFO exceeds slot count");
        }
        let free_fifo = take_counted(input, free_count, SLOT_RESERVE)?;
        let kernel = Self { slots, high_water, free_fifo, level_time_ms, frame_serial };
        match kernel.validate() {
            Ok(()) => Ok(kernel),
            Err(_) => refuse("invalid EntityKernel snapshot"),
        }
    }
}
