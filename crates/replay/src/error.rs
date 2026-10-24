//! What can go wrong reading or writing a recording.

use std::io;

use net::{TransportError, WireError};

use crate::file::MAGIC;

#[derive(Debug)]
pub enum ReplayError {
    /// The disk refused, or the file ended inside its header.
    Io(io::Error),
    /// The file does not start with [`MAGIC`].
    Magic { found: [u8; 8] },
    /// Recorded by a build that speaks a different network protocol.
    Version { found: u32, expected: u32 },
    /// A frame in the file does not decode.
    Wire(WireError),
}

impl core::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReplayError::Io(error) => error.fmt(f),
            ReplayError::Magic { found } => write!(
                f,
                "not a bo2mp recording: it starts with {:?}, a recording starts with {:?}",
                String::from_utf8_lossy(found),
                String::from_utf8_lossy(&MAGIC)
            ),
            ReplayError::Version { found, expected } => write!(
                f,
                "recording is protocol {found} but this build speaks {expected}; \
                 frames from one cannot be read as the other"
            ),
            ReplayError::Wire(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReplayError::Io(error) => Some(error),
            ReplayError::Wire(error) => Some(error),
            ReplayError::Magic { .. } | ReplayError::Version { .. } => None,
        }
    }
}

impl From<io::Error> for ReplayError {
    fn from(error: io::Error) -> Self {
        ReplayError::Io(error)
    }
}

impl From<WireError> for ReplayError {
    fn from(error: WireError) -> Self {
        ReplayError::Wire(error)
    }
}

impl From<TransportError> for ReplayError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Io(error) => ReplayError::Io(error),
            TransportError::Wire(error) => ReplayError::Wire(error),
            TransportError::Ended => ReplayError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                error.to_string(),
            )),
        }
    }
}

impl From<ReplayError> for TransportError {
    fn from(error: ReplayError) -> Self {
        match error {
            ReplayError::Io(error) => TransportError::Io(error),
            ReplayError::Wire(error) => TransportError::Wire(error),
            other => TransportError::Io(io::Error::other(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_magic_says_what_the_file_started_with() {
        let text = ReplayError::Magic {
            found: *b"PK\x03\x04junk",
        }
        .to_string();
        assert!(text.starts_with("not a bo2mp recording"), "{text}");
        assert!(text.contains("PK"), "{text}");
        assert!(text.contains("BO2MPDEM"), "{text}");
    }

    #[test]
    fn a_wrong_version_names_both_protocols() {
        let text = ReplayError::Version {
            found: 7,
            expected: 95,
        }
        .to_string();
        assert!(text.contains("protocol 7"), "{text}");
        assert!(text.contains("speaks 95"), "{text}");
    }

    #[test]
    fn io_and_wire_errors_pass_their_own_words_through() {
        let io = ReplayError::Io(io::Error::other("disk on fire"));
        assert_eq!(io.to_string(), "disk on fire");
        let wire = ReplayError::Wire(WireError::Malformed("bad tag"));
        assert_eq!(
            wire.to_string(),
            WireError::Malformed("bad tag").to_string()
        );
    }

    #[test]
    fn only_io_and_wire_have_a_cause() {
        use std::error::Error;
        assert!(ReplayError::Io(io::Error::other("x")).source().is_some());
        assert!(
            ReplayError::Wire(WireError::UnknownField(3))
                .source()
                .is_some()
        );
        assert!(ReplayError::Magic { found: [0; 8] }.source().is_none());
        let version = ReplayError::Version {
            found: 1,
            expected: 2,
        };
        assert!(version.source().is_none());
    }

    #[test]
    fn io_and_wire_errors_keep_their_kind_going_to_a_transport() {
        let io = TransportError::from(ReplayError::Io(io::Error::from(io::ErrorKind::NotFound)));
        assert!(matches!(io, TransportError::Io(e) if e.kind() == io::ErrorKind::NotFound));
        let wire = TransportError::from(ReplayError::Wire(WireError::UnknownField(9)));
        assert!(matches!(
            wire,
            TransportError::Wire(WireError::UnknownField(9))
        ));
    }

    #[test]
    fn the_other_errors_reach_a_transport_as_io_with_their_message() {
        let error = ReplayError::Magic { found: [0; 8] };
        let message = error.to_string();
        match TransportError::from(error) {
            TransportError::Io(io) => assert_eq!(io.to_string(), message),
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_error_comes_back_as_the_same_kind() {
        let wire = ReplayError::from(TransportError::Wire(WireError::Malformed("x")));
        assert!(matches!(wire, ReplayError::Wire(WireError::Malformed("x"))));
        let io = ReplayError::from(TransportError::Io(io::Error::from(io::ErrorKind::TimedOut)));
        assert!(matches!(io, ReplayError::Io(e) if e.kind() == io::ErrorKind::TimedOut));
    }

    #[test]
    fn a_stream_that_ended_is_an_unexpected_eof() {
        let error = ReplayError::from(TransportError::Ended);
        assert!(matches!(error, ReplayError::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof));
    }
}
