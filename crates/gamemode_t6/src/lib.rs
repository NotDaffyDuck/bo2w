#![no_std]
#![forbid(unsafe_code)]

//! BO2 multiplayer game-mode rules. Every rule cites a line `gm-N` of the Streaks facts note
//! facts/gamemode-t6.md (how BO2 behaves, written as facts; this is our own code).

pub mod pain_cues;
pub mod dem;
#[path = "free_for_all.rs"]
pub mod dm;
pub mod domination;
mod body_push;
mod factions;
mod kind;
mod match_flow;
mod blast_damage;
mod sides;
pub mod tdm;

pub use body_push::{SHOVE_SPEED_BASE, ShoveBody, ShovedPair, shove_apart};
pub use pain_cues::{HIT_MARKER_MATERIAL, HIT_FEEDBACK_SOUND};
pub use factions::{
    ATTACKERS, DEFAULT_ALLIES_FACTION, DEFAULT_AXIS_FACTION, DEFENDERS, Faction, PMC, SEALS,
};
pub use kind::RulesetKind;
pub use match_flow::{INTERLUDE_STATE_KEY, WarmupStage, OPENING_USES_START_SPAWNS};
pub use blast_damage::{
    BLAST_BLOCK_MASK, can_damage_player_vis_scale, radius_damage_amount,
    radius_damage_area_half_extent, blast_gap_to_box,
};
pub use sides::{HOSTILE_TINT, FRIENDLY_TINT, Team};
