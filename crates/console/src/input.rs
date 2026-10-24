//! Text selection and copying at the console: pasted lines, and the place a
//! pointer or a selection lands on the laid-out scrollback.

/// The scrollback as laid out: each glyph is a `(row id, character index)`.
pub type ScrollCell = (usize, usize);

/// The box of one row of text: `(min x, min y, max x, max y)`.
pub type RunBounds = (f32, f32, f32, f32);

/// A pasted command goes on one line.
pub fn normalize_command_paste(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// What copying the prompt's selection came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptCopy {
    /// Nothing was selected.
    Ignored,
    Copied,
    Failed(String),
}

/// Writes the selection with `write`, if there is one.
pub fn copy_prompt_selection(
    selected: Option<&str>,
    write: impl FnOnce(&str) -> Result<(), String>,
) -> PromptCopy {
    match selected {
        Some(text) if !text.is_empty() => match write(text) {
            Ok(()) => PromptCopy::Copied,
            Err(error) => PromptCopy::Failed(error),
        },
        _ => PromptCopy::Ignored,
    }
}

/// The line the console prints about a copy, if it prints one.
pub fn copy_prompt_echo(result: &PromptCopy) -> Option<String> {
    match result {
        PromptCopy::Ignored => None,
        PromptCopy::Copied => Some("Text copied".to_owned()),
        PromptCopy::Failed(error) => Some(format!("clipboard write failed: {error}")),
    }
}

/// For each of the first `glyph_count` glyphs of `text` — a newline has none —
/// the index of the character it draws.
pub fn map_glyphs_to_chars(text: &str, glyph_count: usize) -> Vec<usize> {
    text.chars()
        .enumerate()
        .filter(|&(_, c)| c != '\n')
        .map(|(index, _)| index)
        .take(glyph_count)
        .collect()
}

/// The character indices drawn on each row, in the order the rows first show up.
fn rows(cells: &[ScrollCell]) -> Vec<Vec<usize>> {
    let mut ids: Vec<usize> = Vec::new();
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for &(id, ch) in cells {
        let row = ids.iter().position(|&seen| seen == id).unwrap_or_else(|| {
            ids.push(id);
            rows.push(Vec::new());
            rows.len() - 1
        });
        rows[row].push(ch);
    }
    rows
}

/// How wide one glyph of a row is, by dividing the row's box evenly.
fn advance(run: RunBounds, glyphs: usize) -> f32 {
    (run.2 - run.0) / glyphs as f32
}

/// The row and column a point falls on. A point below the last row is on the
/// last row; one left of the text is at column 0, and one right of it past the
/// last glyph.
pub fn hit_scroll_cell(
    cells: &[ScrollCell],
    runs: &[RunBounds],
    local: (f32, f32),
) -> Option<(usize, usize)> {
    let rows = rows(cells);
    if rows.is_empty() || runs.is_empty() {
        return None;
    }
    let row = runs
        .iter()
        .position(|run| local.1 <= run.3)
        .unwrap_or(runs.len() - 1);
    let glyphs = rows.get(row).map_or(0, Vec::len);
    let step = advance(runs[row], glyphs);
    if glyphs == 0 || step <= 0.0 {
        return Some((row, 0));
    }
    let column = ((local.0 - runs[row].0) / step)
        .floor()
        .clamp(0.0, glyphs as f32);
    Some((row, column as usize))
}

/// The character a row and column stand for. The column just past the end of a
/// row is the character after its last one.
pub fn cell_char_index(
    cells: &[ScrollCell],
    runs: &[RunBounds],
    row: usize,
    col: usize,
) -> Option<usize> {
    if row >= runs.len() {
        return None;
    }
    let rows = rows(cells);
    let chars = rows.get(row).filter(|chars| !chars.is_empty())?;
    match col.cmp(&chars.len()) {
        std::cmp::Ordering::Less => Some(chars[col]),
        std::cmp::Ordering::Equal => Some(chars[col - 1] + 1),
        std::cmp::Ordering::Greater => None,
    }
}

/// The characters a row covers, with its line break.
pub fn line_char_range(
    cells: &[ScrollCell],
    runs: &[RunBounds],
    row: usize,
) -> Option<(usize, usize)> {
    if row >= runs.len() {
        return None;
    }
    let rows = rows(cells);
    let chars = rows.get(row)?;
    Some((*chars.first()?, chars.last()? + 1))
}

/// The boxes to paint for the characters from `lo` up to `hi`: one for the part
/// of each row they cover, reaching `block_right` when they run through the
/// row's line break (the last row has none: nothing follows it).
pub fn cell_rects_for_char_range(
    lo: usize,
    hi: usize,
    cells: &[ScrollCell],
    runs: &[RunBounds],
    block_right: f32,
) -> Vec<RunBounds> {
    let rows = rows(cells);
    let mut rects = Vec::new();
    for (at, (chars, &run)) in rows.iter().zip(runs).enumerate() {
        let (Some(&first), Some(&last)) = (chars.first(), chars.last()) else {
            continue;
        };
        let line_break = last + 1;
        if hi <= first || lo > line_break {
            continue;
        }
        let from = chars.iter().filter(|&&c| c < lo).count();
        let to = chars.iter().filter(|&&c| c < hi).count();
        let through_break = at + 1 < rows.len() && hi > line_break && lo <= line_break;
        if from >= to && !through_break {
            continue;
        }
        let step = advance(run, chars.len());
        let left = run.0 + from as f32 * step;
        let right = if through_break {
            block_right
        } else {
            run.0 + to as f32 * step
        };
        if right > left {
            rects.push((left, run.1, right, run.3));
        }
    }
    rects
}

/// Two ends of a selection, smaller first.
pub fn ordered_char_range(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// The characters from one end of a selection to the other.
pub fn slice_char_range(text: &str, a: usize, b: usize) -> &str {
    let (lo, hi) = ordered_char_range(a, b);
    let byte = |nth: usize| {
        text.char_indices()
            .nth(nth)
            .map_or(text.len(), |(at, _)| at)
    };
    &text[byte(lo)..byte(hi)]
}

/// The run of blank, or of non-blank, characters around `at`.
pub fn word_bounds(text: &str, at: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let at = at.min(chars.len() - 1);
    let blank = chars[at].is_whitespace();
    let same = |c: &char| c.is_whitespace() == blank;
    let before = chars[..at].iter().rev().take_while(|c| same(c)).count();
    let after = chars[at..].iter().take_while(|c| same(c)).count();
    (at - before, at + after)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "ab\ncd" laid out as two rows of two glyphs, 10 wide each, 5 tall.
    fn two_rows() -> (Vec<ScrollCell>, Vec<RunBounds>) {
        (
            vec![(7, 0), (7, 1), (9, 3), (9, 4)],
            vec![(0.0, 0.0, 20.0, 5.0), (0.0, 5.0, 20.0, 10.0)],
        )
    }

    #[test]
    fn a_pasted_command_loses_its_line_breaks() {
        assert_eq!(
            normalize_command_paste("map a\r\nwait 1\nquit"),
            "map a  wait 1 quit"
        );
    }

    #[test]
    fn copying_reports_what_happened() {
        assert_eq!(copy_prompt_selection(None, |_| Ok(())), PromptCopy::Ignored);
        assert_eq!(
            copy_prompt_selection(Some(""), |_| Ok(())),
            PromptCopy::Ignored
        );
        let mut got = String::new();
        let copied = copy_prompt_selection(Some("abc"), |text| {
            got = text.to_owned();
            Ok(())
        });
        assert_eq!((copied, got.as_str()), (PromptCopy::Copied, "abc"));
        let failed = copy_prompt_selection(Some("abc"), |_| Err("busy".to_owned()));
        assert_eq!(failed, PromptCopy::Failed("busy".to_owned()));
    }

    #[test]
    fn the_echo_for_each_copy_result() {
        assert_eq!(copy_prompt_echo(&PromptCopy::Ignored), None);
        assert_eq!(
            copy_prompt_echo(&PromptCopy::Copied).as_deref(),
            Some("Text copied")
        );
        assert_eq!(
            copy_prompt_echo(&PromptCopy::Failed("busy".into())).as_deref(),
            Some("clipboard write failed: busy")
        );
    }

    #[test]
    fn glyphs_skip_line_breaks_and_count_characters_not_bytes() {
        assert_eq!(map_glyphs_to_chars("a\né€", 9), [0, 2, 3]);
        assert_eq!(map_glyphs_to_chars("abc", 2), [0, 1]);
    }

    #[test]
    fn a_point_lands_on_a_row_and_column() {
        let (cells, runs) = two_rows();
        assert_eq!(hit_scroll_cell(&cells, &runs, (3.0, 1.0)), Some((0, 0)));
        assert_eq!(hit_scroll_cell(&cells, &runs, (12.0, 1.0)), Some((0, 1)));
        assert_eq!(hit_scroll_cell(&cells, &runs, (12.0, 7.0)), Some((1, 1)));
    }

    #[test]
    fn a_point_outside_the_text_lands_at_its_edge() {
        let (cells, runs) = two_rows();
        assert_eq!(hit_scroll_cell(&cells, &runs, (-5.0, 1.0)), Some((0, 0)));
        assert_eq!(hit_scroll_cell(&cells, &runs, (99.0, 1.0)), Some((0, 2)));
        assert_eq!(hit_scroll_cell(&cells, &runs, (3.0, 99.0)), Some((1, 0)));
        assert_eq!(hit_scroll_cell(&[], &runs, (1.0, 1.0)), None);
        assert_eq!(hit_scroll_cell(&cells, &[], (1.0, 1.0)), None);
    }

    #[test]
    fn a_column_is_the_character_it_draws() {
        let (cells, runs) = two_rows();
        assert_eq!(cell_char_index(&cells, &runs, 0, 1), Some(1));
        assert_eq!(
            cell_char_index(&cells, &runs, 0, 2),
            Some(2),
            "the line break"
        );
        assert_eq!(cell_char_index(&cells, &runs, 1, 0), Some(3));
        assert_eq!(cell_char_index(&cells, &runs, 1, 3), None);
        assert_eq!(cell_char_index(&cells, &runs, 2, 0), None);
    }

    #[test]
    fn a_row_covers_its_characters_and_its_break() {
        let (cells, runs) = two_rows();
        assert_eq!(line_char_range(&cells, &runs, 0), Some((0, 2)));
        assert_eq!(line_char_range(&cells, &runs, 1), Some((3, 5)));
        assert_eq!(line_char_range(&cells, &runs, 2), None);
    }

    #[test]
    fn a_selection_paints_the_part_of_each_row_it_covers() {
        let (cells, runs) = two_rows();
        // From the second glyph of row 0 into the first of row 1.
        let rects = cell_rects_for_char_range(1, 4, &cells, &runs, 50.0);
        assert_eq!(rects, [(10.0, 0.0, 50.0, 5.0), (0.0, 5.0, 10.0, 10.0)]);
    }

    #[test]
    fn a_selection_inside_one_row_stops_at_its_last_glyph() {
        let (cells, runs) = two_rows();
        assert_eq!(
            cell_rects_for_char_range(0, 1, &cells, &runs, 50.0),
            [(0.0, 0.0, 10.0, 5.0)]
        );
        assert!(cell_rects_for_char_range(2, 2, &cells, &runs, 50.0).is_empty());
        assert!(cell_rects_for_char_range(5, 9, &cells, &runs, 50.0).is_empty());
    }

    #[test]
    fn the_ends_of_a_selection_come_in_either_order() {
        assert_eq!(ordered_char_range(5, 2), (2, 5));
        assert_eq!(slice_char_range("a€bc", 3, 1), "€b");
        assert_eq!(slice_char_range("abc", 1, 1), "");
        assert_eq!(slice_char_range("abc", 1, 99), "bc");
    }

    #[test]
    fn a_word_is_a_run_of_the_same_kind_of_character() {
        assert_eq!(word_bounds("map  nuke", 1), (0, 3));
        assert_eq!(
            word_bounds("map  nuke", 3),
            (3, 5),
            "the blanks are a run too"
        );
        assert_eq!(word_bounds("map  nuke", 7), (5, 9));
        assert_eq!(word_bounds("map", 40), (0, 3));
        assert_eq!(word_bounds("", 0), (0, 0));
    }
}
