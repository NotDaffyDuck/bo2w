mod recipe;
mod paint;
mod particle_bin;
mod holes;
mod pane;
mod clock;
mod decal;
mod flight;
mod facing;
mod launch;
mod depth_order;
mod birth;
mod host;
mod ribbon;
mod tick;

pub use recipe::{EmitterRecipeSummary, ParticleRecipeSummary};
pub use paint::{
    SwarmCopy, PartDrawInputs, RibbonDrawInputs, RibbonSampleInputs,
    ParticleGlowCopy, CornerBuildOutput, MeshParticleCopy, EffectSpriteInstance, StreakPaintRecipe,
    StreakMeshCopy, StreakSampleLook, build_corners, build_corners_and_streaks,
};
pub use particle_bin::ParticleBinSlot;
pub use holes::{ChildKind, ProceduralMeshPhase, EffectHole, EffectHoleReason, GapBook};
pub use pane::{PaneShardOwner, GlassWorldContact, GlassWorldTrace};
pub use glass_t6::{
    T6_GLASS_LOW_LOD_DIST, T6Glass, T6PaneShape, T6Shard, T6ShardCounts, T6ShardMotion,
    T6ShardVertex,
};
pub use clock::{
    EffectClockMs, THING_FLYING_STREAK, PATH_STRAIGHT_LINE, CLIENT_THING_LIMIT, ClientOnlyThingBank,
    ClientOnlyThingSlot, client_thing_alive, set_show_clock, tracer_flight_ms,
};
pub use decal::{
    DecalOwner, DecalMeshTally, DecalMeshPatch, DecalContactAsk, MarkOutcome,
    DecalAcceptToggle, DecalRayLog,
};
pub use flight::{compute_bouncing_particle_path, compute_particle_path};
pub use launch::{
    EffectStartFrame, EffectStartAsk, PlaybackResult, basis_off_contact_facing, start_attached, start_aligned,
    play_hit_or_death_effect, play_facing,
};
pub use birth::{order_effect_parts, birth_repeat_fraction};
pub use host::{
    NO_RECIPE_LISTING, AttachPointFrame, EffectBoltTarget, LiveEffect, BakedShadeSource,
    SettledAttachFrame, EffectSystemHost, QueuedDecalBirth, QueuedSpawnerBirth, QueuedSound,
    QueuedTrailHit, SpawnRefusal,
};
pub use host::UNTURNED_BASIS;
pub use ribbon::StreakBounceContact;
pub use tick::{
    OffspringSort, OffspringBirthAsk, ParticleMoveAsk, ParticleMoveOutcome, ParticleRayContact,
    EmissionAsk, ContactBirth, QueuedBounceCheck, advance_effects,
};
