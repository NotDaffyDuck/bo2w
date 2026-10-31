//! What the client does with each kind of state when an authoritative snapshot arrives:
//! which state is adopted, which is replayed, which is left alone, and which entity
//! identities and side effects must not be duplicated.

mod boundary;
mod identity;
mod side_effects;
mod state_owners;

pub use boundary::{CorrectionCutoff, FramePosition, place_of_tick};
pub use identity::{IDENTITY_RULES, IdentityRuleEntry};
pub use side_effects::{AFTEREFFECT_RULES, AftereffectRuleEntry, AftereffectRerunPolicy};
pub use state_owners::{CorrectionStrategy, STATE_RULES, StateCategory, StateRuleEntry};
