//! What can go wrong reading or writing a recording.

use std::io;
use std::io::Error;

use net::{LinkIoFailure, PacketDecodeFault};

use crate::file::FORMAT_TAG;

#[derive(Debug)]
pub enum RecordingFault {
    /// The disk refused, or the file ended inside its header.
    Io(Error),
    /// The file does not start with [`FORMAT_TAG`].
    Magic { found: [u8; 8] },
    /// Recorded by a build that speaks a different network protocol.
    Version { found: u32, wanted: u32 },
    /// A frame in the file does not decode.
    Wire(PacketDecodeFault),
}

impl core::fmt::Display for RecordingFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecordingFault::Io(error) => error.fmt(f),
            RecordingFault::Magic { found: opening } => write!(
                f,
                "not a bo2mp recording: it starts with {:?}, a recording starts with {:?}",
                String::from_utf8_lossy(opening),
                String::from_utf8_lossy(&FORMAT_TAG)
            ),
            Self::Version { found, wanted } => write!(
                f,
                "recording is protocol {found} but this build speaks {wanted}; \
                 frames from one cannot be read as the other"
            ),
            RecordingFault::Wire(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecordingFault {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            RecordingFault::Io(error) => Some(error),
            RecordingFault::Wire(error) => Some(error),
            RecordingFault::Magic { .. } | RecordingFault::Version { .. } => None,
        }
    }
}

impl From<io::Error> for RecordingFault {
    fn from(error: io::Error) -> Self {
        RecordingFault::Io(error)
    }
}

impl From<PacketDecodeFault> for RecordingFault {
    fn from(error: PacketDecodeFault) -> Self {
        RecordingFault::Wire(error)
    }
}

impl From<LinkIoFailure> for RecordingFault {
    fn from(error: LinkIoFailure) -> Self {
        match error {
            LinkIoFailure::Io(error) => RecordingFault::Io(error),
            LinkIoFailure::Wire(error) => RecordingFault::Wire(error),
            LinkIoFailure::Ended => RecordingFault::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                error.to_string(),
            )),
        }
    }
}

impl From<RecordingFault> for LinkIoFailure {
    fn from(error: RecordingFault) -> Self {
        match error {
            RecordingFault::Io(error) => LinkIoFailure::Io(error),
            RecordingFault::Wire(error) => LinkIoFailure::Wire(error),
            other => LinkIoFailure::Io(io::Error::other(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_magic_says_what_the_file_started_with() {
        let text = RecordingFault::Magic {
            found: *b"PK\x03\x04junk",
        }
        .to_string();
        assert!(text.starts_with("not a bo2mp recording"), "{text}");
        assert!(text.contains("PK"), "{text}");
        assert!(text.contains("BO2MPDEM"), "{text}");
    }

    #[test]
    fn a_wrong_version_names_both_protocols() {
        let text = RecordingFault::Version {
            found: 7,
            wanted: 95,
        }
        .to_string();
        assert!(text.contains("protocol 7"), "{text}");
        assert!(text.contains("speaks 95"), "{text}");
    }

    #[test]
    fn io_and_wire_errors_pass_their_own_words_through() {
        let io = RecordingFault::Io(io::Error::other("disk on fire"));
        assert_eq!(io.to_string(), "disk on fire");
        let wire = RecordingFault::Wire(PacketDecodeFault::GarbledRequest("bad tag"));
        assert_eq!(
            wire.to_string(),
            PacketDecodeFault::GarbledRequest("bad tag").to_string()
        );
    }

    #[test]
    fn only_io_and_wire_have_a_cause() {
        use std::error::Error;
        assert!(RecordingFault::Io(io::Error::other("x")).source().is_some());
        assert!(
            RecordingFault::Wire(PacketDecodeFault::UnlistedField(3))
                .source()
                .is_some()
        );
        assert!(RecordingFault::Magic { found: [0; 8] }.source().is_none());
        let version = RecordingFault::Version {
            found: 1,
            wanted: 2,
        };
        assert!(version.source().is_none());
    }

    #[test]
    fn io_and_wire_errors_keep_their_kind_going_to_a_transport() {
        let io = LinkIoFailure::from(RecordingFault::Io(io::Error::from(io::ErrorKind::NotFound)));
        assert!(matches!(io, LinkIoFailure::Io(e) if e.kind() == io::ErrorKind::NotFound));
        let wire = LinkIoFailure::from(RecordingFault::Wire(PacketDecodeFault::UnlistedField(9)));
        assert!(matches!(
            wire,
            LinkIoFailure::Wire(PacketDecodeFault::UnlistedField(9))
        ));
    }

    #[test]
    fn the_other_errors_reach_a_transport_as_io_with_their_message() {
        let error = RecordingFault::Magic { found: [0; 8] };
        let expected_text = error.to_string();
        match LinkIoFailure::from(error) {
            LinkIoFailure::Io(io) => assert_eq!(io.to_string(), expected_text),
            other => panic!("expected an io error, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_error_comes_back_as_the_same_kind() {
        let wire = RecordingFault::from(LinkIoFailure::Wire(PacketDecodeFault::GarbledRequest("x")));
        assert!(matches!(wire, RecordingFault::Wire(PacketDecodeFault::GarbledRequest("x"))));
        let io = RecordingFault::from(LinkIoFailure::Io(io::Error::from(io::ErrorKind::TimedOut)));
        assert!(matches!(io, RecordingFault::Io(e) if e.kind() == io::ErrorKind::TimedOut));
    }

    #[test]
    fn a_stream_that_ended_is_an_unexpected_eof() {
        let error = RecordingFault::from(LinkIoFailure::Ended);
        assert!(matches!(error, RecordingFault::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof));
    }
}
