//! The byte-level building blocks of every wire format in this crate: a writer and a reader
//! for little-endian numbers, and the error a reader gives when the bytes are wrong.

mod error;
mod reader;
mod writer;

pub use error::WireError;
pub use reader::WireReader;
pub use writer::WireWriter;
