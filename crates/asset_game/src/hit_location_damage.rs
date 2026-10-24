//! The damage scale for each body part a bullet can hit, read from the
//! multiplayer hit-location raw file.

use weapon_t6::damage::{LOCATION_DAMAGE_IDENTITY, location_damage_is_valid};
use weapon_t6::hitloc::{HITLOC_COUNT, HITLOC_NAMES};

/// The raw file's name inside the zone.
const RAWFILE_NAME: &str = "info/mp_lochit_dmgtable";

// TODO(fact): the header word Black Ops II writes is not confirmed; the word
// the older games use is assumed.
const HEADER_WORD: &str = "LOCDMGTABLE";

/// The scale of every part before the file speaks: 1.0 each, except the gun,
/// which takes no damage from a hit.
fn untouched_table() -> [f32; HITLOC_COUNT] {
    let mut table = LOCATION_DAMAGE_IDENTITY;
    for (scale, name) in table.iter_mut().zip(HITLOC_NAMES) {
        if name == "gun" {
            *scale = 0.0;
        }
    }
    table
}

/// Reads the `\name\scale\name\scale` pairs after the header word. A name the
/// game does not know keeps no scale of its own and is skipped; a missing
/// header word, a dangling name, a scale that is not a number, or a finished
/// table that is not usable all reject the whole file.
fn parse_pairs(text: &str) -> Option<[f32; HITLOC_COUNT]> {
    let body = text.trim().strip_prefix(HEADER_WORD)?;
    let mut table = untouched_table();
    let mut tokens = body.split('\\').filter(|token| !token.is_empty());
    while let Some(part) = tokens.next() {
        let scale: f32 = tokens.next()?.parse().ok()?;
        if let Some(slot) = HITLOC_NAMES.iter().position(|known| *known == part) {
            table[slot] = scale;
        }
    }
    location_damage_is_valid(&table).then_some(table)
}

/// The hit-location damage table, if `name` is the raw file that holds it
/// and `data` parses. Padding zeros after the text are ignored.
pub fn capture_lochit_table(name: &str, data: &[u8]) -> Option<[f32; HITLOC_COUNT]> {
    if name != RAWFILE_NAME {
        return None;
    }
    let end = data.iter().rposition(|&byte| byte != 0).map_or(0, |last| last + 1);
    let text = core::str::from_utf8(&data[..end]).ok()?;
    parse_pairs(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale_of(table: &[f32; HITLOC_COUNT], part: &str) -> f32 {
        table[HITLOC_NAMES.iter().position(|known| *known == part).unwrap()]
    }

    #[test]
    fn a_bare_header_leaves_every_part_at_one_but_the_gun() {
        let table = parse_pairs("LOCDMGTABLE").unwrap();
        assert_eq!(scale_of(&table, "head"), 1.0);
        assert_eq!(scale_of(&table, "gun"), 0.0);
    }

    #[test]
    fn named_parts_take_their_scale_and_unknown_names_are_skipped() {
        let table = parse_pairs(r"LOCDMGTABLE\head\1.5\no_such_part\9\neck\1.25").unwrap();
        assert_eq!(scale_of(&table, "head"), 1.5);
        assert_eq!(scale_of(&table, "neck"), 1.25);
        assert_eq!(scale_of(&table, "torso_upper"), 1.0);
        assert!(!table.contains(&9.0));
    }

    #[test]
    fn a_file_that_is_cut_off_or_garbled_is_rejected() {
        assert!(parse_pairs(r"head\2").is_none());
        assert!(parse_pairs(r"LOCDMGTABLE\head").is_none());
        assert!(parse_pairs(r"LOCDMGTABLE\head\lots").is_none());
        assert!(parse_pairs(r"LOCDMGTABLE\no_such_part\lots").is_none());
    }

    #[test]
    fn only_the_hit_location_file_is_taken_and_padding_is_ignored() {
        let data = b"LOCDMGTABLE\\helmet\\2\0\0";
        let table = capture_lochit_table(RAWFILE_NAME, data).unwrap();
        assert_eq!(scale_of(&table, "helmet"), 2.0);
        assert!(capture_lochit_table("info/bullet_penetration_mp", data).is_none());
        assert!(capture_lochit_table(RAWFILE_NAME, b"BULLET_PEN_TABLE").is_none());
        assert!(capture_lochit_table(RAWFILE_NAME, &[0, 0]).is_none());
    }
}
