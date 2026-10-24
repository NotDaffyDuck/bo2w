//! Bullets against the world: one straight trace, a bullet's whole flight (glass, penetration, a riot shield), and
//! the stored collision frames a shot is rewound to.

mod collider;
mod dobj_state;
mod entity_caps;
mod geometry;
mod history;
mod logs;
mod pierce;
mod poses;
mod posed_coll;
mod sample;
mod tracer;

#[cfg(test)]
mod tests;

pub use collider::{
    AuthorityModelOwner, BulletPath, BulletTraceQuery, BulletTraceSegment, ColliderId, HITLOC_SHIELD,
    SKY_PENETRATION_DEPTH, SURF_SKY, SURF_TYPE_RIOTSHIELD, TraceInvalidReason, TraceOutcome,
};
pub(crate) use collider::is_sky;
pub use dobj_state::{AuthorityDObjState, ScriptModelPlayAnim};
pub use entity_caps::{
    EntityCollisionCapabilities, EntityCollisionEpoch, EntityCollisionTraceGeom, LinkedBrushCollisionBrush,
};
pub use history::{
    CollisionHistory, CurrentAuthorityReason, EntityCollisionFrame, EntityCollisionHistory, EntityCollisionSample,
    HistoryClampReason, HistoryFrame, HistoryPhase, HistoryRefusalReason, HistorySample, HistorySampleVerdict,
    LagcompQuery,
};
pub use pierce::{
    Flight, bullet_trace_segments, bullet_trace_segments_with_entity_models, glass_piece_from_hit,
};
pub(crate) use pierce::bullet_trace_segments_filtered;
pub use posed_coll::{
    AuthorityDObjCollTrace, AuthorityDObjCollision, AuthorityDObjCollisionBone, DObjColl, DObjCollBone, DObjCollSurf,
    RigidXform,
};
pub use poses::{HitVolumeKind, PlayerCollisionPose, ShieldCover};
pub use sample::{ShotSampleProvenance, ShotSampleQuality};
pub use tracer::{bullet_trace, bullet_trace_with_entity_models};
pub(crate) use tracer::coll_tris_clip_available;

pub const CONTENTS_SOLID: u32 = 0x0000_0001;
pub const CONTENTS_BODY: u32 = 0x0200_0000;
pub const MASK_PLAYER_SOLID: u32 = 0x0281_0011;
pub const MASK_SHOT: u32 = 0x0280_6831;
/// What a bullet is stopped by in the static world.
pub const MASK_BULLET_WORLD: u32 = MASK_SHOT;

pub const PLAYER_MINS: [f32; 3] = [-15.0, -15.0, 0.0];
pub const PLAYER_MAXS: [f32; 3] = [15.0, 15.0, 70.0];

/// How many ticks of collision frames are kept.
pub const COLLISION_HISTORY_TICKS: usize = 32;
/// The furthest back in time a shot may be rewound.
pub const LAGCOMP_MAX_REWIND_TICKS: u32 = 4;

/// True when a thing whose contents are `contents` is one the `mask` asks for. A thing with no contents recorded
/// (or none set) matches every mask.
pub fn contents_match_mask(contents: Option<u32>, mask: u32) -> bool {
    match contents {
        None | Some(0) => true,
        Some(bits) => bits & mask != 0,
    }
}
