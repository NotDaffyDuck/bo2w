//! Voice bookkeeping: how many voices each entity channel holds, and which
//! voices of one alias are playing, so that limits can cut one off.

use std::collections::HashMap;

use asset_core::AssetNamespace;
use bevy::prelude::*;

use crate::start::SuppressReason;

/// Number of entity sound channels.
/// TODO(fact): the real BO2 channel count is not known (see QUESTIONS.md); 64 is a stand-in.
pub(crate) const SND_ENTCHANNEL_MAX: usize = 64;

/// Whether a channel with `voice_count` playing and `loading_streams` still
/// loading has room under `max_voices`. A max of zero or less never has room.
/// TODO(fact): the real BO2 rule is not known; this is a plain count check.
pub(crate) fn has_free_voice(voice_count: i32, loading_streams: i32, max_voices: i32) -> bool {
    max_voices > 0 && voice_count.saturating_add(loading_streams) < max_voices
}

/// How much a higher-priority sound must outrank the weakest playing one
/// before it may replace it.
const PRIORITY_MARGIN: f32 = 3.0;

/// Limit modes read from the alias data.
const LIMIT_NONE: u32 = 0;
const LIMIT_DROP_OLDEST: u32 = 1;
const LIMIT_DROP_QUIETEST: u32 = 3;

#[derive(Component, Clone, Copy, Debug)]
pub struct VoiceLease {
    pub channel: u32,
    pub snd_ent: Option<u32>,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct AliasVoiceLease {
    namespace: AssetNamespace,
    alias: String,
    snd_ent: Option<u32>,
    priority: f32,
    sequence: u64,
}

#[derive(Resource, Debug)]
pub struct VoiceOccupancy {
    counts: [i32; SND_ENTCHANNEL_MAX],
    live: HashMap<Entity, VoiceLease>,
    aliases: HashMap<Entity, AliasVoiceLease>,
    sequence: u64,
}

impl Default for VoiceOccupancy {
    fn default() -> Self {
        Self {
            counts: [0; SND_ENTCHANNEL_MAX],
            live: HashMap::new(),
            aliases: HashMap::new(),
            sequence: 0,
        }
    }
}

impl VoiceOccupancy {
    /// Applies an alias's voice limit before a new voice starts.
    ///
    /// `limit` is `(mode, count)`: once `count` voices of the alias are
    /// playing, mode 1 cuts the oldest and mode 3 cuts the lowest-priority
    /// one, but only if the newcomer outranks it by a clear margin. Any other
    /// mode refuses the newcomer. Returns the voice to cut, if any.
    pub(crate) fn limit_alias(
        &mut self,
        namespace: AssetNamespace,
        alias: &str,
        snd_ent: Option<u32>,
        limit: (u32, u8),
        per_entity: bool,
        priority: f32,
    ) -> Result<Option<Entity>, SuppressReason> {
        let (mode, count) = limit;
        if mode == LIMIT_NONE {
            return Ok(None);
        }

        let same_alias: Vec<(Entity, &AliasVoiceLease)> = self
            .aliases
            .iter()
            .filter(|(_, lease)| {
                lease.namespace == namespace
                    && lease.alias == alias
                    && (!per_entity || lease.snd_ent == snd_ent)
            })
            .map(|(entity, lease)| (*entity, lease))
            .collect();
        if same_alias.len() < usize::from(count) {
            return Ok(None);
        }

        let victim = match mode {
            LIMIT_DROP_OLDEST => same_alias
                .iter()
                .min_by_key(|(_, lease)| lease.sequence)
                .map(|(entity, _)| *entity),
            LIMIT_DROP_QUIETEST => {
                let weakest = same_alias
                    .iter()
                    .min_by(|(_, a), (_, b)| a.priority.total_cmp(&b.priority));
                match weakest {
                    Some((entity, lease)) if priority > lease.priority + PRIORITY_MARGIN => {
                        Some(*entity)
                    }
                    _ => return Err(SuppressReason::VoiceLimit),
                }
            }
            _ => return Err(SuppressReason::VoiceLimit),
        };

        if let Some(entity) = victim {
            self.aliases.remove(&entity);
        }
        Ok(victim)
    }

    pub(crate) fn update_alias_priority(&mut self, entity: Entity, priority: f32) {
        if let Some(lease) = self.aliases.get_mut(&entity) {
            lease.priority = priority;
        }
    }

    /// Records a playing alias voice and hands back its lease so the caller
    /// can put it on the voice entity.
    pub(crate) fn track_alias(
        &mut self,
        entity: Entity,
        namespace: AssetNamespace,
        alias: &str,
        snd_ent: Option<u32>,
        priority: f32,
    ) -> AliasVoiceLease {
        self.sequence += 1;
        let lease = AliasVoiceLease {
            namespace,
            alias: alias.to_owned(),
            snd_ent,
            priority,
            sequence: self.sequence,
        };
        self.aliases.insert(entity, lease.clone());
        lease
    }

    pub fn voice_count(&self, channel: u32) -> i32 {
        self.counts.get(channel as usize).copied().unwrap_or(0)
    }

    pub fn track(&mut self, entity: Entity, lease: VoiceLease) {
        if let Some(count) = self.counts.get_mut(lease.channel as usize) {
            *count = count.saturating_add(1);
        }
        self.live.insert(entity, lease);
    }

    pub fn reclaim(&mut self, entity: Entity) {
        if let Some(lease) = self.live.remove(&entity) {
            self.release_slot(lease.channel);
        }
    }

    /// Removes and returns every voice on `snd_ent`'s `channel`.
    pub fn take_entity_channel(&mut self, snd_ent: u32, channel: u32) -> Vec<Entity> {
        let taken: Vec<Entity> = self
            .live
            .iter()
            .filter(|(_, lease)| lease.snd_ent == Some(snd_ent) && lease.channel == channel)
            .map(|(entity, _)| *entity)
            .collect();
        for entity in &taken {
            self.live.remove(entity);
            self.release_slot(channel);
        }
        taken
    }

    fn release_slot(&mut self, channel: u32) {
        if let Some(count) = self.counts.get_mut(channel as usize) {
            *count = (*count - 1).max(0);
        }
    }
}

pub(crate) fn reclaim_finished_voices(
    mut occupancy: ResMut<VoiceOccupancy>,
    mut gone_leases: RemovedComponents<VoiceLease>,
    mut gone_aliases: RemovedComponents<AliasVoiceLease>,
) {
    for entity in gone_aliases.read() {
        occupancy.aliases.remove(&entity);
    }
    for entity in gone_leases.read() {
        occupancy.reclaim(entity);
    }
}
