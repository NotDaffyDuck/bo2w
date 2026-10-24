//! Sixteen-byte identifiers and the content bit set a room asks of its joiners.

use core::{fmt, str::FromStr};

use crate::ProtocolError;

/// A room. The advert in the room list and the room itself share this id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdvertId(pub [u8; 16]);

/// One player connection inside a room, handed out by the service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(pub [u8; 16]);

/// Writes the id as 32 lowercase hex digits.
fn show_hex(raw: &[u8; 16], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    raw.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Reads exactly 32 hex digits (either case). Anything else is `InvalidId`.
fn parse_hex(text: &str) -> Result<[u8; 16], ProtocolError> {
    let digits = text.as_bytes();
    if digits.len() != 32 {
        return Err(ProtocolError::InvalidId);
    }
    let mut raw = [0u8; 16];
    for (slot, pair) in raw.iter_mut().zip(digits.chunks_exact(2)) {
        let (high, low) = hex_digit(pair[0])
            .zip(hex_digit(pair[1]))
            .ok_or(ProtocolError::InvalidId)?;
        *slot = (high << 4) | low;
    }
    Ok(raw)
}

impl fmt::Display for AdvertId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        show_hex(&self.0, f)
    }
}

impl fmt::Display for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        show_hex(&self.0, f)
    }
}

impl FromStr for AdvertId {
    type Err = ProtocolError;

    fn from_str(text: &str) -> Result<Self, ProtocolError> {
        parse_hex(text).map(Self)
    }
}

impl FromStr for MemberId {
    type Err = ProtocolError;

    fn from_str(text: &str) -> Result<Self, ProtocolError> {
        parse_hex(text).map(Self)
    }
}

/// Optional content packs, one bit each. A joiner must hold every bit the room sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ContentFlags(pub u8);

impl ContentFlags {
    /// The bits wanted here that `have` does not hold.
    pub const fn missing_from(self, have: Self) -> Self {
        Self(self.0 & !have.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}
