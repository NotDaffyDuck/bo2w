//! Frame encoding of the optional scoreboard text: a flag byte, then for 1 a u16 length and
//! that many UTF-8 bytes.

use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub fn encode_svc_scores(sink: &mut ByteSinkWriter, command: Option<&str>) {
    let Some(board_text) = command else {
        sink.put_u8(0);
        return;
    };
    debug_assert!(board_text.len() <= u16::MAX as usize);
    sink.put_u8(1);
    sink.put_u16(board_text.len() as u16);
    sink.append_raw_slice(board_text.as_bytes());
}

pub fn parse_scoreboard_message(wire: &mut ByteCursorReader<'_>) -> Result<Option<String>, PacketDecodeFault> {
    match wire.get_u8()? {
        0 => Ok(None),
        1 => {
            let mut utf8_buf = vec![0; wire.get_u16()? as usize];
            wire.copy_out(&mut utf8_buf)?;
            String::from_utf8(utf8_buf)
                .map(Some)
                .map_err(|_| PacketDecodeFault::GarbledRequest("svc 'b' is not UTF-8"))
        }
        _ => Err(PacketDecodeFault::GarbledRequest("svc scores tag is not 0/1")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(command: Option<&str>) -> Vec<u8> {
        let mut sink = ByteSinkWriter::new();
        encode_svc_scores(&mut sink, command);
        sink.into_bytes()
    }

    #[test]
    fn nothing_is_a_single_zero_byte() {
        assert_eq!(encoded(None), [0]);
        assert_eq!(parse_scoreboard_message(&mut ByteCursorReader::new(&[0])), Ok(None));
    }

    #[test]
    fn text_is_a_one_then_a_length_then_the_bytes() {
        assert_eq!(encoded(Some("b 0")), [1, 3, 0, b'b', b' ', b'0']);
    }

    #[test]
    fn text_survives_a_round_trip() {
        for text in ["", "b 0 1 2 3", "ünï ✓"] {
            let bytes = encoded(Some(text));
            let mut reader = ByteCursorReader::new(&bytes);
            assert_eq!(parse_scoreboard_message(&mut reader), Ok(Some(text.to_owned())));
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn any_other_flag_is_malformed() {
        assert_eq!(
            parse_scoreboard_message(&mut ByteCursorReader::new(&[2])),
            Err(PacketDecodeFault::GarbledRequest("svc scores tag is not 0/1"))
        );
    }

    #[test]
    fn bytes_that_are_not_text_are_malformed() {
        assert_eq!(
            parse_scoreboard_message(&mut ByteCursorReader::new(&[1, 1, 0, 0xff])),
            Err(PacketDecodeFault::GarbledRequest("svc 'b' is not UTF-8"))
        );
    }

    #[test]
    fn a_cut_short_message_is_truncated() {
        for bytes in [&[][..], &[1][..], &[1, 5, 0, b'a'][..]] {
            assert!(
                matches!(parse_scoreboard_message(&mut ByteCursorReader::new(bytes)), Err(PacketDecodeFault::ShortRead { .. })),
                "{bytes:?}"
            );
        }
    }
}
