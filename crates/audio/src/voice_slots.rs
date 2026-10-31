//! PlayingCue bookkeeping: how many voices each entity channel holds, and which
//! voices of one alias are playing, so that limits can cut one off.

use std::collections::HashMap;

use asset_core::AssetCategoryScope;
use bevy::prelude::*;

use crate::playback_gate::MuteCause;

/// Number of entity sound channels.
/// TODO(fact): the real BO2 channel count is not known (see QUESTIONS.md); 64 is a stand-in.
pub(crate) const ENT_CHANNEL_CAP: usize = 64;

/// Whether a channel with `voice_count` playing and `loading_streams` still
/// loading has room under `simultaneous_channel_cap`. A max of zero or less never has room.
/// TODO(fact): the real BO2 rule is not known; this is a plain count check.
pub(crate) fn has_free_voice(voice_count: i32, streams_loading: i32, simultaneous_channel_cap: i32) -> bool {
    simultaneous_channel_cap > 0 && voice_count.saturating_add(streams_loading) < simultaneous_channel_cap
}

/// How much a higher-priority sound must outrank the weakest playing one
/// before it may replace it.
const PRIORITY_MARGIN: f32 = 3.0;

/// Limit modes read from the alias data.
const LIMIT_NONE: u32 = 0;
const LIMIT_DROP_OLDEST: u32 = 1;
const LIMIT_DROP_QUIETEST: u32 = 3;

#[derive(Component, Clone, Copy, Debug)]
pub struct ChannelBooking {
    pub channel: u32,
    pub source_entity_no: Option<u32>,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct AliasVoiceLease {
    seq_no: u64,
    namespace: AssetCategoryScope,
    name: String,
    source_entity_no: Option<u32>,
    priority: f32,
}

#[derive(Resource, Debug)]
pub struct ChannelUsageTally {
    seq_ix: u64,
    aliases: HashMap<Entity, AliasVoiceLease>,
    leases: HashMap<Entity, ChannelBooking>,
    held: Vec<i32>,
}

impl Default for ChannelUsageTally {
    fn default() -> Self {
        Self {
            seq_ix: 0,
            aliases: HashMap::default(),
            leases: HashMap::default(),
            held: vec![0; ENT_CHANNEL_CAP],
        }
    }
}

impl ChannelUsageTally {
    /// Applies an alias's voice limit before a new voice starts.
    ///
    /// `limit` is `(mode, count)`: once `count` voices of the alias are
    /// playing, mode 1 cuts the oldest and mode 3 cuts the lowest-priority
    /// one, but only if the newcomer outranks it by a clear margin. Any other
    /// mode refuses the newcomer. Returns the voice to cut, if any.
    pub(crate) fn limit_alias(
        &mut self,
        namespace: AssetCategoryScope,
        cue_id: &str,
        source_entity_no: Option<u32>,
        limit: (u32, u8),
        per_entity: bool,
        priority: f32,
    ) -> Result<Option<Entity>, MuteCause> {
        let (mode, count) = limit;
        if mode == LIMIT_NONE {
            return Ok(None);
        }

        let same_alias: Vec<(Entity, &AliasVoiceLease)> = self
            .aliases
            .iter()
            .filter(|(_, lease)| {
                lease.namespace == namespace
                    && lease.name == cue_id
                    && (!per_entity || lease.source_entity_no == source_entity_no)
            })
            .map(|(ent, lease)| (*ent, lease))
            .collect();
        if same_alias.len() < usize::from(count) {
            return Ok(None);
        }

        let victim = match mode {
            LIMIT_DROP_OLDEST => same_alias
                .iter()
                .min_by_key(|(_, lease)| lease.seq_no)
                .map(|(ent_id, _)| *ent_id),
            LIMIT_DROP_QUIETEST => {
                let weakest = same_alias
                    .iter()
                    .min_by(|(_, a), (_, b)| a.priority.total_cmp(&b.priority));
                match weakest {
                    Some((entity, lease)) if priority > lease.priority + PRIORITY_MARGIN => {
                        Some(*entity)
                    }
                    _ => return Err(MuteCause::AtVoiceCap),
                }
            }
            _ => return Err(MuteCause::AtVoiceCap),
        };

        if let Some(entity) = victim {
            self.aliases.remove(&entity);
        }
        Ok(victim)
    }

    pub(crate) fn update_alias_priority(&mut self, ent: Entity, priority: f32) {
        if let Some(lease) = self.aliases.get_mut(&ent) {
            lease.priority = priority;
        }
    }

    /// Records a playing alias voice and hands back its lease so the caller
    /// can put it on the voice entity.
    pub(crate) fn track_alias(
        &mut self,
        ent: Entity,
        namespace: AssetCategoryScope,
        cue_id: &str,
        source_entity_no: Option<u32>,
        priority: f32,
    ) -> AliasVoiceLease {
        self.seq_ix += 1;
        let lease = AliasVoiceLease {
            namespace,
            name: cue_id.to_owned(),
            source_entity_no,
            priority,
            seq_no: self.seq_ix,
        };
        self.aliases.insert(ent, lease.clone());
        lease
    }

    pub fn voices_on(&self, chan: u32) -> i32 {
        usize::try_from(chan)
            .ok()
            .and_then(|at| self.held.get(at))
            .map_or(0, |held| *held)
    }

    pub fn hold_entity(&mut self, ent: Entity, lease: ChannelBooking) {
        self.adjust(lease.channel, 1);
        self.leases.insert(ent, lease);
    }

    pub fn drop_lease(&mut self, ent: Entity) {
        if let Some(ChannelBooking { channel, .. }) = self.leases.remove(&ent) {
            self.adjust(channel, -1);
        }
    }

    /// Removes and returns every voice on `source_entity_no`'s `channel`.
    pub fn take_ent_channel(&mut self, source_entity_no: u32, chan: u32) -> Vec<Entity> {
        let (taken, kept): (HashMap<_, _>, HashMap<_, _>) = std::mem::take(&mut self.leases)
            .into_iter()
            .partition(|(_, lease)| lease.source_entity_no == Some(source_entity_no) && lease.channel == chan);
        self.leases = kept;
        taken
            .into_keys()
            .inspect(|_| self.adjust(chan, -1))
            .collect()
    }

    /// Moves a channel's count by `delta`; it never goes below zero.
    fn adjust(&mut self, chan: u32, delta: i32) {
        let slot = usize::try_from(chan)
            .ok()
            .and_then(|at| self.held.get_mut(at));
        if let Some(held) = slot {
            *held = held.saturating_add(delta).max(0);
        }
    }
}

pub(crate) fn free_spent_channels(
    mut occupancy: ResMut<ChannelUsageTally>,
    mut gone_leases: RemovedComponents<ChannelBooking>,
    mut gone_aliases: RemovedComponents<AliasVoiceLease>,
) {
    gone_aliases.read().for_each(|entity| {
        occupancy.aliases.remove(&entity);
    });
    gone_leases.read().for_each(|entity| occupancy.drop_lease(entity));
}
