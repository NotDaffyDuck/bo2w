//! Fixed-size text fields: a menu name, a menu answer or a killstreak name, held inside an action.

/// The most bytes of text one field holds.
pub const MENU_RESPONSE_BYTES: usize = 48;

/// Puts `text` into a field, padded with zero bytes. `None` when it does not fit.
pub fn menu_response_field(text: &str) -> Option<[u8; MENU_RESPONSE_BYTES]> {
    let mut field = [0u8; MENU_RESPONSE_BYTES];
    field.get_mut(..text.len())?.copy_from_slice(text.as_bytes());
    Some(field)
}

/// The text in a field, up to its first zero byte. Bytes that are not valid UTF-8 read as empty text.
pub fn menu_response_text(field: &[u8; MENU_RESPONSE_BYTES]) -> &str {
    let text = field.split(|&byte| byte == 0).next().unwrap_or_default();
    std::str::from_utf8(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_padded_and_reads_back() {
        let field = menu_response_field("class_select").unwrap();
        assert_eq!(&field[..12], b"class_select");
        assert!(field[12..].iter().all(|&byte| byte == 0));
        assert_eq!(menu_response_text(&field), "class_select");
    }

    #[test]
    fn a_full_field_fits_and_one_byte_more_does_not() {
        let full = "x".repeat(MENU_RESPONSE_BYTES);
        let field = menu_response_field(&full).unwrap();
        assert_eq!(menu_response_text(&field), full);
        assert_eq!(menu_response_field(&format!("{full}x")), None);
    }

    #[test]
    fn empty_and_broken_fields_read_as_empty_text() {
        assert_eq!(menu_response_text(&menu_response_field("").unwrap()), "");
        let mut broken = [0u8; MENU_RESPONSE_BYTES];
        broken[..2].copy_from_slice(&[0xff, 0xfe]);
        assert_eq!(menu_response_text(&broken), "");
    }

    #[test]
    fn reading_stops_at_the_first_zero() {
        let mut field = menu_response_field("ab").unwrap();
        field[5] = b'z';
        assert_eq!(menu_response_text(&field), "ab");
    }
}
