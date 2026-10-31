//! The 12-byte file preamble and, on signed zones, the auth header that
//! follows it. Everything after these is XChunk records (see `xchunk`).
//!
//! Signed retail PC zones (`TAff0100`) carry a 300-byte auth header:
//! `PHEEBs71`, a u32 of load flags (always zero on retail), the zone name in
//! a NUL-padded 32-byte field and a 256-byte RSA signature. the old engine reads game
//! files it does not own the keys to and never re-signs them, so the
//! signature is kept as bytes and not verified.

pub const SIGNED_ZONE_TAG: &[u8; 8] = b"TAff0100";

pub const UNSIGNED_ZONE_TAG: &[u8; 8] = b"TAffu100";

pub const FF_TAG_SERVER: &[u8; 8] = b"TAsvu100";

pub const T6_PC_ZONE_REV: u32 = 147;

pub const LEADING_BYTES_SIZE: usize = 8 + 4;

pub const AUTH_MAGIC: &[u8; 8] = b"PHEEBs71";

pub const AUTH_NAME_LEN: usize = 32;

pub const AUTH_SIGNATURE_LEN: usize = 256;

pub const AUTH_HEADER_LEN: usize = 8 + 4 + AUTH_NAME_LEN + AUTH_SIGNATURE_LEN;

/// The name seeds the Salsa20 IV table; the game's buffer held 31 characters
/// and a terminator, so a longer name is cut to 31.
pub const ZONE_NAME_KEY_MAX: usize = AUTH_NAME_LEN - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthSealState {
    /// `TAff0100`: auth header, Salsa20-encrypted chunks.
    Signed,
    /// `TAffu100`: no auth header, Salsa20-encrypted chunks keyed by the file name.
    Unsigned,
    /// `TAsvu100`: no auth header, plain chunks.
    UnsignedServer,
}

impl AuthSealState {
    pub const fn is_encrypted(self) -> bool {
        match self { AuthSealState::UnsignedServer => false, _ => true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchivePreamble {
    pub signing: AuthSealState,
    pub version: u32,
    /// The auth header's load flags; `None` on unsigned zones.
    pub auth_flags: Option<u32>,
    name: [u8; AUTH_NAME_LEN],
    name_len: usize,
    /// File offset of the first XChunk record.
    pub body_offset: usize,
}

impl ArchivePreamble {
    /// The zone name stored in the auth header. Unsigned zones carry none;
    /// their chunks are keyed by the file name instead.
    pub fn signed_name(&self) -> Option<&str> {
        if self.signing != AuthSealState::Signed {
            return None;
        }
        core::str::from_utf8(&self.name[..self.name_len]).ok()
    }

    pub fn signature<'a>(&self, file: &'a [u8]) -> Option<&'a [u8]> {
        if self.signing != AuthSealState::Signed {
            return None;
        }
        let start = LEADING_BYTES_SIZE + 8 + 4 + AUTH_NAME_LEN;
        file.get(start..start + AUTH_SIGNATURE_LEN)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeadingBytesFault {
    TooShort { len: usize, need: usize },
    WrongSignature { got: [u8; 8] },
    UnsupportedRevision { got: u32 },
    BadAuthMagic { got: [u8; 8] },
    NameNotTerminated,
    NameNotUtf8,
}

impl core::fmt::Display for LeadingBytesFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LeadingBytesFault::TooShort { len, need } => {
                write!(f, "file is {len} bytes, the T6 header needs {need}")
            }
            LeadingBytesFault::WrongSignature { got } => write!(f, "not a T6 fastfile: magic {got:02x?}"),
            LeadingBytesFault::UnsupportedRevision { got } => {
                write!(f, "T6 zone version {got} (PC is {T6_PC_ZONE_REV})")
            }
            LeadingBytesFault::BadAuthMagic { got } => {
                write!(f, "signed zone without PHEEBs71 auth header: {got:02x?}")
            }
            LeadingBytesFault::NameNotTerminated => {
                write!(f, "auth header zone name has no terminator")
            }
            LeadingBytesFault::NameNotUtf8 => write!(f, "auth header zone name is not UTF-8"),
        }
    }
}

/// True when `bytes` starts like a T6 fastfile of any platform or signing.
pub fn is_t6_magic(bytes: &[u8]) -> bool {
    bytes
        .first_chunk::<8>()
        .is_some_and(|m| [SIGNED_ZONE_TAG, UNSIGNED_ZONE_TAG, FF_TAG_SERVER].contains(&m))
}

fn le_u32(raw4: &[u8]) -> u32 {
    u32::from_le_bytes([raw4[0], raw4[1], raw4[2], raw4[3]])
}

pub fn read_ff_preamble(bytes: &[u8]) -> Result<ArchivePreamble, LeadingBytesFault> {
    let too_short = |need: usize| LeadingBytesFault::TooShort { len: bytes.len(), need };
    let preamble = bytes.get(..LEADING_BYTES_SIZE).ok_or_else(|| too_short(LEADING_BYTES_SIZE))?;
    let (magic_bytes, version_bytes) = preamble.split_at(8);
    let magic: [u8; 8] = magic_bytes.try_into().map_err(|_| too_short(LEADING_BYTES_SIZE))?;
    let signing = [
        (SIGNED_ZONE_TAG, AuthSealState::Signed),
        (UNSIGNED_ZONE_TAG, AuthSealState::Unsigned),
        (FF_TAG_SERVER, AuthSealState::UnsignedServer),
    ]
    .into_iter()
    .find_map(|(known, kind)| (*known == magic).then_some(kind))
    .ok_or(LeadingBytesFault::WrongSignature { got: magic })?;
    let version = match le_u32(version_bytes) {
        T6_PC_ZONE_REV => T6_PC_ZONE_REV,
        got => return Err(LeadingBytesFault::UnsupportedRevision { got }),
    };
    let mut header = ArchivePreamble {
        signing,
        version,
        auth_flags: None,
        name: [0u8; AUTH_NAME_LEN],
        name_len: 0,
        body_offset: LEADING_BYTES_SIZE,
    };
    if signing == AuthSealState::Signed {
        read_auth_block(bytes, &mut header, &too_short)?;
    }
    Ok(header)
}

/// Fills `header` from the signed zone's auth block, which follows the preamble.
/// Where the first zero byte of `bytes` sits, if there is one.
fn memchr_zero(bytes: &[u8]) -> Option<usize> {
    bytes.split(|&byte| byte == 0).next().filter(|head| head.len() < bytes.len()).map(<[u8]>::len)
}

fn read_auth_block(
    file_img: &[u8],
    header: &mut ArchivePreamble,
    too_short: &dyn Fn(usize) -> LeadingBytesFault,
) -> Result<(), LeadingBytesFault> {
    let end = LEADING_BYTES_SIZE + AUTH_HEADER_LEN;
    let auth = file_img.get(LEADING_BYTES_SIZE..end).ok_or_else(|| too_short(end))?;
    if auth[..8] != AUTH_MAGIC[..] {
        return Err(LeadingBytesFault::BadAuthMagic { got: auth[..8].try_into().unwrap() });
    }
    header.name.copy_from_slice(&auth[12..12 + AUTH_NAME_LEN]);
    let name_len = memchr_zero(&header.name).ok_or(LeadingBytesFault::NameNotTerminated)?;
    core::str::from_utf8(&header.name[..name_len]).map_err(|_| LeadingBytesFault::NameNotUtf8)?;
    header.name_len = name_len;
    header.auth_flags = Some(le_u32(&auth[8..]));
    header.body_offset = end;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(name: &[u8]) -> [u8; LEADING_BYTES_SIZE + AUTH_HEADER_LEN] {
        let mut img = [0u8; LEADING_BYTES_SIZE + AUTH_HEADER_LEN];
        img[0..8].copy_from_slice(SIGNED_ZONE_TAG);
        img[8..12].copy_from_slice(&T6_PC_ZONE_REV.to_le_bytes());
        img[12..20].copy_from_slice(AUTH_MAGIC);
        img[24..24 + name.len()].copy_from_slice(name);
        img
    }

    #[test]
    fn signed_header_names_the_zone_and_starts_chunks_at_312() {
        let b = signed(b"zm_nuked");
        let h = read_ff_preamble(&b).unwrap();
        assert_eq!(h.signing, AuthSealState::Signed);
        assert_eq!(h.signed_name(), Some("zm_nuked"));
        assert_eq!(h.auth_flags, Some(0));
        assert_eq!(h.body_offset, 312);
        assert!(h.signing.is_encrypted());
    }

    #[test]
    fn server_zone_has_no_auth_header_and_no_cipher() {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(FF_TAG_SERVER);
        b[8..12].copy_from_slice(&T6_PC_ZONE_REV.to_le_bytes());
        let h = read_ff_preamble(&b).unwrap();
        assert_eq!(h.body_offset, LEADING_BYTES_SIZE);
        assert!(!h.signing.is_encrypted());
        assert_eq!(h.signed_name(), None);
    }

    #[test]
    fn other_versions_and_magics_are_refused() {
        let mut b = signed(b"x");
        b[8..12].copy_from_slice(&146u32.to_le_bytes());
        assert_eq!(
            read_ff_preamble(&b),
            Err(LeadingBytesFault::UnsupportedRevision { got: 146 })
        );
        let mut b = signed(b"x");
        b[0..4].copy_from_slice(b"IWff");
        assert!(matches!(
            read_ff_preamble(&b),
            Err(LeadingBytesFault::WrongSignature { .. })
        ));
    }

    #[test]
    fn a_full_32_byte_name_is_refused() {
        let b = signed(&[b'a'; 32]);
        assert_eq!(
            read_ff_preamble(&b),
            Err(LeadingBytesFault::NameNotTerminated)
        );
    }
}
