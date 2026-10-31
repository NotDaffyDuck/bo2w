//! The client's record of each networked entity: the state last sent, the pose it was sampled
//! at, whether it is drawn, and how server-side players and corpses become entity states.

mod convert;
mod draw;
mod pose;

pub use convert::{
    OtherPlayerPoseReading, cadaver_slot_to_object_state, avatar_entity_row, other_player_pose,
};
pub use draw::{PlayerDrawGate, OtherPlayerDrawRoute, peer_avatar_submission_class, remote_body_draws};
pub use pose::{
    EntityModelHandle, EntityEffectHandle, ClientEntityPoseState, BlendedPoseNow, DEAD_ENTITY_FLAG,
    SNAP_MOVE_FLAG, refresh_known, restart_body, tear_down_object,
};
