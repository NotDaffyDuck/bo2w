//! The model objects the server builds for script-spawned entities: which models are bolted
//! together, which parts are shown, and the animation tree driving them.

use sim::{ServerModelSource, ScriptModelId};
use xmodel_runtime::{
    RigLayout, RigMeshEntry, DObjRagdoll, RigPoseRecord, HiddenParts,
    PartSet, TagOffset, GraphSlot, NodeClock, GraphNodeRecord, AnimGraphRole, GraphCapture,
};

use super::codec::{Wire, put_flagged, put_list, refuse, take_flagged, take_list};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Models in one object / nodes in one tree that are reserved for before the bytes prove them.
const MODEL_RESERVE: usize = 20;
const NODE_RESERVE: usize = 256;

/// A parent that is not there travels as the largest two-byte value.
const NO_PARENT: u16 = u16::MAX;

/// Who a model object belongs to. Only script models have one so far.
impl Wire for ServerModelSource {
    fn put(&self, wr: &mut ByteSinkWriter) {
        match self {
            Self::ScriptModel(id) => {
                wr.put_u8(0);
                id.put(wr);
            }
        }
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        match rd.get_u8()? {
            0 => ScriptModelId::take(rd).map(Self::ScriptModel),
            _ => refuse("unknown DObj owner tag"),
        }
    }
}

impl Wire for PartSet {
    fn put(&self, wr: &mut ByteSinkWriter) {
        <[u32; Self::BIT_CHUNK_COUNT] as Wire>::put(self.words(), wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        <[u32; Self::BIT_CHUNK_COUNT] as Wire>::take(rd).map(Self::from_bit_chunks)
    }
}

impl Wire for HiddenParts {
    fn put(&self, wr: &mut ByteSinkWriter) {
        <[u32; Self::BIT_CHUNK_COUNT] as Wire>::put(self.words(), wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        <[u32; Self::BIT_CHUNK_COUNT] as Wire>::take(rd).map(Self::from_bit_chunks)
    }
}

impl Wire for RigMeshEntry {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.model.put(wr);
        self.host_index.unwrap_or(NO_PARENT).put(wr);
        put_flagged(wr, &self.mount_tag);
        self.no_hits.put(wr);
        put_flagged(wr, &self.tag_offset);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let model = String::take(rd)?;
        let parent = u16::take(rd)?;
        let attach_tag = take_flagged(rd, "unknown optional text tag")?;
        let ignore_collision = bool::take(rd)?;
        let tag_offset = take_flagged(rd, "unknown optional tag offset tag")?;
        Ok(Self {
            model,
            host_index: (parent != NO_PARENT).then_some(parent),
            mount_tag: attach_tag,
            no_hits: ignore_collision,
            tag_offset,
        })
    }
}

/// Where a model sits on its tag: three offsets, then pitch, yaw and roll.
impl Wire for TagOffset {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.offset().put(wr);
        self.rotation().put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let offset = <[f32; 3]>::take(rd)?;
        Ok(Self::new(offset, <[f32; 3]>::take(rd)?))
    }
}

wire_fields!(DObjRagdoll { began_at_ms, launch, launches });

impl Wire for RigLayout {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.revision.put(wr);
        put_list(wr, &self.models);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let revision = u32::take(rd)?;
        Ok(Self { revision, models: take_list(rd, MODEL_RESERVE)? })
    }
}

wire_tags! {
    AnimGraphRole, "unknown XAnim node tag", {
        AnimGraphRole::Blend => 0,
        AnimGraphRole::Additive => 1,
        AnimGraphRole::Leaf => 2,
    }
}

/// The two cycle counters are small numbers that travel as full words; one that does not fit
/// back into its small form is refused.
fn narrow_cycle(wide: i32, what: &'static str) -> Result<i16, PacketDecodeFault> {
    match i16::try_from(wide) {
        Ok(narrow) => Ok(narrow),
        Err(_) => refuse(what),
    }
}

impl Wire for NodeClock {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.time.put(wr);
        self.prev_t.put(wr);
        i32::from(self.laps).put(wr);
        i32::from(self.prev_laps).put(wr);
        self.blend_left.put(wr);
        self.target_w.put(wr);
        self.weight.put(wr);
        self.rate.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let time = f32::take(rd)?;
        let old_time = f32::take(rd)?;
        let cycle_count_val = narrow_cycle(i32::take(rd)?, "XAnim cycle count overflow")?;
        let old_cycle_count_val = narrow_cycle(i32::take(rd)?, "XAnim old cycle count overflow")?;
        Ok(Self {
            time,
            prev_t: old_time,
            laps: cycle_count_val,
            prev_laps: old_cycle_count_val,
            blend_left: f32::take(rd)?,
            target_w: f32::take(rd)?,
            weight: f32::take(rd)?,
            rate: f32::take(rd)?,
        })
    }
}

impl Wire for GraphNodeRecord {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.parent.map_or(NO_PARENT, |id| id.0).put(wr);
        self.kind.put(wr);
        put_flagged(wr, &self.clip);
        put_flagged(wr, &self.parts);
        self.state.put(wr);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let parent = u16::take(rd)?;
        let kind = AnimGraphRole::take(rd)?;
        let clip = take_flagged(rd, "unknown optional text tag")?;
        let parts = take_flagged(rd, "unknown optional part-bits tag")?;
        let state = NodeClock::take(rd)?;
        Ok(Self {
            parent: (parent != NO_PARENT).then_some(GraphSlot(parent)),
            kind,
            clip,
            parts,
            state,
        })
    }
}

impl Wire for GraphCapture {
    fn put(&self, wr: &mut ByteSinkWriter) {
        self.shape_epoch.put(wr);
        self.clock_epoch.put(wr);
        put_list(wr, &self.nodes);
    }

    fn take(rd: &mut ByteCursorReader<'_>) -> Result<Self, PacketDecodeFault> {
        let def_rev = u32::take(rd)?;
        let st_rev = u32::take(rd)?;
        Ok(Self { shape_epoch: def_rev, clock_epoch: st_rev, nodes: take_list(rd, NODE_RESERVE)? })
    }
}

wire_fields!(RigPoseRecord {
    composition,
    pose_epoch,
    wanted_mask: flag "unknown optional part-bits tag",
    hidden_mask,
    vehicle_steer: flag "unknown optional vehicle steer tag",
    turret_aim: flag "unknown optional turret aim tag",
    tree: flag "unknown optional XAnim tree tag",
    ragdoll: flag "unknown optional ragdoll tag",
    eye_luminance: flag "unknown optional eye luminance tag",
});
