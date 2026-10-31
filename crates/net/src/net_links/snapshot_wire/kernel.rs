//! The entity kernel: which of the world's entity slots are alive, who owns, carries or stands on
//! whom, and which slots are next in line to be reused.

use sim::{
    EntityKernelOccupiedSnapshot, EntityKernelSlotSnapshot, EntityKernelSnapshot, EntityRelations,
    EntityBehaviorClass,
};

use super::codec::{
    Wire, put_flagged, put_list, refuse, take_counted, take_flagged,
};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Room reserved up front for slots; the count on the wire may be larger but is read lazily.
const SLOT_RESERVE: usize = 1024;

wire_tags! {
    EntityBehaviorClass, "unknown EntityRunKind tag", {
        EntityBehaviorClass::ScriptedMover => 0,
        EntityBehaviorClass::Item => 1,
        EntityBehaviorClass::Missile => 2,
        EntityBehaviorClass::TempEvent => 3,
        EntityBehaviorClass::MainLightSource => 4,
        EntityBehaviorClass::General => 5,
        EntityBehaviorClass::FallenPlayerBody => 6,
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
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.generation.put(wr);
        self.freed_at_ms.put(wr);
        put_flagged(wr, &self.occupied);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let gen_num = u32::take(rd)?;
        let freed_ms = i32::take(rd)?;
        let occupied = take_flagged(rd, "bad EntityKernel occupancy tag")?;
        Ok(Self { generation: gen_num, occupied, freed_at_ms: freed_ms })
    }
}

impl Wire for EntityKernelSnapshot {
    fn put(&self, wr: &mut ByteSinkWriter) {
        debug_assert!(self.validate().is_ok());
        self.high_water.put(wr);
        self.level_time_ms.put(wr);
        self.frame_serial.put(wr);
        put_list(wr, &self.slots);
        put_list(wr, &self.free_fifo);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let peak = i32::take(rd)?;
        let lvl_ms = i32::take(rd)?;
        let frame_serial = u32::take(rd)?;
        let slot_total = usize::from(rd.get_u16()?);
        let slots = take_counted(rd, slot_total, SLOT_RESERVE)?;
        let free_count = usize::from(rd.get_u16()?);
        if free_count > slot_total {
            return refuse("EntityKernel free FIFO exceeds slot count");
        }
        let free_fifo = take_counted(rd, free_count, SLOT_RESERVE)?;
        let kernel = Self { slots, high_water: peak, free_fifo, level_time_ms: lvl_ms, frame_serial };
        match kernel.validate() {
            Ok(()) => Ok(kernel),
            Err(_) => refuse("invalid EntityKernel snapshot"),
        }
    }
}
