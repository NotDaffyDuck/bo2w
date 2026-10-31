//! The byte layout of the small structures packets are built from. Words are little-endian
//! through `ByteSinkWriter`; a 64-bit value travels as two words, the high one first.

use sim::ShotRecordGrade;

use super::handshake::{AssetDigestSet, JoinGreeting, JoinRefusal, NegotiatedCaps};
use super::identity::{LinkNumber, DatagramPreamble};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// Writes 32-bit words in order.
fn put_words<const N: usize>(sink: &mut ByteSinkWriter, parts: [u32; N]) {
    parts.into_iter().for_each(|part| sink.put_u32(part));
}

/// Reads `N` 32-bit words in order.
fn take_words<const N: usize>(cursor: &mut ByteCursorReader<'_>) -> Result<[u32; N], PacketDecodeFault> {
    let mut parts = [0u32; N];
    for slot in &mut parts {
        *slot = cursor.read_word32()?;
    }
    Ok(parts)
}

pub(super) fn write_u64(sink: &mut ByteSinkWriter, wide: u64) {
    let [low, high] = [wide as u32, (wide >> 32) as u32];
    put_words(sink, [high, low]);
}

pub(super) fn read_u64(cursor: &mut ByteCursorReader<'_>) -> Result<u64, PacketDecodeFault> {
    let [high, low] = take_words(cursor)?;
    Ok((u64::from(high) << 32) | u64::from(low))
}

pub(super) fn write_connection(sink: &mut ByteSinkWriter, connection: LinkNumber) {
    write_u64(sink, connection.0);
}

pub(super) fn read_connection(cursor: &mut ByteCursorReader<'_>) -> Result<LinkNumber, PacketDecodeFault> {
    Ok(LinkNumber(read_u64(cursor)?))
}

/// Connection id, then sequence, acknowledged sequence and epoch.
pub(super) fn write_header(sink: &mut ByteSinkWriter, header: &DatagramPreamble) {
    write_connection(sink, header.connection);
    put_words(sink, [header.packet_no, header.confirm_receipt, header.round_era]);
}

pub(super) fn read_header(cursor: &mut ByteCursorReader<'_>) -> Result<DatagramPreamble, PacketDecodeFault> {
    let connection = read_connection(cursor)?;
    let [sequence, confirm_receipt, epoch] = take_words(cursor)?;
    Ok(DatagramPreamble { connection, packet_no: sequence, confirm_receipt, round_era: epoch })
}

/// Protocol version, the five content digests, then the limits.
pub(super) fn write_hello(sink: &mut ByteSinkWriter, hello: &JoinGreeting) {
    let digests = &hello.content;
    let caps = &hello.limits;
    put_words(sink, [hello.wire_version]);
    [digests.rules_digest, digests.map, digests.model_digest, digests.weapon_digest, digests.class_digest]
        .into_iter()
        .for_each(|digest| write_u64(sink, digest));
    put_words(sink, [caps.host_step_rate]);
    sink.put_u16(caps.moves_budget_per_step);
    sink.put_u16(caps.action_budget_per_step);
    put_words(sink, [caps.datagram_size_ceiling]);
}

pub(super) fn read_hello(cursor: &mut ByteCursorReader<'_>) -> Result<JoinGreeting, PacketDecodeFault> {
    let [protocol_version] = take_words(cursor)?;
    let mut digests = [0u64; 5];
    for digest in &mut digests {
        *digest = read_u64(cursor)?;
    }
    let [gameplay, map, models, weapons, classes] = digests;
    let [host_step_rate] = take_words(cursor)?;
    let moves_budget_per_step = cursor.get_u16()?;
    let action_budget_per_step = cursor.get_u16()?;
    let [datagram_size_ceiling] = take_words(cursor)?;
    Ok(JoinGreeting {
        wire_version: protocol_version,
        content: AssetDigestSet { rules_digest: gameplay, map, model_digest: models, weapon_digest: weapons, class_digest: classes },
        limits: NegotiatedCaps { host_step_rate, moves_budget_per_step, action_budget_per_step, datagram_size_ceiling },
    })
}

/// Refusal tags on the wire.
mod refusal_tag {
    pub const VERSION: u8 = 1;
    pub const MAP: u8 = 2;
    pub const WEAPONS: u8 = 3;
    pub const CLASSES: u8 = 4;
    pub const STEP_RATE: u8 = 5;
    pub const LIMITS: u8 = 6;
}

/// A refusal is its tag, then (for the mismatch kinds) our value and theirs:
/// two words for the version and step rate, two 64-bit digests for content.
pub(super) fn write_reject(sink: &mut ByteSinkWriter, reject: JoinRefusal) {
    enum Pair {
        Words(u32, u32),
        Digests(u64, u64),
        Bare,
    }
    let (code, pair) = match reject {
        JoinRefusal::VersionClash { host_side, peer_side } => (refusal_tag::VERSION, Pair::Words(host_side, peer_side)),
        JoinRefusal::LevelAssetHashes { host_side, peer_side } => (refusal_tag::MAP, Pair::Digests(host_side, peer_side)),
        JoinRefusal::WeaponLedger { host_side, peer_side } => (refusal_tag::WEAPONS, Pair::Digests(host_side, peer_side)),
        JoinRefusal::LoadoutTableHashes { host_side, peer_side } => (refusal_tag::CLASSES, Pair::Digests(host_side, peer_side)),
        JoinRefusal::StepFrequency { host_side, peer_side } => (refusal_tag::STEP_RATE, Pair::Words(host_side, peer_side)),
        JoinRefusal::Limits => (refusal_tag::LIMITS, Pair::Bare),
    };
    sink.put_u8(code);
    match pair {
        Pair::Words(mine, other) => put_words(sink, [mine, other]),
        Pair::Digests(mine, other) => {
            write_u64(sink, mine);
            write_u64(sink, other);
        }
        Pair::Bare => {}
    }
}

pub(super) fn read_reject(cursor: &mut ByteCursorReader<'_>) -> Result<JoinRefusal, PacketDecodeFault> {
    let code = cursor.get_u8()?;
    let digest_pair = |cursor: &mut ByteCursorReader<'_>| -> Result<(u64, u64), PacketDecodeFault> {
        Ok((read_u64(cursor)?, read_u64(cursor)?))
    };
    Ok(match code {
        refusal_tag::VERSION | refusal_tag::STEP_RATE => {
            let [host_side, peer_side] = take_words(cursor)?;
            if code == refusal_tag::VERSION {
                JoinRefusal::VersionClash { host_side, peer_side }
            } else {
                JoinRefusal::StepFrequency { host_side, peer_side }
            }
        }
        refusal_tag::MAP => {
            let (host_side, peer_side) = digest_pair(cursor)?;
            JoinRefusal::LevelAssetHashes { host_side, peer_side }
        }
        refusal_tag::WEAPONS => {
            let (host_side, peer_side) = digest_pair(cursor)?;
            JoinRefusal::WeaponLedger { host_side, peer_side }
        }
        refusal_tag::CLASSES => {
            let (host_side, peer_side) = digest_pair(cursor)?;
            JoinRefusal::LoadoutTableHashes { host_side, peer_side }
        }
        refusal_tag::LIMITS => JoinRefusal::Limits,
        _ => return Err(PacketDecodeFault::GarbledRequest("unknown HandshakeReject tag")),
    })
}

/// Each shot-sample quality and its byte on the wire.
const QUALITY_CODES: [(ShotRecordGrade, u8); 5] = [
    (ShotRecordGrade::None, 0),
    (ShotRecordGrade::Exact, 1),
    (ShotRecordGrade::SmoothedBetweenFrames, 2),
    (ShotRecordGrade::Held, 3),
    (ShotRecordGrade::RanDry, 4),
];

/// The two ticks, the blend factor's bits, then the quality byte.
pub(super) fn write_shot_sample(sink: &mut ByteSinkWriter, sample: &sim::ShotRecordOrigin) {
    put_words(sink, [sample.left.0, sample.right.0, sample.opacity.to_bits()]);
    let code = QUALITY_CODES.iter().find(|(grade, _)| *grade == sample.quality).map_or(0, |&(_, code)| code);
    sink.put_u8(code);
}

pub(super) fn read_shot_sample(cursor: &mut ByteCursorReader<'_>) -> Result<sim::ShotRecordOrigin, PacketDecodeFault> {
    let [left, right, alpha_bits] = take_words(cursor)?;
    let code = cursor.get_u8()?;
    let Some(&(quality, _)) = QUALITY_CODES.iter().find(|&&(_, known)| known == code) else {
        return Err(PacketDecodeFault::GarbledRequest("unknown ShotSampleQuality tag"));
    };
    Ok(sim::ShotRecordOrigin {
        left: sim::Tick(left),
        right: sim::Tick(right),
        opacity: f32::from_bits(alpha_bits),
        quality,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(fill: impl FnOnce(&mut ByteSinkWriter)) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        fill(&mut sink);
        sink.into_bytes()
    }

    #[test]
    fn a_64_bit_value_travels_high_word_first() {
        let bytes = written(|sink| write_u64(sink, 0x0102_0304_0506_0708));
        assert_eq!(bytes, [4, 3, 2, 1, 8, 7, 6, 5]);
        assert_eq!(
            read_u64(&mut ByteCursorReader::new(&bytes)).unwrap(),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn a_header_is_twenty_bytes_and_round_trips() {
        let header = DatagramPreamble {
            connection: LinkNumber(0xAABB_CCDD_0000_0001),
            packet_no: 9,
            confirm_receipt: 8,
            round_era: 7,
        };
        let bytes = written(|sink| write_header(sink, &header));
        assert_eq!(bytes.len(), 20);
        assert_eq!(read_header(&mut ByteCursorReader::new(&bytes)).unwrap(), header);
    }

    #[test]
    fn a_hello_round_trips() {
        let hello = JoinGreeting {
            wire_version: 11,
            content: AssetDigestSet {
                rules_digest: 1,
                map: 2,
                model_digest: 3,
                weapon_digest: 4,
                class_digest: 5,
            },
            limits: NegotiatedCaps {
                host_step_rate: 60,
                moves_budget_per_step: 16,
                action_budget_per_step: 32,
                datagram_size_ceiling: 1000,
            },
        };
        let bytes = written(|sink| write_hello(sink, &hello));
        assert_eq!(bytes.len(), 4 + 5 * 8 + 4 + 2 + 2 + 4);
        assert_eq!(read_hello(&mut ByteCursorReader::new(&bytes)).unwrap(), hello);
    }

    #[test]
    fn every_reject_round_trips_and_an_unknown_tag_is_refused() {
        for reject in [
            JoinRefusal::VersionClash { host_side: 1, peer_side: 2 },
            JoinRefusal::LevelAssetHashes { host_side: 3, peer_side: 4 },
            JoinRefusal::WeaponLedger { host_side: 5, peer_side: 6 },
            JoinRefusal::LoadoutTableHashes { host_side: 7, peer_side: 8 },
            JoinRefusal::StepFrequency { host_side: 9, peer_side: 10 },
            JoinRefusal::Limits,
        ] {
            let bytes = written(|sink| write_reject(sink, reject));
            assert_eq!(read_reject(&mut ByteCursorReader::new(&bytes)).unwrap(), reject);
        }
        assert!(read_reject(&mut ByteCursorReader::new(&[0])).is_err());
        assert!(read_reject(&mut ByteCursorReader::new(&[7])).is_err());
    }

    #[test]
    fn every_shot_sample_quality_round_trips() {
        for quality in [
            ShotRecordGrade::None,
            ShotRecordGrade::Exact,
            ShotRecordGrade::SmoothedBetweenFrames,
            ShotRecordGrade::Held,
            ShotRecordGrade::RanDry,
        ] {
            let sample = sim::ShotRecordOrigin {
                left: sim::Tick(4),
                right: sim::Tick(5),
                opacity: 0.25,
                quality,
            };
            let bytes = written(|sink| write_shot_sample(sink, &sample));
            assert_eq!(bytes.len(), 13);
            assert_eq!(
                read_shot_sample(&mut ByteCursorReader::new(&bytes)).unwrap(),
                sample
            );
        }
    }

    #[test]
    fn an_unknown_quality_tag_is_refused() {
        let mut bytes = written(|sink| {
            write_shot_sample(sink, &sim::ShotRecordOrigin::default());
        });
        *bytes.last_mut().unwrap() = 9;
        assert!(read_shot_sample(&mut ByteCursorReader::new(&bytes)).is_err());
    }
}