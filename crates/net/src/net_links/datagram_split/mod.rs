//! Cutting a packet into datagram-sized fragments and putting it back together.
//!
//! Every fragment starts with a 16-byte header, little-endian:
//! `"RF"`, version, reserved (0), fragment index (u16), fragment count (u16), packet id (u32),
//! whole packet length (u32). All fragments but the last carry the same payload size.

mod header;
mod reassemble;
mod split;

pub use reassemble::FragmentStitcher;
pub use split::DatagramSplitter;

pub const FRAGMENT_HEADER_BYTES: usize = header::BYTES;

/// Why a packet could not be split or a fragment was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitJoinFailure(pub String);

impl core::fmt::Display for SplitJoinFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl std::error::Error for SplitJoinFailure {}

impl From<SplitJoinFailure> for String {
    fn from(error: SplitJoinFailure) -> Self {
        error.0
    }
}

fn err(message: impl Into<String>) -> SplitJoinFailure {
    SplitJoinFailure(message.into())
}

/// The payload that fits in a datagram of `datagram_bytes` after the header; never zero.
fn payload_room(datagram_bytes: usize) -> usize {
    datagram_bytes.saturating_sub(FRAGMENT_HEADER_BYTES).max(1)
}

/// Most fragments one packet may be cut into: the count is a u16.
const SPLIT_PIECE_LIMIT: usize = u16::MAX as usize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_is_what_the_header_leaves_and_at_least_one() {
        assert_eq!(payload_room(1200), 1200 - 16);
        assert_eq!(payload_room(16), 1);
        assert_eq!(payload_room(0), 1);
    }

    #[test]
    fn the_error_is_its_message() {
        let error = err("bad");
        assert_eq!(error.to_string(), "bad");
        assert_eq!(String::from(error), "bad");
    }
}
