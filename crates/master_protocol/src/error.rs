//! Why a frame could not be read or written.

use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCodecFault {
    /// The bytes stop before the layout does.
    Truncated,
    /// The first four bytes are not `BO2M`.
    WrongSignature,
    /// The peer speaks another wire revision than this build.
    WireRevisionSkew { local: u16, remote: u16 },
    /// A kind, tag or route byte this version does not know.
    UnrecognizedKindByte(u8),
    /// A length or count over its cap; carries the offending size.
    SizeOverLimit(usize),
    /// The header's body length disagrees with the bytes that follow it.
    BodyLengthMismatch { header: usize, present: usize },
    /// A field holds a value the layout forbids, or bytes are left over.
    MalformedContent,
    /// A text field whose bytes are not UTF-8.
    TextNotUtf8,
    /// Id text that is not 32 hex digits.
    InvalidId,
}

impl fmt::Display for ControlCodecFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Truncated => f.write_str("control message ends early"),
            Self::WrongSignature => f.write_str("control message has the wrong magic"),
            Self::WireRevisionSkew { local, remote } => {
                write!(f, "wire revision mismatch: local={local} remote={remote}")
            }
            Self::UnrecognizedKindByte(tag) => write!(f, "control tag {tag} is not known"),
            Self::SizeOverLimit(len) => write!(f, "control message of {len} bytes is over the limit"),
            Self::BodyLengthMismatch { header, present } => {
                write!(f, "control body length mismatch: header={header} present={present}")
            }
            Self::MalformedContent => f.write_str("control message body is malformed"),
            Self::TextNotUtf8 => f.write_str("control text is not valid UTF-8"),
            Self::InvalidId => f.write_str("id is not 32 lowercase hex digits"),
        }
    }
}

impl std::error::Error for ControlCodecFault {}
