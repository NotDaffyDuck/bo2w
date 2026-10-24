#![no_std]
#![forbid(unsafe_code)]

//! BO2 multiplayer game-mode rules. Every rule cites a line `gm-N` of the Streaks facts note
//! facts/gamemode-t6.md (how BO2 behaves, written as facts; this is our own code).

pub mod damage_feedback;
pub mod dem;
#[path = "free_for_all.rs"]
pub mod dm;
pub mod dom;
mod body_push;
mod factions;
mod kind;
mod match_flow;
mod radius_damage;
mod teams;
pub mod tdm;

pub use body_push::{G_PLAYER_COLLISION_EJECT_SPEED_DEFAULT, StuckClient, StuckEject, stuck_in_client};
pub use damage_feedback::{DAMAGE_FEEDBACK_SHADER, HIT_ALERT_ALIAS};
pub use factions::{
    ATTACKERS, DEFAULT_ALLIES_FACTION, DEFAULT_AXIS_FACTION, DEFENDERS, Faction, PMC, SEALS,
};
pub use kind::GameModeKind;
pub use match_flow::{MP_GLOBAL_INTERMISSION, PrematchStep, USE_START_SPAWNS_AT_START};
pub use radius_damage::{
    G_CAN_DAMAGE_CONTENTS_MASK, can_damage_player_vis_scale, radius_damage_amount,
    radius_damage_area_half_extent, radius_damage_distance_to_aabb,
};
pub use teams::{TEAM_COLOR_ENEMY_TEAM, TEAM_COLOR_MY_TEAM, Team};
