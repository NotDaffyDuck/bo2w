//! Places where the client or server could not keep an entity's identity straight, recorded
//! instead of silently aliasing two entities onto one slot.

mod cause;
mod kinds;
mod ledger;

pub use cause::LinkDeficiencyOrigin;
pub use kinds::{LinkDeficiency, ScriptEventKind};
pub use ledger::LinkIdentityDeficits;
