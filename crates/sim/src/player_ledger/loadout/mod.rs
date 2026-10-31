//! What a player picked to spawn with: the loadout they carry, the class definitions the authority holds, and the
//! classes players build themselves.

mod class;
mod personal;
mod spec;

pub use super::reasons::{LoadoutRefusal, ClassRejectReason, ConfigurationChangeRejectReason, HandoutRefusal, GiveRejectReason};
pub use class::LoadoutClassSpec;
pub use personal::{PERSONAL_CLASS_SLOTS, PersonalClass};
pub use spec::{KitPlan, LoadoutSpec};
