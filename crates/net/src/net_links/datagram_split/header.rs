//! The 16-byte header in front of every fragment.

pub(super) const BYTES: usize = 16;
const FORMAT_TAG: [u8; 2] = *b"RF";
pub(super) const VERSION: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Header {
    pub index: u16,
    pub count: u16,
    pub packet_id: u32,
    pub total_len: u32,
}

/// Why a header could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Too short, or not ours.
    GarbledRequest,
    /// Ours, but another version; carries the version and reserved bytes.
    OtherBuild { version: u8, reserved: u8 },
}

impl Header {
    pub fn write(self, out: &mut Vec<u8>) {
        out.extend_from_slice(&FORMAT_TAG);
        out.extend_from_slice(&[VERSION, 0]);
        out.extend_from_slice(&self.index.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.packet_id.to_le_bytes());
        out.extend_from_slice(&self.total_len.to_le_bytes());
    }

    /// Read the header from the front of `bytes`.
    pub fn read(bytes: &[u8]) -> Result<Self, Refusal> {
        let Some(head) = bytes.first_chunk::<BYTES>() else {
            return Err(Refusal::GarbledRequest);
        };
        if head[..2] != FORMAT_TAG {
            return Err(Refusal::GarbledRequest);
        }
        if head[2] != VERSION || head[3] != 0 {
            return Err(Refusal::OtherBuild {
                version: head[2],
                reserved: head[3],
            });
        }
        let u16_at = |at: usize| u16::from_le_bytes([head[at], head[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
        Ok(Self {
            index: u16_at(4),
            count: u16_at(6),
            packet_id: u32_at(8),
            total_len: u32_at(12),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        Header {
            index: 3,
            count: 9,
            packet_id: 0xA1B2_C3D4,
            total_len: 0x0001_0203,
        }
    }

    #[test]
    fn the_layout_is_fixed() {
        let mut bytes = Vec::new();
        sample().write(&mut bytes);
        assert_eq!(
            bytes,
            [
                b'R', b'F', 2, 0, // magic, version, reserved
                3, 0, 9, 0, // index, count
                0xD4, 0xC3, 0xB2, 0xA1, // packet id
                0x03, 0x02, 0x01, 0x00, // total length
            ]
        );
    }

    #[test]
    fn a_header_reads_back_and_ignores_the_payload_after_it() {
        let mut bytes = Vec::new();
        sample().write(&mut bytes);
        bytes.extend_from_slice(b"payload");
        assert_eq!(Header::read(&bytes), Ok(sample()));
    }

    #[test]
    fn short_or_foreign_bytes_are_malformed() {
        let mut bytes = Vec::new();
        sample().write(&mut bytes);
        assert_eq!(Header::read(&bytes[..15]), Err(Refusal::GarbledRequest));
        bytes[0] = b'X';
        assert_eq!(Header::read(&bytes), Err(Refusal::GarbledRequest));
    }

    #[test]
    fn another_version_or_reserved_byte_names_the_other_build() {
        let mut bytes = Vec::new();
        sample().write(&mut bytes);
        bytes[2] = 3;
        assert_eq!(
            Header::read(&bytes),
            Err(Refusal::OtherBuild { version: 3, reserved: 0 })
        );
        bytes[2] = VERSION;
        bytes[3] = 1;
        assert_eq!(
            Header::read(&bytes),
            Err(Refusal::OtherBuild { version: VERSION, reserved: 1 })
        );
    }
}
