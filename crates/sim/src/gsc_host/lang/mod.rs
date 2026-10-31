//! The script language: reading text, linking calls, running functions.
//! Nothing in here knows which game a script belongs to; the engine commands
//! a program may call are handed in as a [`Catalog`] and run through a
//! [`Host`].

mod builtins;
mod eval;
mod fault;
mod parse;
mod program;
mod source;
mod syntax;
mod token;
mod value;

#[cfg(test)]
mod tests;

pub use builtins::{Builtin, Catalog, Namespace};
pub use eval::{ScriptWorld, play_call};
pub use fault::{Fault, Location};
pub use program::{Program, Realm};
pub use source::{SourceResolver, decode_source};
pub use value::{Heap, LEVEL, Value};
