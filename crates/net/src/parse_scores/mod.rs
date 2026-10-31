//! Reading the scoreboard command: `b <count> <axis> <allies> <limit>` followed by eight numbers
//! for each listed client.

mod number;
mod parse;

pub use parse::read_board_words;

/// One client's row on the scoreboard.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardRow {
    pub client: i32,
    pub tally: i32,
    pub latency: i32,
    pub times_died: i32,
    pub side: i32,
    pub frags: i32,
    pub tier: i32,
    pub helps: i32,
    pub icon_code: u32,
}

/// Most clients one scoreboard command lists.
pub const SCOREBOARD_ROW_LIMIT: usize = 0x12;

/// Numbers the command carries for each client.
pub(crate) const WORDS_PER_BOARD_ROW: usize = 8;

/// A scoreboard command read into fixed-size tables; only the first `num` rows are filled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecodedScoreboard {
    pub num: usize,
    pub opposing_side_points: i32,
    pub friendly_side_points: i32,
    pub points_cap: i32,
    pub rows: [BoardRow; SCOREBOARD_ROW_LIMIT],
    pub extra_score: [i32; SCOREBOARD_ROW_LIMIT],
    pub icon_column: [i32; SCOREBOARD_ROW_LIMIT],
}

impl Default for DecodedScoreboard {
    fn default() -> Self {
        Self {
            num: 0,
            opposing_side_points: 0,
            friendly_side_points: 0,
            points_cap: 0,
            rows: [BoardRow::default(); SCOREBOARD_ROW_LIMIT],
            extra_score: [0; SCOREBOARD_ROW_LIMIT],
            icon_column: [0; SCOREBOARD_ROW_LIMIT],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_eighteen_rows_of_eight_numbers() {
        assert_eq!(SCOREBOARD_ROW_LIMIT, 18);
        assert_eq!(WORDS_PER_BOARD_ROW, 8);
    }

    #[test]
    fn a_default_table_is_all_zero() {
        let blank = DecodedScoreboard::default();
        assert_eq!(blank.num, 0);
        assert!(blank.rows.iter().all(|row| *row == BoardRow::default()));
        assert!(blank.extra_score.iter().all(|&n| n == 0));
        assert!(blank.icon_column.iter().all(|&n| n == 0));
    }
}
