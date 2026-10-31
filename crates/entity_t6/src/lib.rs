//! Black Ops II's entities: `entityState_s`, `trajectory_t`,
//! `entityType_t` and `entity_event_t`, with BO2's own field order,
//! numbers and trajectory math.
//!
//! Pure data and BO2's small functions on it. No ECS, bevy, or I/O.

#![no_std]
#![forbid(unsafe_code)]

pub mod angles;
pub mod client_info;
pub mod entity_state;
pub mod entity_type;
pub mod event;
pub mod client_event;
pub mod event_ring;
pub mod link;
pub mod missile;
pub mod mover;
pub mod body_turn;
pub mod script_mover;
pub mod tag_link;
pub mod flight;

pub use client_info::{ScoreRow, bg_team_name, team};
pub use entity_state::{
    ClientLinkInfo, EntitySnapshot, LerpEntityState, LerpMissile, LerpPlayer, LerpScriptMover,
    LerpUnion, PlayerAnimState, Un2,
};
pub use entity_type::{BO2_MAX_GENTITIES, NO_ENTITY_SLOT, ENTITYNUM_WORLD, MAX_GENTITIES, et};
pub use client_event::{
    EntityEventResponse, EventCode, LOCAL_SOUND_ENTITY, OrderedEntityEvent, EntityEventNotHandled,
    bullet_hit_event, drain_entity_events, entity_event_action, gunner_fire_event, packet_entity_uses_event_ring,
    predicted_weapon_fire_event,
};
pub use event_ring::{BadEvent, EventList, player_state_events, player_to_entity_events};
pub use link::{EntityShared, LinkShape, set_angle, set_origin};
pub use mover::{MoverBase, MoverInfo, adjust_position_for_mover};
pub use body_turn::{AnimClock, ClientAngles, LerpAngles, AngleTuning, player_angles};
pub use flight::{
    Gravity, Trajectory, BadMotionKind, evaluate_trajectory, evaluate_trajectory_delta, path_kind,
};
