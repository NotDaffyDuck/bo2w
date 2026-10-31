//! Black Ops II's client effects system, rebuilt from facts/fx-t6.md.
//!
//! This first part holds BO2's limits, types, flag bits and the pure rules
//! the runtime is built on. Where a fact is missing the code keeps MW2's
//! behaviour, marked `BO2 unknown`, with a line in facts/QUESTIONS.md.
#![forbid(unsafe_code)]

mod beam;
mod bolt;
mod world_hits;
mod def;
mod draw_class;
mod look_kind;
mod flags;
mod laser;
mod pool;
mod late_lighting;
mod random;
mod random_table;
pub mod runtime;
mod sort;
mod update;

pub use beam::*;
pub use bolt::*;
pub use world_hits::*;
pub use def::*;
pub use draw_class::*;
pub use look_kind::*;
pub use flags::*;
pub use laser::*;
pub use pool::*;
pub use late_lighting::*;
pub use random::*;
pub use sort::*;
pub use update::*;
