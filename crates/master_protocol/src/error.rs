//! Why a frame could not be read or written.

use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// The bytes stop before the layout does.
    Truncated,
    /// The first four bytes are not `BO2M`.
    BadMagic,
    VersionMismatch { ours: u16, theirs: u16 },
    /// A kind, tag or route byte this version does not know.
    UnknownTag(u8),
    /// A length or count over its cap; carries the offending size.
    LengthExceeded(usize),
    /// The header's body length disagrees with the bytes that follow it.
    LengthMismatch { declared: usize, actual: usize },
    /// A field holds a value the layout forbids, or bytes are left over.
    InvalidBody,
    InvalidUtf8,
    /// Id text that is not 32 hex digits.
    InvalidId,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated => f.write_str("control message is truncated"),
            Self::BadMagic => f.write_str("control message has invalid magic"),
            Self::VersionMismatch { ours, theirs } => {
                write!(f, "protocol version ours={ours} theirs={theirs}")
            }
            Self::UnknownTag(tag) => write!(f, "unknown control tag {tag}"),
            Self::LengthExceeded(len) => write!(f, "control message length {len} exceeds limit"),
            Self::LengthMismatch { declared, actual } => {
                write!(f, "control body length declared={declared} actual={actual}")
            }
            Self::InvalidBody => f.write_str("control message body is invalid"),
            Self::InvalidUtf8 => f.write_str("control string is not UTF-8"),
            Self::InvalidId => f.write_str("identifier is not canonical lowercase hex"),
        }
    }
}

impl std::error::Error for ProtocolError {}
