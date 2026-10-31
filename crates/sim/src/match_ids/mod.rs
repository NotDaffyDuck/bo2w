//! The small identifiers the simulation passes around (who died, which shot, which life) and the match's own seeded
//! random number source.

mod ids;
mod rng;

pub use self::ids::ActionStamp;
pub use self::ids::EventStamp;
pub use self::ids::LifeCount;
pub use self::ids::ThrownId;
pub use self::ids::ScriptedModel;
pub use self::ids::{BuckshotId, GameRoundStage, HarmOrigin, BulletNo};
pub use rng::{MatchRandom, RANDOM_STREAM_LAYOUT_VERSION, RandomStreamKind};

/// Older spellings other crates still call; the rename sweep moves them over.
pub use self::EventStamp as EventSequence;
pub use self::LifeCount as LifeSequence;
pub use self::ThrownId as ProjectileId;
pub use self::ScriptedModel as ScriptModelId;
pub use self::BulletNo as ShotId;
