//! One player's state on the wire: the number of leading fields covered, a bit per covered field
//! saying whether it changed, then the changed fields in order.

use playerstate_t6::game::AvatarState;

use crate::net_links::crossing_fields::{PLAYER_WIRE_FIELD_TOTAL, value_changed, extract_value, store_value};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

const MASK_BYTES: usize = PLAYER_WIRE_FIELD_TOTAL.div_ceil(8);

pub(super) fn encode_player(sink: &mut ByteSinkWriter, baseline: &AvatarState, current: &AvatarState) {
    let changed: Vec<usize> = (0..PLAYER_WIRE_FIELD_TOTAL)
        .filter(|&index| value_changed(baseline, current, index))
        .collect();
    // Fields after the last changed one are not mentioned at all.
    let covered = changed.last().map_or(0, |&last| last + 1);
    sink.put_u8(covered as u8);

    let mut changed_bits = [0u8; MASK_BYTES];
    for &index in &changed {
        changed_bits[index / 8] |= 1 << (index % 8);
    }
    for &byte in &changed_bits[..covered.div_ceil(8)] {
        sink.put_u8(byte);
    }
    for &index in &changed {
        store_value(sink, current, index);
    }
}

pub(super) fn parse_participant_state(
    cursor: &mut ByteCursorReader<'_>,
    state: &mut AvatarState,
) -> Result<(), PacketDecodeFault> {
    let covered = cursor.get_u8()? as usize;
    if covered > PLAYER_WIRE_FIELD_TOTAL {
        return Err(PacketDecodeFault::GarbledRequest(
            "changed-field count exceeds the playerState registry",
        ));
    }

    let mut changed_bits = [0u8; MASK_BYTES];
    for byte in &mut changed_bits[..covered.div_ceil(8)] {
        *byte = cursor.get_u8()?;
    }
    for index in 0..covered {
        if changed_bits[index / 8] >> (index % 8) & 1 != 0 {
            extract_value(cursor, state, index)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(baseline: &AvatarState, current: &AvatarState) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        encode_player(&mut sink, baseline, current);
        sink.into_bytes()
    }

    #[test]
    fn an_unchanged_player_is_one_zero_byte() {
        assert_eq!(encoded(&AvatarState::ZERO, &AvatarState::ZERO), vec![0]);
    }

    #[test]
    fn a_changed_player_decodes_back_onto_its_baseline() {
        let mut current = AvatarState::ZERO;
        current.last_cmd_ms = 1234;
        current.origin = [1.0, 2.0, 3.0];
        let bytes = encoded(&AvatarState::ZERO, &current);

        let mut state = AvatarState::ZERO;
        let mut cursor = ByteCursorReader::new(&bytes);
        parse_participant_state(&mut cursor, &mut state).unwrap();
        assert!(cursor.is_empty());
        assert_eq!(state, current);
    }

    #[test]
    fn only_the_difference_from_the_baseline_is_sent() {
        let mut baseline = AvatarState::ZERO;
        baseline.last_cmd_ms = 10;
        let mut current = baseline;
        current.origin = [5.0, 0.0, 0.0];
        let delta = encoded(&baseline, &current);
        let full = encoded(&AvatarState::ZERO, &current);
        assert!(delta.len() < full.len());

        let mut state = baseline;
        parse_participant_state(&mut ByteCursorReader::new(&delta), &mut state).unwrap();
        assert_eq!(state, current);
    }

    #[test]
    fn a_field_count_beyond_the_registry_is_malformed() {
        let bytes = [(PLAYER_WIRE_FIELD_TOTAL + 1) as u8];
        let mut state = AvatarState::ZERO;
        let err = parse_participant_state(&mut ByteCursorReader::new(&bytes), &mut state).unwrap_err();
        assert!(matches!(err, PacketDecodeFault::GarbledRequest(_)));
    }

    #[test]
    fn a_truncated_mask_is_an_error() {
        let bytes = [8u8];
        let mut state = AvatarState::ZERO;
        assert!(parse_participant_state(&mut ByteCursorReader::new(&bytes), &mut state).is_err());
    }
}
