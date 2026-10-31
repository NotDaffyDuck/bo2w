//! Sixteen-byte identifiers and the content bit set a room asks of its joiners.

use core::fmt::{self, Formatter};
use core::str;

use crate::ControlCodecFault;
use core::str::FromStr;

/// A room. The advert in the room list and the room itself share this id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListingId(pub [u8; 16]);

/// One player connection inside a room, handed out by the service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoomOccupantId(pub [u8; 16]);

/// Writes the id as 32 lowercase hex digits.
fn show_hex(raw: &[u8; 16], out: &mut Formatter<'_>) -> fmt::Result {
    raw.iter().try_for_each(|byte| write!(out, "{byte:02x}"))
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
fn parse_hex(text: &str) -> Result<[u8; 16], ControlCodecFault> {
    let digits = text.as_bytes();
    if digits.len() != 32 {
        return Err(ControlCodecFault::InvalidId);
    }
    let mut raw = [0u8; 16];
    for (slot, pair) in raw.iter_mut().zip(digits.chunks_exact(2)) {
        let (high, low) = hex_digit(pair[0])
            .zip(hex_digit(pair[1]))
            .ok_or(ControlCodecFault::InvalidId)?;
        *slot = (high << 4) | low;
    }
    Ok(raw)
}

impl fmt::Display for ListingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        show_hex(&self.0, f)
    }
}

impl fmt::Display for RoomOccupantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        show_hex(&self.0, f)
    }
}

impl FromStr for ListingId {
    type Err = ControlCodecFault;

    fn from_str(text: &str) -> Result<Self, ControlCodecFault> {
        parse_hex(text).map(Self)
    }
}

impl FromStr for RoomOccupantId {
    type Err = ControlCodecFault;

    fn from_str(text: &str) -> Result<Self, ControlCodecFault> {
        parse_hex(text).map(Self)
    }
}

/// Optional content packs, one bit each. A joiner must hold every bit the room sets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AssetCoverageBits(pub u8);

impl AssetCoverageBits {
    /// The bits wanted here that `held` lacks.
    pub const fn lacking_in(self, held: Self) -> Self {
        Self(self.0 & !held.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}
