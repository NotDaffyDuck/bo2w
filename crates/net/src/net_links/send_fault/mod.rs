//! Why a datagram could not be sent.

use core::fmt;
use std::error::Error;
use std::io;

/// A failed UDP send: the packet was too big to send at all, or the socket refused it.
#[derive(Debug)]
pub enum UdpTransmitFailure {
    /// The encoded packet is longer than the largest one this link sends.
    BeyondSizeLimit { byte_len: usize, datagram_size_ceiling: usize },
    /// The operating system's send failed.
    OsSend(io::Error),
}

impl fmt::Display for UdpTransmitFailure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeyondSizeLimit { byte_len, datagram_size_ceiling } => {
                write!(out, "datagram {byte_len} bytes exceeds max_packet_bytes {datagram_size_ceiling}")
            }
            Self::OsSend(err) => write!(out, "udp send: {err}"),
        }
    }
}

impl Error for UdpTransmitFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::BeyondSizeLimit { .. } => None,
            Self::OsSend(err) => Some(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_packet_names_both_lengths() {
        let err = UdpTransmitFailure::BeyondSizeLimit { byte_len: 1500, datagram_size_ceiling: 1200 };
        assert_eq!(err.to_string(), "datagram 1500 bytes exceeds max_packet_bytes 1200");
        assert!(err.source().is_none());
    }

    #[test]
    fn an_io_failure_is_prefixed_and_keeps_its_cause() {
        let err = UdpTransmitFailure::OsSend(io::Error::other("socket closed"));
        assert_eq!(err.to_string(), "udp send: socket closed");
        assert!(err.source().is_some());
    }
}
