//! The model objects the server builds for script-spawned entities: which models are bolted
//! together, which parts are shown, and the animation tree driving them.

use sim::{AuthorityModelOwner, ScriptModelId};
use xmodel_runtime::{
    DObjCompositionDescriptor, DObjModelDescriptor, DObjSemanticState, HidePartBits, PartBits,
    XAnimNodeId, XAnimNodeState, XAnimSemanticNode, XAnimSemanticNodeKind, XAnimTreeSnapshot,
};

use super::codec::{Decoded, Wire, put_flagged, put_list, refuse, take_flagged, take_list};
use crate::transport::wire::{WireReader, WireWriter};

/// Models in one object / nodes in one tree that are reserved for before the bytes prove them.
const MODEL_RESERVE: usize = 20;
const NODE_RESERVE: usize = 256;

/// A parent that is not there travels as the largest two-byte value.
const NO_PARENT: u16 = u16::MAX;

/// Who a model object belongs to. Only script models have one so far.
impl Wire for AuthorityModelOwner {
    fn put(&self, out: &mut WireWriter) {
        match self {
            Self::ScriptModel(id) => {
                out.put_u8(0);
                id.put(out);
            }
        }
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        match input.get_u8()? {
            0 => ScriptModelId::take(input).map(Self::ScriptModel),
            _ => refuse("unknown DObj owner tag"),
        }
    }
}

impl Wire for PartBits {
    fn put(&self, out: &mut WireWriter) {
        <[u32; Self::WORDS] as Wire>::put(self.words(), out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        <[u32; Self::WORDS] as Wire>::take(input).map(Self::from_words)
    }
}

impl Wire for HidePartBits {
    fn put(&self, out: &mut WireWriter) {
        <[u32; Self::WORDS] as Wire>::put(self.words(), out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        <[u32; Self::WORDS] as Wire>::take(input).map(Self::from_words)
    }
}

impl Wire for DObjModelDescriptor {
    fn put(&self, out: &mut WireWriter) {
        self.model.put(out);
        self.parent_model.unwrap_or(NO_PARENT).put(out);
        put_flagged(out, &self.attach_tag);
        self.ignore_collision.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let model = String::take(input)?;
        let parent = u16::take(input)?;
        let attach_tag = take_flagged(input, "unknown optional text tag")?;
        let ignore_collision = bool::take(input)?;
        Ok(Self {
            model,
            parent_model: (parent != NO_PARENT).then_some(parent),
            attach_tag,
            ignore_collision,
        })
    }
}

impl Wire for DObjCompositionDescriptor {
    fn put(&self, out: &mut WireWriter) {
        self.revision.put(out);
        put_list(out, &self.models);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let revision = u32::take(input)?;
        Ok(Self { revision, models: take_list(input, MODEL_RESERVE)? })
    }
}

wire_tags! {
    XAnimSemanticNodeKind, "unknown XAnim node tag", {
        XAnimSemanticNodeKind::Blend => 0,
        XAnimSemanticNodeKind::Additive => 1,
        XAnimSemanticNodeKind::Leaf => 2,
    }
}

/// The two cycle counters are small numbers that travel as full words; one that does not fit
/// back into its small form is refused.
fn narrow_cycle(wide: i32, what: &'static str) -> Decoded<i16> {
    match i16::try_from(wide) {
        Ok(narrow) => Ok(narrow),
        Err(_) => refuse(what),
    }
}

impl Wire for XAnimNodeState {
    fn put(&self, out: &mut WireWriter) {
        self.time.put(out);
        self.old_time.put(out);
        i32::from(self.cycle_count).put(out);
        i32::from(self.old_cycle_count).put(out);
        self.goal_time.put(out);
        self.goal_weight.put(out);
        self.weight.put(out);
        self.rate.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let time = f32::take(input)?;
        let old_time = f32::take(input)?;
        let cycle_count = narrow_cycle(i32::take(input)?, "XAnim cycle count overflow")?;
        let old_cycle_count = narrow_cycle(i32::take(input)?, "XAnim old cycle count overflow")?;
        Ok(Self {
            time,
            old_time,
            cycle_count,
            old_cycle_count,
            goal_time: f32::take(input)?,
            goal_weight: f32::take(input)?,
            weight: f32::take(input)?,
            rate: f32::take(input)?,
        })
    }
}

impl Wire for XAnimSemanticNode {
    fn put(&self, out: &mut WireWriter) {
        self.parent.map_or(NO_PARENT, |id| id.0).put(out);
        self.kind.put(out);
        put_flagged(out, &self.clip);
        put_flagged(out, &self.parts);
        self.state.put(out);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let parent = u16::take(input)?;
        let kind = XAnimSemanticNodeKind::take(input)?;
        let clip = take_flagged(input, "unknown optional text tag")?;
        let parts = take_flagged(input, "unknown optional part-bits tag")?;
        let state = XAnimNodeState::take(input)?;
        Ok(Self {
            parent: (parent != NO_PARENT).then_some(XAnimNodeId(parent)),
            kind,
            clip,
            parts,
            state,
        })
    }
}

impl Wire for XAnimTreeSnapshot {
    fn put(&self, out: &mut WireWriter) {
        self.definition_revision.put(out);
        self.state_revision.put(out);
        put_list(out, &self.nodes);
    }

    fn take(input: &mut WireReader<'_>) -> Decoded<Self> {
        let definition_revision = u32::take(input)?;
        let state_revision = u32::take(input)?;
        Ok(Self { definition_revision, state_revision, nodes: take_list(input, NODE_RESERVE)? })
    }
}

wire_fields!(DObjSemanticState {
    composition,
    pose_revision,
    requested_parts: flag "unknown optional part-bits tag",
    hide_part_bits,
    vehicle_steer: flag "unknown optional vehicle steer tag",
    turret_aim: flag "unknown optional turret aim tag",
    tree: flag "unknown optional XAnim tree tag",
});
