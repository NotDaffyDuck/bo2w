//! Black Ops II's entities: `entityState_s`, `trajectory_t`,
//! `entityType_t` and `entity_event_t`, with BO2's own field order,
//! numbers and trajectory math.
//!
//! Pure data and BO2's small functions on it. No ECS, bevy, or I/O.

#![no_std]
#![forbid(unsafe_code)]

pub mod angles;
pub mod client_state;
pub mod corpse;
pub mod entity_state;
pub mod entity_type;
pub mod event;
pub mod client_event;
pub mod event_ring;
pub mod link;
pub mod missile;
pub mod mover;
pub mod player_angles;
pub mod script_mover;
pub mod tag_link;
pub mod trajectory;

pub use client_state::{ClientState, DisplayName, Score, bg_team_name, team};
pub use entity_state::{
    ClientLinkInfo, EntityState, LerpEntityState, LerpMissile, LerpPlayer, LerpScriptMover,
    LerpUnion, PlayerAnimState, Un2,
};
pub use entity_type::{BO2_MAX_GENTITIES, ENTITYNUM_NONE, ENTITYNUM_WORLD, MAX_GENTITIES, et};
pub use client_event::{
    EntityEventAction, EntityEventKind, LOCAL_SOUND_ENTITY, SequencedEntityEvent, UnsupportedEntityEvent,
    bullet_hit_event, consume_entity_events, entity_event_action, gunner_fire_event, packet_entity_uses_event_ring,
    predicted_weapon_fire_event,
};
pub use event_ring::{BadEvent, EventList, player_state_events, player_to_entity_events};
pub use link::{EntityShared, LinkShape, set_angle, set_origin};
pub use mover::{MoverBase, MoverInfo, adjust_position_for_mover};
pub use player_angles::{AnimClock, ClientAngles, LerpAngles, PlayerAngleDvars, player_angles};
pub use trajectory::{
    Gravity, Trajectory, UnknownTrType, evaluate_trajectory, evaluate_trajectory_delta, tr_type,
};
