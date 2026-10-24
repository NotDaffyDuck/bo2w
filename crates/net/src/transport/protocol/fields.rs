//! The byte layout of the small structures packets are built from. Words are little-endian
//! through `WireWriter`; a 64-bit value travels as two words, the high one first.

use super::handshake::{ContentFingerprint, HandshakeHello, HandshakeReject, ProtocolLimits};
use super::identity::{ConnectionId, PacketHeader};
use crate::transport::wire::{WireError, WireReader, WireWriter};

pub(super) fn write_u64(out: &mut WireWriter, value: u64) {
    out.put_u32((value >> 32) as u32);
    out.put_u32(value as u32);
}

pub(super) fn read_u64(input: &mut WireReader<'_>) -> Result<u64, WireError> {
    let high = u64::from(input.get_u32()?);
    let low = u64::from(input.get_u32()?);
    Ok(high << 32 | low)
}

pub(super) fn write_connection(out: &mut WireWriter, connection: ConnectionId) {
    write_u64(out, connection.0);
}

pub(super) fn read_connection(input: &mut WireReader<'_>) -> Result<ConnectionId, WireError> {
    read_u64(input).map(ConnectionId)
}

pub(super) fn write_header(out: &mut WireWriter, header: &PacketHeader) {
    write_connection(out, header.connection);
    out.put_u32(header.sequence);
    out.put_u32(header.ack);
    out.put_u32(header.epoch);
}

pub(super) fn read_header(input: &mut WireReader<'_>) -> Result<PacketHeader, WireError> {
    Ok(PacketHeader {
        connection: read_connection(input)?,
        sequence: input.get_u32()?,
        ack: input.get_u32()?,
        epoch: input.get_u32()?,
    })
}

pub(super) fn write_hello(out: &mut WireWriter, hello: &HandshakeHello) {
    let ContentFingerprint {
        gameplay,
        map,
        models,
        weapons,
        classes,
    } = hello.content;
    out.put_u32(hello.protocol_version);
    for digest in [gameplay, map, models, weapons, classes] {
        write_u64(out, digest);
    }
    out.put_u32(hello.limits.authority_hz);
    out.put_u16(hello.limits.max_cmds_per_tick);
    out.put_u16(hello.limits.max_actions_per_tick);
    out.put_u32(hello.limits.max_packet_bytes);
}

pub(super) fn read_hello(input: &mut WireReader<'_>) -> Result<HandshakeHello, WireError> {
    let protocol_version = input.get_u32()?;
    let content = ContentFingerprint {
        gameplay: read_u64(input)?,
        map: read_u64(input)?,
        models: read_u64(input)?,
        weapons: read_u64(input)?,
        classes: read_u64(input)?,
    };
    let limits = ProtocolLimits {
        authority_hz: input.get_u32()?,
        max_cmds_per_tick: input.get_u16()?,
        max_actions_per_tick: input.get_u16()?,
        max_packet_bytes: input.get_u32()?,
    };
    Ok(HandshakeHello {
        protocol_version,
        content,
        limits,
    })
}

const REJECT_PROTOCOL_VERSION: u8 = 1;
const REJECT_MAP_CONTENT: u8 = 2;
const REJECT_WEAPON_REGISTRY: u8 = 3;
const REJECT_CLASS_REGISTRY: u8 = 4;
const REJECT_TICK_RATE: u8 = 5;
const REJECT_LIMITS: u8 = 6;

pub(super) fn write_reject(out: &mut WireWriter, reject: HandshakeReject) {
    let digests = |out: &mut WireWriter, tag: u8, ours: u64, theirs: u64| {
        out.put_u8(tag);
        write_u64(out, ours);
        write_u64(out, theirs);
    };
    match reject {
        HandshakeReject::ProtocolVersion { ours, theirs } => {
            out.put_u8(REJECT_PROTOCOL_VERSION);
            out.put_u32(ours);
            out.put_u32(theirs);
        }
        HandshakeReject::MapContent { ours, theirs } => {
            digests(out, REJECT_MAP_CONTENT, ours, theirs)
        }
        HandshakeReject::WeaponRegistry { ours, theirs } => {
            digests(out, REJECT_WEAPON_REGISTRY, ours, theirs)
        }
        HandshakeReject::ClassRegistry { ours, theirs } => {
            digests(out, REJECT_CLASS_REGISTRY, ours, theirs)
        }
        HandshakeReject::TickRate { ours, theirs } => {
            out.put_u8(REJECT_TICK_RATE);
            out.put_u32(ours);
            out.put_u32(theirs);
        }
        HandshakeReject::Limits => out.put_u8(REJECT_LIMITS),
    }
}

pub(super) fn read_reject(input: &mut WireReader<'_>) -> Result<HandshakeReject, WireError> {
    let tag = input.get_u8()?;
    let reject = match tag {
        REJECT_PROTOCOL_VERSION => HandshakeReject::ProtocolVersion {
            ours: input.get_u32()?,
            theirs: input.get_u32()?,
        },
        REJECT_MAP_CONTENT => HandshakeReject::MapContent {
            ours: read_u64(input)?,
            theirs: read_u64(input)?,
        },
        REJECT_WEAPON_REGISTRY => HandshakeReject::WeaponRegistry {
            ours: read_u64(input)?,
            theirs: read_u64(input)?,
        },
        REJECT_CLASS_REGISTRY => HandshakeReject::ClassRegistry {
            ours: read_u64(input)?,
            theirs: read_u64(input)?,
        },
        REJECT_TICK_RATE => HandshakeReject::TickRate {
            ours: input.get_u32()?,
            theirs: input.get_u32()?,
        },
        REJECT_LIMITS => HandshakeReject::Limits,
        _ => return Err(WireError::Malformed("unknown HandshakeReject tag")),
    };
    Ok(reject)
}

const SAMPLE_NONE: u8 = 0;
const SAMPLE_EXACT: u8 = 1;
const SAMPLE_INTERPOLATED: u8 = 2;
const SAMPLE_HELD: u8 = 3;
const SAMPLE_STARVED: u8 = 4;

pub(super) fn write_shot_sample(out: &mut WireWriter, sample: &sim::ShotSampleProvenance) {
    use sim::ShotSampleQuality as Quality;
    out.put_u32(sample.left.0);
    out.put_u32(sample.right.0);
    out.put_u32(sample.alpha.to_bits());
    out.put_u8(match sample.quality {
        Quality::None => SAMPLE_NONE,
        Quality::Exact => SAMPLE_EXACT,
        Quality::Interpolated => SAMPLE_INTERPOLATED,
        Quality::Held => SAMPLE_HELD,
        Quality::Starved => SAMPLE_STARVED,
    });
}

pub(super) fn read_shot_sample(
    input: &mut WireReader<'_>,
) -> Result<sim::ShotSampleProvenance, WireError> {
    use sim::ShotSampleQuality as Quality;
    let left = sim::Tick(input.get_u32()?);
    let right = sim::Tick(input.get_u32()?);
    let alpha = f32::from_bits(input.get_u32()?);
    let quality = match input.get_u8()? {
        SAMPLE_NONE => Quality::None,
        SAMPLE_EXACT => Quality::Exact,
        SAMPLE_INTERPOLATED => Quality::Interpolated,
        SAMPLE_HELD => Quality::Held,
        SAMPLE_STARVED => Quality::Starved,
        _ => return Err(WireError::Malformed("unknown ShotSampleQuality tag")),
    };
    Ok(sim::ShotSampleProvenance {
        left,
        right,
        alpha,
        quality,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(fill: impl FnOnce(&mut WireWriter)) -> Vec<u8> {
        let mut out = WireWriter::new();
        fill(&mut out);
        out.finish()
    }

    #[test]
    fn a_64_bit_value_travels_high_word_first() {
        let bytes = written(|out| write_u64(out, 0x0102_0304_0506_0708));
        assert_eq!(bytes, [4, 3, 2, 1, 8, 7, 6, 5]);
        assert_eq!(
            read_u64(&mut WireReader::new(&bytes)).unwrap(),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn a_header_is_twenty_bytes_and_round_trips() {
        let header = PacketHeader {
            connection: ConnectionId(0xAABB_CCDD_0000_0001),
            sequence: 9,
            ack: 8,
            epoch: 7,
        };
        let bytes = written(|out| write_header(out, &header));
        assert_eq!(bytes.len(), 20);
        assert_eq!(read_header(&mut WireReader::new(&bytes)).unwrap(), header);
    }

    #[test]
    fn a_hello_round_trips() {
        let hello = HandshakeHello {
            protocol_version: 11,
            content: ContentFingerprint {
                gameplay: 1,
                map: 2,
                models: 3,
                weapons: 4,
                classes: 5,
            },
            limits: ProtocolLimits {
                authority_hz: 60,
                max_cmds_per_tick: 16,
                max_actions_per_tick: 32,
                max_packet_bytes: 1000,
            },
        };
        let bytes = written(|out| write_hello(out, &hello));
        assert_eq!(bytes.len(), 4 + 5 * 8 + 4 + 2 + 2 + 4);
        assert_eq!(read_hello(&mut WireReader::new(&bytes)).unwrap(), hello);
    }

    #[test]
    fn every_reject_round_trips_and_an_unknown_tag_is_refused() {
        for reject in [
            HandshakeReject::ProtocolVersion { ours: 1, theirs: 2 },
            HandshakeReject::MapContent { ours: 3, theirs: 4 },
            HandshakeReject::WeaponRegistry { ours: 5, theirs: 6 },
            HandshakeReject::ClassRegistry { ours: 7, theirs: 8 },
            HandshakeReject::TickRate { ours: 9, theirs: 10 },
            HandshakeReject::Limits,
        ] {
            let bytes = written(|out| write_reject(out, reject));
            assert_eq!(read_reject(&mut WireReader::new(&bytes)).unwrap(), reject);
        }
        assert!(read_reject(&mut WireReader::new(&[0])).is_err());
        assert!(read_reject(&mut WireReader::new(&[7])).is_err());
    }

    #[test]
    fn every_shot_sample_quality_round_trips() {
        use sim::ShotSampleQuality as Quality;
        for quality in [
            Quality::None,
            Quality::Exact,
            Quality::Interpolated,
            Quality::Held,
            Quality::Starved,
        ] {
            let sample = sim::ShotSampleProvenance {
                left: sim::Tick(4),
                right: sim::Tick(5),
                alpha: 0.25,
                quality,
            };
            let bytes = written(|out| write_shot_sample(out, &sample));
            assert_eq!(bytes.len(), 13);
            assert_eq!(
                read_shot_sample(&mut WireReader::new(&bytes)).unwrap(),
                sample
            );
        }
    }

    #[test]
    fn an_unknown_quality_tag_is_refused() {
        let mut bytes = written(|out| {
            write_shot_sample(out, &sim::ShotSampleProvenance::default());
        });
        *bytes.last_mut().unwrap() = 9;
        assert!(read_shot_sample(&mut WireReader::new(&bytes)).is_err());
    }
}
