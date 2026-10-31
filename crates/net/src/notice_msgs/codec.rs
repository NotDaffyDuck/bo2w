//! Frame encoding of the notice list.
//!
//! Layout: a u16 count, then per notice a u32 id, a u8 tag, and two strings, each a u8 length
//! followed by that many UTF-8 bytes (name, then key). A frame with nothing left to read where
//! the count would be carries no notices.

use super::{HOST_MSG_LEAVE_ALERT, HOST_MSG_TEXT_LINE, SVC_PRINT_BOLD, HostGameAnnouncement};
use crate::net_links::byte_codec::{PacketDecodeFault, ByteCursorReader, ByteSinkWriter};

pub fn pack_host_game_alerts(out: &mut ByteSinkWriter, notices: &[HostGameAnnouncement]) {
    out.put_u16(u16::try_from(notices.len()).expect("gamenotify count"));
    for notice in notices {
        out.put_u32(notice.id);
        out.put_u8(notice.tag);
        put_string(out, &notice.name);
        put_string(out, &notice.key);
    }
}

fn put_string(out: &mut ByteSinkWriter, field_text: &str) {
    debug_assert!(field_text.len() <= u8::MAX as usize);
    out.put_u8(field_text.len() as u8);
    out.append_raw_slice(field_text.as_bytes());
}

pub fn parse_host_game_alerts(
    cursor: &mut ByteCursorReader<'_>,
) -> Result<Vec<HostGameAnnouncement>, PacketDecodeFault> {
    if cursor.unread_len() == 0 {
        return Ok(Vec::new());
    }
    let total = cursor.get_u16()? as usize;
    let mut notices = Vec::with_capacity(total);
    for _ in 0..total {
        let id = cursor.read_word32()?;
        let kind = cursor.get_u8()?;
        if !matches!(kind, HOST_MSG_LEAVE_ALERT | HOST_MSG_TEXT_LINE | SVC_PRINT_BOLD) {
            return Err(PacketDecodeFault::GarbledRequest("gamenotify tag is not 'e', 'f' or 'g'"));
        }
        let name = get_string(cursor, "gamenotify name UTF-8")?;
        let key = get_string(cursor, "gamenotify key UTF-8")?;
        notices.push(HostGameAnnouncement { id, tag: kind, name, key });
    }
    Ok(notices)
}

fn get_string(cursor: &mut ByteCursorReader<'_>, utf8_error: &'static str) -> Result<String, PacketDecodeFault> {
    let mut raw = vec![0; cursor.get_u8()? as usize];
    cursor.copy_out(&mut raw)?;
    String::from_utf8(raw).map_err(|_| PacketDecodeFault::GarbledRequest(utf8_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(id: u32, tag: u8, who_name: &str, text_key: &str) -> HostGameAnnouncement {
        HostGameAnnouncement { id, tag, name: who_name.to_owned(), key: text_key.to_owned() }
    }

    fn encoded(notices: &[HostGameAnnouncement]) -> Vec<u8> {
        let mut out = ByteSinkWriter::new();
        pack_host_game_alerts(&mut out, notices);
        out.into_bytes()
    }

    #[test]
    fn the_layout_is_count_id_tag_then_two_length_prefixed_strings() {
        let bytes = encoded(&[notice(0x01020304, HOST_MSG_TEXT_LINE, "ab", "c")]);
        let expected_id = 0x01020304u32.to_le_bytes();
        assert_eq!(&bytes[..2], 1u16.to_le_bytes());
        assert_eq!(&bytes[2..6], expected_id);
        assert_eq!(&bytes[6..], [b'f', 2, b'a', b'b', 1, b'c']);
    }

    #[test]
    fn notices_survive_a_round_trip_in_order() {
        let notices = vec![
            notice(5, HOST_MSG_LEAVE_ALERT, "Ada", "EXE_LEFTGAME"),
            notice(6, SVC_PRINT_BOLD, "", "KEY"),
            notice(7, HOST_MSG_TEXT_LINE, "ünï", ""),
        ];
        let bytes = encoded(&notices);
        let mut reader = ByteCursorReader::new(&bytes);
        assert_eq!(parse_host_game_alerts(&mut reader), Ok(notices));
        assert!(reader.is_empty());
    }

    #[test]
    fn nothing_left_to_read_means_no_notices() {
        let mut reader = ByteCursorReader::new(&[]);
        assert_eq!(parse_host_game_alerts(&mut reader), Ok(Vec::new()));
    }

    #[test]
    fn an_explicit_zero_count_is_also_empty() {
        assert_eq!(encoded(&[]), [0, 0]);
        let mut reader = ByteCursorReader::new(&[0, 0]);
        assert_eq!(parse_host_game_alerts(&mut reader), Ok(Vec::new()));
    }

    #[test]
    fn an_unknown_tag_is_malformed() {
        let mut bytes = encoded(&[notice(1, HOST_MSG_TEXT_LINE, "a", "b")]);
        bytes[6] = b'z';
        let mut reader = ByteCursorReader::new(&bytes);
        assert_eq!(
            parse_host_game_alerts(&mut reader),
            Err(PacketDecodeFault::GarbledRequest("gamenotify tag is not 'e', 'f' or 'g'"))
        );
    }

    #[test]
    fn bad_utf8_in_the_name_or_the_key_is_named_in_the_error() {
        let mut bad_name = encoded(&[notice(1, HOST_MSG_TEXT_LINE, "a", "b")]);
        bad_name[8] = 0xff;
        assert_eq!(
            parse_host_game_alerts(&mut ByteCursorReader::new(&bad_name)),
            Err(PacketDecodeFault::GarbledRequest("gamenotify name UTF-8"))
        );
        let mut bad_key = encoded(&[notice(1, HOST_MSG_TEXT_LINE, "a", "b")]);
        bad_key[10] = 0xff;
        assert_eq!(
            parse_host_game_alerts(&mut ByteCursorReader::new(&bad_key)),
            Err(PacketDecodeFault::GarbledRequest("gamenotify key UTF-8"))
        );
    }

    #[test]
    fn a_list_cut_short_is_truncated() {
        let bytes = encoded(&[notice(1, HOST_MSG_TEXT_LINE, "name", "key")]);
        for cut in 3..bytes.len() {
            assert!(
                matches!(
                    parse_host_game_alerts(&mut ByteCursorReader::new(&bytes[..cut])),
                    Err(PacketDecodeFault::ShortRead { .. })
                ),
                "cut at {cut}"
            );
        }
    }
}
