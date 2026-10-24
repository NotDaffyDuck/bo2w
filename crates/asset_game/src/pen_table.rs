//! The bullet penetration depths, read from the multiplayer raw file.

use weapon_t6::penetration::{PenetrationDepthTable, split_pen_key_in};
use weapon_t6::surface::T6_SURFACE_TYPE_NAMES;

/// The word the raw file starts with.
const HEADER_WORD: &str = "BULLET_PEN_TABLE";

/// Reads `info/bullet_penetration_mp`: the header word, then
/// `\key\depth` pairs where each key names a penetrate type and a surface.
/// Padding zeros after the text are ignored. A missing header word, a key
/// that names no surface, a dangling key or a depth that is not a number all
/// reject the whole file, and the error says which.
pub fn parse_pen_table_t6(data: &[u8]) -> Result<PenetrationDepthTable, &'static str> {
    let end = data.iter().rposition(|&byte| byte != 0).map_or(0, |last| last + 1);
    let text = core::str::from_utf8(&data[..end]).map_err(|_| "not text")?;
    let body = text
        .trim()
        .strip_prefix(HEADER_WORD)
        .ok_or("no header word")?;
    let mut table = PenetrationDepthTable::empty();
    let mut tokens = body.split('\\').filter(|token| !token.is_empty());
    while let Some(key) = tokens.next() {
        let depth: f32 = tokens
            .next()
            .ok_or("key without a depth")?
            .parse()
            .map_err(|_| "depth is not a number")?;
        let (penetrate_type, surface) =
            split_pen_key_in(key, &T6_SURFACE_TYPE_NAMES).ok_or("key names no surface")?;
        table.set_depth(penetrate_type, surface, depth);
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_key() -> String {
        format!("{}_{}", "small", T6_SURFACE_TYPE_NAMES[7])
    }

    #[test]
    fn a_bare_header_is_an_empty_table() {
        assert!(parse_pen_table_t6(b"BULLET_PEN_TABLE").is_ok());
        assert!(parse_pen_table_t6(b"BULLET_PEN_TABLE\0\0").is_ok());
    }

    #[test]
    fn a_file_without_the_header_word_is_rejected() {
        assert_eq!(parse_pen_table_t6(b"\\small_flesh\\1"), Err("no header word"));
        assert_eq!(parse_pen_table_t6(&[0xff, 0xfe]), Err("not text"));
    }

    #[test]
    fn a_broken_pair_rejects_the_file() {
        let key = first_key();
        let dangling = format!("BULLET_PEN_TABLE\\{key}");
        assert_eq!(parse_pen_table_t6(dangling.as_bytes()), Err("key without a depth"));
        let not_a_number = format!("BULLET_PEN_TABLE\\{key}\\deep");
        assert_eq!(parse_pen_table_t6(not_a_number.as_bytes()), Err("depth is not a number"));
        assert_eq!(
            parse_pen_table_t6(b"BULLET_PEN_TABLE\\no_such_key\\1"),
            Err("key names no surface")
        );
    }
}
