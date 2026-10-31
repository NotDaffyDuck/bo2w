//! Messages other systems send to ask the audio layer for a sound.

use asset_core::AssetCategoryScope;
use bevy::prelude::*;

/// `source_entity_no` value for sounds that belong to the local player.
pub const SOUND_SOURCE_SELF: u32 = 0;

/// One alias to play, optionally at a world position and tied to a sound entity.
#[derive(Clone, Debug)]
pub struct TriggerCueMsg {
    pub cue_ns: AssetCategoryScope,
    pub alias: String,
    /// Tried when `alias` has nothing playable.
    pub fallback: Option<String>,
    pub position_in_inches: Option<[f32; 3]>,
    pub source_entity_no: Option<u32>,
}

#[derive(Message, Clone, Debug)]
pub enum AliasCommand {
    Play(TriggerCueMsg),
    PlayPitched {
        sound: TriggerCueMsg,
        pitch: f32,
    },
    Stop {
        namespace: AssetCategoryScope,
        alias: String,
        source_entity_no: Option<u32>,
    },
}

#[derive(Message, Clone, Debug)]
pub struct FootfallMsg {
    pub alias: &'static str,
    pub fallback: &'static str,
    pub position_in_inches: Option<[f32; 3]>,
    pub source_entity_no: Option<u32>,
}

#[derive(Message, Clone, Debug)]
pub struct TouchdownNoiseMsg {
    pub alias: &'static str,
    pub fallback: &'static str,
    pub position_in_inches: Option<[f32; 3]>,
    pub source_entity_no: Option<u32>,
}

/// A weapon sound named by alias.
#[derive(Message, Clone, Debug)]
pub struct GunNoiseMsg {
    pub cue_ns: AssetCategoryScope,
    pub alias: String,
    pub position_in_inches: Option<[f32; 3]>,
    pub source_entity_no: Option<u32>,
}

/// A weapon sound named by its slot in a sound bank of a given revision.
#[derive(Message, Clone, Debug)]
pub struct BoundWeaponSound {
    pub catalog_edition: u64,
    pub index: usize,
    pub position_in_inches: Option<[f32; 3]>,
    pub source_entity_no: Option<u32>,
}

/// Notetrack names fired by one weapon's viewmodel animation.
#[derive(Message, Clone, Debug)]
pub struct FirstPersonAnimCues {
    pub generation: frame::LevelGeneration,
    pub client: sim::ClientId,
    pub life: sim::LifeSequence,
    pub cue_wpn: u32,
    pub cue_list: Vec<String>,
}

/// Entity numbers below zero mean "no entity".
pub fn ent_from_number(ent_no: i32) -> Option<u32> {
    u32::try_from(ent_no).ok()
}
