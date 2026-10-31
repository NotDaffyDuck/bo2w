//! Mouse and clipboard support for the console: cleaning up pasted text,
//! copying the prompt's selection, and working out which characters of the
//! drawn scrollback a pointer or a drag covers.

use std::cmp::Ordering;

/// One drawn letter_shape of the scrollback: the id of the row it sits in and
/// the index of the character it draws.
#[derive(Clone, Copy)]
pub struct GlyphSpot {
    pub row_id: usize,
    pub ch: usize,
}

/// Edges of one drawn row of text, or of one highlight box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowBounds {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

/// Pasted text becomes one command line: each `\r` and `\n` turns into a blank.
pub fn tidy_pasted_orders(text: &str) -> String {
    text.chars().map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch }).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptClipboardPick {
    /// Nothing was selected, so nothing was copied.
    Ignored,
    /// The selection is on the clipboard.
    Copied,
    /// The clipboard refused it, for this reason.
    Refused(String),
}

/// Gives the selected text, if there is any, to `copy_out`.
pub fn clip_prompt_highlight(
    picked: Option<&str>,
    copy_out: impl FnOnce(&str) -> Result<(), String>,
) -> PromptClipboardPick {
    let Some(text) = picked.filter(|text| !text.is_empty()) else {
        return PromptClipboardPick::Ignored;
    };
    match copy_out(text) {
        Ok(()) => PromptClipboardPick::Copied,
        Err(reason) => PromptClipboardPick::Refused(reason),
    }
}

/// The line the console prints for a copy result; a copy of nothing prints
/// nothing.
pub fn copy_prompt_echo(result: &PromptClipboardPick) -> Option<String> {
    match *result {
        PromptClipboardPick::Copied => Some(String::from("Text copied")),
        PromptClipboardPick::Refused(ref reason) => Some(format!("clipboard write failed: {reason}")),
        PromptClipboardPick::Ignored => None,
    }
}

/// For each of the first `glyph_count` glyphs drawn for `shown`, the index of
/// the character it shows. Line breaks are not drawn, so they get no letter_shape.
pub fn map_glyphs_to_chars(shown: &str, glyph_count: usize) -> Vec<usize> {
    let mut drawn = Vec::with_capacity(glyph_count);
    for (index, ch) in shown.chars().enumerate() {
        if drawn.len() == glyph_count {
            break;
        }
        if ch != '\n' {
            drawn.push(index);
        }
    }
    drawn
}

/// The scrollback as drawn: for every row, the characters its glyphs show and
/// the box it fills. Rows are ordered by where their id first appears.
pub struct LogGrid {
    rows: Vec<Vec<usize>>,
    runs: Vec<RowBounds>,
}

impl LogGrid {
    pub fn new(cells: &[GlyphSpot], runs: Vec<RowBounds>) -> Self {
        let mut ids: Vec<usize> = Vec::new();
        let mut rows: Vec<Vec<usize>> = Vec::new();
        for &GlyphSpot { row_id: id, ch } in cells {
            let slot = match ids.iter().position(|&seen| seen == id) {
                Some(slot) => slot,
                None => {
                    ids.push(id);
                    rows.push(Vec::new());
                    rows.len() - 1
                }
            };
            rows[slot].push(ch);
        }
        Self { rows, runs }
    }

    /// Characters drawn in row `line`; none when the row has no box or no glyphs.
    fn line_cells(&self, line: usize) -> Option<&[usize]> {
        let has_box = line < self.runs.len();
        let chars = self.rows.get(line).filter(|_| has_box)?;
        (!chars.is_empty()).then_some(chars.as_slice())
    }

    /// Width each letter_shape of a row takes when the row's box is shared out evenly.
    fn step(run: &RowBounds, glyphs: usize) -> f32 {
        (run.x_max - run.x_min) / glyphs as f32
    }

    /// The row and column a point falls on. A point under the last row picks
    /// the last row; left of the text is column 0, and right of it is the
    /// column after the row's last letter_shape.
    pub fn hit(&self, (x, y): (f32, f32)) -> Option<(usize, usize)> {
        let Some(last) = self.runs.len().checked_sub(1).filter(|_| !self.rows.is_empty()) else { return None };
        let row = match self.runs.iter().position(|run| y <= run.y_max) {
            Some(found) => found,
            None => last,
        };
        let run = &self.runs[row];
        let glyphs = self.rows.get(row).map_or(0, |chars| chars.len());
        let step = Self::step(run, glyphs);
        if glyphs == 0 || step <= 0.0 {
            return Some((row, 0));
        }
        let column = ((x - run.x_min) / step).floor().clamp(0.0, glyphs as f32) as usize;
        Some((row, column))
    }

    /// The character drawn at a row and column. One column past the end of a
    /// row stands for the character after its last (its line break).
    pub fn char_at(&self, row: usize, col: usize) -> Option<usize> {
        let chars = self.line_cells(row)?;
        match chars.len().cmp(&col) {
            Ordering::Greater => Some(chars[col]),
            Ordering::Equal => Some(chars[chars.len() - 1] + 1),
            Ordering::Less => None,
        }
    }

    /// First character of a row, and the one after its last (so the span
    /// takes in the row's line break).
    pub fn row_span(&self, row: usize) -> Option<(usize, usize)> {
        let chars = self.line_cells(row)?;
        let (first, last) = (chars.first()?, chars.last()?);
        Some((*first, last + 1))
    }

    /// Right edge of the widest row, or 0 when nothing is drawn.
    pub fn widest(&self) -> f32 {
        let mut right = 0.0_f32;
        for run in &self.runs {
            right = right.max(run.x_max);
        }
        right
    }

    /// Highlight boxes for the characters `lo..hi`, one for each row the range
    /// touches. When the range runs on over a row's line break the box reaches
    /// out to `text_box_right`; the last row never does, as no break follows it.
    pub fn paint(&self, lo: usize, hi: usize, text_box_right: f32) -> Vec<RowBounds> {
        let mut boxes = Vec::new();
        let row_count = self.rows.len();
        for (index, (chars, run)) in self.rows.iter().zip(&self.runs).enumerate() {
            let (Some(&first), Some(&last)) = (chars.first(), chars.last()) else { continue };
            let line_break = last + 1;
            let outside = hi <= first || lo > line_break;
            if outside {
                continue;
            }
            let from = chars.iter().filter(|&&ch| ch < lo).count();
            let to = chars.iter().filter(|&&ch| ch < hi).count();
            let runs_on = index + 1 < row_count && lo <= line_break && hi > line_break;
            if to <= from && !runs_on {
                continue;
            }
            let step = Self::step(run, chars.len());
            let left = run.x_min + step * from as f32;
            let right = match runs_on {
                true => text_box_right,
                false => run.x_min + step * to as f32,
            };
            if left < right {
                boxes.push(RowBounds { x_min: left, y_min: run.y_min, x_max: right, y_max: run.y_max });
            }
        }
        boxes
    }
}

/// The ends of a selection, smaller first.
pub fn sorted_glyph_span(one: usize, other: usize) -> (usize, usize) {
    if other < one { (other, one) } else { (one, other) }
}

/// Byte offset where character `nth` of `text` starts, or the end of `text`.
fn char_offset(text: &str, nth: usize) -> usize {
    match text.char_indices().nth(nth) {
        Some((at, _)) => at,
        None => text.len(),
    }
}

/// The slice of `text` between two selection ends given in characters; ends
/// past the text stop at its end.
pub fn cut_glyph_span(text: &str, first: usize, last: usize) -> &str {
    let (from, to) = sorted_glyph_span(first, last);
    &text[char_offset(text, from)..char_offset(text, to)]
}

/// The run of blanks, or of non-blanks, that holds character `index` of
/// `line`, as `(first, one past last)`. An index past the end uses the last
/// character.
pub fn word_bounds(line: &str, index: usize) -> (usize, usize) {
    let letters: Vec<char> = line.chars().collect();
    let pivot = match letters.len() {
        0 => return (0, 0),
        count => index.min(count - 1),
    };
    let blank = letters[pivot].is_whitespace();
    let mut from = pivot;
    while from > 0 && letters[from - 1].is_whitespace() == blank {
        from -= 1;
    }
    let mut to = pivot;
    while to < letters.len() && letters[to].is_whitespace() == blank {
        to += 1;
    }
    (from, to)
}

#[cfg(test)]
mod tests {
    use crate::key_input::*;

    /// "ab\ncd" laid out as two rows of two glyphs, 10 wide each, 5 tall.
    fn two_rows() -> (Vec<GlyphSpot>, Vec<RowBounds>) {
        let spots = [(7, 0), (7, 1), (9, 3), (9, 4)];
        let edges = [[0.0, 0.0, 20.0, 5.0], [0.0, 5.0, 20.0, 10.0]];
        (spots.map(|(row_id, ch)| GlyphSpot { row_id, ch }).to_vec(), edges.map(edges_box).to_vec())
    }

    fn edges_box([left, top, right, bottom]: [f32; 4]) -> RowBounds {
        RowBounds { x_min: left, y_min: top, x_max: right, y_max: bottom }
    }

    fn grid() -> LogGrid {
        let (cells, runs) = two_rows();
        LogGrid::new(&cells, runs)
    }

    #[test]
    fn a_pasted_command_loses_its_line_breaks() {
        let expected = "map a  wait 1 quit";
        assert_eq!(tidy_pasted_orders("map a\r\nwait 1\nquit"), expected);
    }

    #[test]
    fn copying_reports_what_happened() {
        assert_eq!(clip_prompt_highlight(None, |_| Ok(())), PromptClipboardPick::Ignored);
        let expected = PromptClipboardPick::Ignored;
        assert_eq!(clip_prompt_highlight(Some(""), |_| Ok(())), expected);
        let mut got = String::new();
        let copied = clip_prompt_highlight(Some("abc"), |text| {
            got = text.to_owned();
            Ok(())
        });
        assert_eq!((copied, got.as_str()), (PromptClipboardPick::Copied, "abc"));
        let failed = clip_prompt_highlight(Some("abc"), |_| Err("busy".to_owned()));
        assert_eq!(failed, PromptClipboardPick::Refused("busy".to_owned()));
    }

    #[test]
    fn the_echo_for_each_copy_result() {
        assert_eq!(copy_prompt_echo(&PromptClipboardPick::Ignored), None);
        let expected = Some("Text copied");
        assert_eq!(copy_prompt_echo(&PromptClipboardPick::Copied).as_deref(), expected);
        let expected = Some("clipboard write failed: busy");
        assert_eq!(copy_prompt_echo(&PromptClipboardPick::Refused("busy".into())).as_deref(), expected);
    }

    #[test]
    fn glyphs_skip_line_breaks_and_count_characters_not_bytes() {
        assert_eq!(map_glyphs_to_chars("a\né€", 9), [0, 2, 3]);
        assert_eq!(map_glyphs_to_chars("abc", 2), [0, 1]);
    }

    #[test]
    fn a_point_lands_on_a_row_and_column() {
        let grid = grid();
        assert_eq!(grid.hit((3.0, 1.0)), Some((0, 0)));
        assert_eq!(grid.hit((12.0, 1.0)), Some((0, 1)));
        assert_eq!(grid.hit((12.0, 7.0)), Some((1, 1)));
    }

    #[test]
    fn a_point_outside_the_text_lands_at_its_edge() {
        let grid = grid();
        assert_eq!(grid.hit((-5.0, 1.0)), Some((0, 0)));
        assert_eq!(grid.hit((99.0, 1.0)), Some((0, 2)));
        assert_eq!(grid.hit((3.0, 99.0)), Some((1, 0)));
        let (cells, runs) = two_rows();
        assert_eq!(LogGrid::new(&[], runs).hit((1.0, 1.0)), None);
        assert_eq!(LogGrid::new(&cells, Vec::new()).hit((1.0, 1.0)), None);
    }

    #[test]
    fn a_column_is_the_character_it_draws() {
        let grid = grid();
        assert_eq!(grid.char_at(0, 1), Some(1));
        let expected = Some(2);
        assert_eq!(grid.char_at(0, 2), expected, "the line break");
        assert_eq!(grid.char_at(1, 0), Some(3));
        assert_eq!(grid.char_at(1, 3), None);
        assert_eq!(grid.char_at(2, 0), None);
    }

    #[test]
    fn a_row_covers_its_characters_and_its_break() {
        let grid = grid();
        assert_eq!(grid.row_span(0), Some((0, 2)));
        assert_eq!(grid.row_span(1), Some((3, 5)));
        assert_eq!(grid.row_span(2), None);
    }

    #[test]
    fn a_selection_paints_the_part_of_each_row_it_covers() {
        let grid = grid();
        // From the second letter_shape of row 0 into the first of row 1.
        let rects = grid.paint(1, 4, 50.0);
        assert_eq!(rects, [[10.0, 0.0, 50.0, 5.0], [0.0, 5.0, 10.0, 10.0]].map(edges_box));
    }

    #[test]
    fn a_selection_inside_one_row_stops_at_its_last_glyph() {
        let grid = grid();
        let expected = [edges_box([0.0, 0.0, 10.0, 5.0])];
        assert_eq!(grid.paint(0, 1, 50.0), expected);
        assert!(grid.paint(2, 2, 50.0).is_empty());
        assert!(grid.paint(5, 9, 50.0).is_empty());
    }

    #[test]
    fn the_ends_of_a_selection_come_in_either_order() {
        assert_eq!(sorted_glyph_span(5, 2), (2, 5));
        assert_eq!(cut_glyph_span("a€bc", 3, 1), "€b");
        assert_eq!(cut_glyph_span("abc", 1, 1), "");
        assert_eq!(cut_glyph_span("abc", 1, 99), "bc");
    }

    #[test]
    fn a_word_is_a_run_of_the_same_kind_of_character() {
        assert_eq!(word_bounds("map  nuke", 1), (0, 3));
        let expected = (3, 5);
        assert_eq!(word_bounds("map  nuke", 3), expected, "the blanks are a run too");
        assert_eq!(word_bounds("map  nuke", 7), (5, 9));
        assert_eq!(word_bounds("map", 40), (0, 3));
        assert_eq!(word_bounds("", 0), (0, 0));
    }
}
