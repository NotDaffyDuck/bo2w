//! The scoreboard command parser.

use super::number::{leading_int, token};
use super::{SCOREBOARD_ROW_LIMIT, DecodedScoreboard, WORDS_PER_BOARD_ROW, BoardRow};

/// Where the per-client numbers start in the command.
const FIRST_CLIENT_TOKEN: usize = 5;

/// Read a command already split into words. Anything that does not start with `b` is empty;
/// the client count is held to `0..=SCOREBOARD_ROW_LIMIT`; missing or bad numbers are 0.
pub fn read_board_words(words: &[&str]) -> DecodedScoreboard {
    let mut parsed = DecodedScoreboard::default();
    if !token(words, 0).starts_with('b') {
        return parsed;
    }
    let count = leading_int(token(words, 1)).clamp(0, SCOREBOARD_ROW_LIMIT as i32);
    parsed.num = count as usize;
    parsed.opposing_side_points = leading_int(token(words, 2));
    parsed.friendly_side_points = leading_int(token(words, 3));
    parsed.points_cap = leading_int(token(words, 4));
    for row in 0..parsed.num {
        let first = FIRST_CLIENT_TOKEN + row * WORDS_PER_BOARD_ROW;
        let number = |offset: usize| leading_int(token(words, first + offset));
        parsed.icon_column[row] = number(4);
        parsed.extra_score[row] = number(7);
        parsed.rows[row] = BoardRow {
            client: number(0),
            tally: number(1),
            latency: number(2),
            times_died: number(3),
            frags: number(5),
            helps: number(6),
            ..BoardRow::default()
        };
    }
    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(command: &str) -> Vec<&str> {
        command.split_whitespace().collect()
    }

    #[test]
    fn a_command_not_starting_with_b_is_empty() {
        assert_eq!(read_board_words(&split("c 1 2 3 4")), DecodedScoreboard::default());
        assert_eq!(read_board_words(&[]), DecodedScoreboard::default());
    }

    #[test]
    fn the_header_numbers_are_axis_allies_and_limit() {
        let parsed = read_board_words(&split("b 0 11 22 33"));
        assert_eq!(parsed.num, 0);
        assert_eq!(
            (parsed.opposing_side_points, parsed.friendly_side_points, parsed.points_cap),
            (11, 22, 33)
        );
    }

    #[test]
    fn a_word_starting_with_b_counts_as_the_tag() {
        assert_eq!(read_board_words(&split("board 0 5 6 7")).opposing_side_points, 5);
    }

    #[test]
    fn each_client_row_reads_its_eight_numbers_in_order() {
        let parsed = read_board_words(&split("b 2 0 0 0  1 10 20 3 40 5 60 70  2 11 21 4 41 6 61 71"));
        assert_eq!(parsed.num, 2);
        let first = parsed.rows[0];
        assert_eq!(
            (first.client, first.tally, first.latency, first.times_died, first.frags, first.helps),
            (1, 10, 20, 3, 5, 60)
        );
        assert_eq!((parsed.icon_column[0], parsed.extra_score[0]), (40, 70));
        let second = parsed.rows[1];
        assert_eq!((second.client, second.tally, second.frags), (2, 11, 6));
        assert_eq!((parsed.icon_column[1], parsed.extra_score[1]), (41, 71));
        assert_eq!(parsed.rows[2], BoardRow::default(), "unlisted rows stay empty");
    }

    #[test]
    fn team_rank_and_status_icon_fields_of_a_row_are_never_filled() {
        let parsed = read_board_words(&split("b 1 0 0 0 1 2 3 4 5 6 7 8"));
        let row = parsed.rows[0];
        assert_eq!((row.side, row.tier, row.icon_code), (0, 0, 0));
    }

    #[test]
    fn the_count_is_held_between_zero_and_the_cap() {
        assert_eq!(read_board_words(&split("b 99 0 0 0")).num, SCOREBOARD_ROW_LIMIT);
        assert_eq!(read_board_words(&split("b -4 0 0 0")).num, 0);
        assert_eq!(read_board_words(&split("b nonsense 0 0 0")).num, 0);
    }

    #[test]
    fn numbers_a_short_command_leaves_out_read_as_zero() {
        let parsed = read_board_words(&split("b 2 0 0 0 1 10"));
        assert_eq!(parsed.num, 2);
        assert_eq!((parsed.rows[0].client, parsed.rows[0].tally, parsed.rows[0].latency), (1, 10, 0));
        assert_eq!(parsed.rows[1], BoardRow::default());
    }

    #[test]
    fn a_full_table_reads_the_last_row() {
        let mut command = String::from("b 18 1 2 3");
        for client in 0..18 {
            command.push_str(&format!(" {client} {client} 0 0 0 0 0 0"));
        }
        let parsed = read_board_words(&split(&command));
        assert_eq!(parsed.num, 18);
        assert_eq!(parsed.rows[17].client, 17);
    }
}
