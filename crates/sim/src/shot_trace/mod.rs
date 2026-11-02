//! Bullets against the world: one straight trace, a bullet's whole flight (glass, walls it goes through, a riot shield), and
//! the stored collision frames a shot is rewound to.

mod collider;
mod puppet_state;
mod entity_caps;
mod ray_boxes;
mod history;
mod logs;
mod pierce;
mod poses;
mod posed_coll;
mod stamp;
mod ray_walk;

#[cfg(test)]
mod tests;

pub use collider::{
    ServerModelSource, ShotRouteKind, ShotRayRequest, ShotPathLeg, HitTargetId, HITLOC_SHIELD,
    SKY_PENETRATION_DEPTH, SURF_SKY, SURF_TYPE_RIOTSHIELD, RayRejectCause, RayResult,
};
pub(crate) use collider::is_sky;
pub use puppet_state::{ServerModelState, ScriptedModelAnimPlay};
pub use entity_caps::{
    EntityHitAbilities, EntityCollisionMoment, EntityRayShape, AttachedSolidShape,
};
pub use history::{
    HitboxTimeline, LiveStateCause, EntityHitboxSnapshot, EntityHitboxTimeline, EntityHitboxReading,
    RewindLimitCause, RewindFrame, RewindStage, RewindDenialCause, RewindSample, RewindSampleRuling,
    RewindRequest,
};
pub use pierce::{
    trace_shot_legs, shot_legs_against_models, pane_from_hit,
};
pub(crate) use pierce::shot_legs_with_filter;
pub use posed_coll::{
    ServerModelHitTrace, ServerModelCollision, DObjColl, DObjCollBone, DObjCollSurf,
    RigidPlacement,
};
pub use poses::{HitboxShapeKind, PlayerHitboxPose, ShieldCover};
pub use stamp::{ShotRecordOrigin, ShotRecordGrade};
pub use ray_walk::{trace_shot, trace_shot_against_models};
pub(crate) use ray_walk::coll_tris_clip_available;

pub const HARD_SURFACE_FLAG: u32 = 1;
pub const SOLID_PLAYER_BODY: u32 = 0x0200_0000;
/// What a living player's movement collides with: BO2's `ClientThink` gives `pmove` 0x2818011 for
/// every movement_mode below 10.
pub const PLAYER_BODY_MASK: u32 = 0x0281_8011;
/// What the dead, spectators and the like collide with: the same mask with the 0x8000 and 0x2000000
/// bits taken out (`ClientThink` leaves them out from movement_mode 10 on).
pub const MASK_PLAYER_DEAD: u32 = 0x0081_0011;
/// What a bullet's world trace is stopped by: the mask `BulletTrace` hands `G_LocationalTrace`.
pub const MASK_SHOT: u32 = 0x0280_e833;
/// What a bullet is stopped by in the static world.
pub const SHOT_WORLD_FILTER: u32 = MASK_SHOT;
/// What a melee swing's traces are stopped by: the mask `Melee_Trace` hands `G_LocationalTraceAllowChildren`. It
/// differs from [`MASK_SHOT`] in one pair of bits (0x80 in, 0x20 out).
pub const MASK_MELEE: u32 = 0x0280_e893;

const PLAYER_HALF_WIDTH: f32 = 15.0;
const PLAYER_HEIGHT: f32 = 70.0;
pub const BODY_BOX_LOWER: [f32; 3] = [-PLAYER_HALF_WIDTH, -PLAYER_HALF_WIDTH, 0.0];
pub const BODY_BOX_UPPER: [f32; 3] = [PLAYER_HALF_WIDTH, PLAYER_HALF_WIDTH, PLAYER_HEIGHT];

/// How many ticks of collision frames are kept.
pub const HITBOX_TIMELINE_LENGTH: usize = 32;
/// The length of one server frame, in ms (BO2 MP runs at 20 frames a second).
pub const SHOT_STEP_MS: u32 = 50;
/// BO2 `G_AntiLagRewindClientPos`: a shot older than this is judged at this many ms back instead.
pub const ANTILAG_MAX_MS: u32 = 400;
/// BO2 `G_AntiLagRewindClientPos`: a shot this many ms back or less is judged against the present.
pub const ANTILAG_SKIP_MS: u32 = 50;
/// The furthest back in time a shot may be rewound.
pub const MAX_COMPENSATION_STEPS: u32 = ANTILAG_MAX_MS / SHOT_STEP_MS;

/// True when a thing whose contents are `contents` is one the `mask` asks for. A thing with no contents recorded
/// (or none set) matches every mask.
pub fn contents_match_mask(contents: Option<u32>, filter: u32) -> bool {
    let Some(bits) = contents else { return true };
    bits == 0 || bits & filter != 0
}
