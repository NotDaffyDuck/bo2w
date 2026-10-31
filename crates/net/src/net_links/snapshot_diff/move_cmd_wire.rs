//! One movement command on the wire. The field order is the wire format.

use playerstate_t6::game::InputCommand;

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

/// The wire value for "no melee charge target".
const NO_CHARGE_ENTITY: u16 = u16::MAX;

pub(crate) fn pack_input_record(sink: &mut ByteSinkWriter, command: &InputCommand) {
    sink.poke_int32(command.authority_clock_ms);
    sink.put_u32(command.buttons);
    for axis in command.angles {
        sink.poke_int32(axis);
    }
    sink.put_u16(command.weapon);
    sink.put_u16(command.weapon_slot_code);
    sink.put_u16(command.secondary_hand_slot);
    sink.put_i8(command.thrust_ahead);
    sink.put_i8(command.thrust_lateral);
    sink.write_real32(command.lunge_heading);
    sink.put_u8(command.lunge_reach);
    sink.put_u16(command.melee_charge_ent.unwrap_or(NO_CHARGE_ENTITY));
    sink.append_raw_slice(&command.chosen_map_spot);
    sink.append_raw_slice(&command.remote_operated);
    sink.put_u8(command.last_input);
}

pub(crate) fn unpack_input_record(cursor: &mut ByteCursorReader<'_>) -> Result<InputCommand, PacketDecodeFault> {
    let authority_clock_ms = cursor.get_i32()?;
    let buttons = cursor.read_word32()?;
    let angles = [cursor.get_i32()?, cursor.get_i32()?, cursor.get_i32()?];
    let weapon = cursor.get_u16()?;
    let weapon_slot_code = cursor.get_u16()?;
    let secondary_hand_slot = cursor.get_u16()?;
    let thrust_ahead = cursor.get_i8()?;
    let thrust_lateral = cursor.get_i8()?;
    let lunge_heading = cursor.read_f32()?;
    let lunge_reach = cursor.get_u8()?;
    let melee_charge_ent = Some(cursor.get_u16()?).filter(|&ent| ent != NO_CHARGE_ENTITY);
    let mut chosen_map_spot = [0u8; 3];
    cursor.copy_out(&mut chosen_map_spot)?;
    let mut remote_operated = [0u8; 2];
    cursor.copy_out(&mut remote_operated)?;
    let last_input = cursor.get_u8()?;
    Ok(InputCommand {
        authority_clock_ms,
        buttons,
        angles,
        weapon,
        weapon_slot_code,
        secondary_hand_slot,
        thrust_ahead,
        thrust_lateral,
        lunge_heading,
        lunge_reach,
        melee_charge_ent,
        chosen_map_spot,
        remote_operated,
        last_input,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InputCommand {
        InputCommand {
            authority_clock_ms: 123_456,
            buttons: 0b1010_0001,
            angles: [10, -20, 30],
            weapon: 4,
            weapon_slot_code: 5,
            secondary_hand_slot: 6,
            thrust_ahead: 127,
            thrust_lateral: -128,
            lunge_heading: 45.5,
            lunge_reach: 9,
            melee_charge_ent: Some(300),
            chosen_map_spot: [1, 2, 3],
            remote_operated: [4, 5],
            last_input: 7,
        }
    }

    fn bytes_of(command: &InputCommand) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        pack_input_record(&mut sink, command);
        sink.into_bytes()
    }

    #[test]
    fn a_command_survives_the_wire() {
        let command = sample();
        let bytes = bytes_of(&command);
        let mut cursor = ByteCursorReader::new(&bytes);
        assert_eq!(unpack_input_record(&mut cursor).unwrap(), command);
        assert!(cursor.is_empty());
    }

    #[test]
    fn no_charge_target_is_the_all_ones_value() {
        let mut command = sample();
        command.melee_charge_ent = None;
        let bytes = bytes_of(&command);
        assert_eq!(unpack_input_record(&mut ByteCursorReader::new(&bytes)).unwrap(), command);
    }

    #[test]
    fn a_command_is_a_fixed_number_of_bytes() {
        // 4 + 4 + 12 + 6 + 2 + 4 + 1 + 2 + 3 + 2 + 1
        assert_eq!(bytes_of(&sample()).len(), 41);
        assert_eq!(bytes_of(&InputCommand::default()).len(), 41);
    }

    #[test]
    fn a_short_command_is_an_error() {
        let bytes = bytes_of(&sample());
        assert!(unpack_input_record(&mut ByteCursorReader::new(&bytes[..bytes.len() - 1])).is_err());
    }
}
