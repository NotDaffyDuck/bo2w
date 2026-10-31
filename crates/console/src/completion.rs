//! Turns the prompt's completion list into coloured runs of text for the panel.

/// Rows of completions the panel draws before it cuts the list off.
pub const HINT_ROWS_SHOWN: usize = 8;

/// The colour role of one run of text in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintShade {
    /// Leading blanks under the prompt, and the line break after a row.
    Indent,
    /// Letters of a completion that the player has typed already.
    Matched,
    /// Untyped letters of the highlighted completion.
    RemainderPicked,
    /// Untyped letters of every other completion.
    RestMuted,
    /// The top marker shown when the list goes on past the window.
    Ellipsis,
}

/// One run of text and the role it is drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintRange {
    pub fill: String,
    pub tone: HintShade,
}

impl HintRange {
    fn of(tone: HintShade, text: impl Into<String>) -> Self {
        let text = text.into();
        Self { fill: text, tone }
    }
}

/// Byte index after the first `chars` characters of `value`, or its length.
fn byte_after(value: &str, chars: usize) -> usize {
    match value.char_indices().nth(chars) {
        Some((at, _)) => at,
        None => value.len(),
    }
}

/// Splits `value` into the piece the player typed and the piece still to come.
/// Nothing counts as typed unless `token` is non-empty and `value` begins with
/// it, ignoring ASCII case.
fn typed_and_rest<'v>(value: &'v str, token: &str) -> (&'v str, &'v str) {
    let at = byte_after(value, token.chars().count());
    let (head, tail) = value.split_at(at);
    if !token.is_empty() && head.eq_ignore_ascii_case(token) { (head, tail) } else { ("", value) }
}

/// Runs for the visible part of `hint_list`. The window opens at the
/// highlighted entry (`highlight`, wrapped to the list) and holds at most
/// [`HINT_ROWS_SHOWN`] entries. Rows come out last-to-first so the highlighted
/// one ends up beside the prompt; an ellipsis row heads the list when entries
/// lie beyond the window.
pub fn suggestion_spans(hint_list: &[String], highlight: usize, stem: &str, indent_cols: usize) -> Vec<HintRange> {
    if hint_list.is_empty() {
        return vec![];
    }
    let total = hint_list.len();
    let top = highlight % total;
    let end = total.min(top + HINT_ROWS_SHOWN);
    let pad = " ".repeat(indent_cols);

    // Empty pieces (no indent, nothing typed, nothing left) are left out.
    let mut runs = Vec::new();
    let mut put = |tone, text: &str| {
        let runs_wanted = !text.is_empty();
        if runs_wanted {
            runs.push(HintRange::of(tone, text));
        }
    };
    if end != total {
        put(HintShade::Ellipsis, &format!("{pad}…\n"));
    }
    for (row, entry) in hint_list[top..end].iter().enumerate().rev() {
        let picked = row == 0;
        let (typed, rest) = typed_and_rest(entry, stem);
        put(HintShade::Indent, &pad);
        put(HintShade::Matched, typed);
        put(if picked { HintShade::RemainderPicked } else { HintShade::RestMuted }, rest);
        if !picked {
            put(HintShade::Indent, "\n");
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use crate::completion::HintShade::{Ellipsis, Indent, Matched, RemainderPicked, RestMuted};
    use crate::completion::*;

    fn maps(how_many: usize) -> Vec<String> {
        (0..how_many).map(|n| format!("map{n}")).collect()
    }

    fn joined(runs: &[HintRange]) -> String {
        runs.iter().map(|run| run.fill.as_str()).collect()
    }

    #[test]
    fn an_empty_list_gives_no_runs() {
        assert!(suggestion_spans(&[], 3, "m", 2).is_empty());
    }

    #[test]
    fn the_highlighted_row_is_drawn_last_beside_the_prompt() {
        let runs = suggestion_spans(&maps(3), 0, "map", 0);
        assert_eq!(joined(&runs), "map2\nmap1\nmap0");
        let last_tone = runs.last().map(|run| run.tone);
        assert_eq!(last_tone, Some(RemainderPicked));
    }

    #[test]
    fn typed_letters_are_marked_and_other_rows_are_muted() {
        let list = ["quit".to_owned(), "quote".to_owned()];
        let runs = suggestion_spans(&list, 0, "QUI", 0);
        let actual: Vec<_> = runs.iter().map(|run| (run.fill.as_str(), run.tone)).collect();
        let wanted = [("quote", RestMuted), ("\n", Indent), ("qui", Matched), ("t", RemainderPicked)];
        assert_eq!(actual, wanted);
    }

    #[test]
    fn the_typed_part_keeps_the_completions_own_case() {
        let runs = suggestion_spans(&["Quit".to_owned()], 0, "qu", 0);
        assert_eq!((runs[0].fill.as_str(), runs[0].tone), ("Qu", Matched));
    }

    #[test]
    fn a_long_list_is_cut_to_a_window_under_an_ellipsis() {
        let runs = suggestion_spans(&maps(20), 0, "map", 2);
        assert_eq!(runs[0], HintRange::of(Ellipsis, "  …\n"));
        let line_count = joined(&runs).lines().count();
        assert_eq!(line_count, HINT_ROWS_SHOWN + 1);
    }

    #[test]
    fn a_window_that_reaches_the_end_has_no_ellipsis() {
        let runs = suggestion_spans(&maps(20), 15, "map", 0);
        assert!(!runs.iter().any(|run| run.tone == Ellipsis));
        let text = joined(&runs);
        assert_eq!(text.lines().count(), 5);
        assert!(text.ends_with("map15"));
    }

    #[test]
    fn a_highlight_past_the_end_wraps_round() {
        let runs = suggestion_spans(&maps(3), 4, "map", 0);
        assert!(joined(&runs).ends_with("map1"));
    }

    #[test]
    fn every_row_starts_with_the_indent() {
        let runs = suggestion_spans(&maps(2), 0, "", 3);
        assert_eq!(joined(&runs), "   map1\n   map0");
        assert_eq!(runs[0], HintRange::of(Indent, "   "));
    }

    #[test]
    fn nothing_is_typed_when_the_token_is_empty() {
        let runs = suggestion_spans(&maps(1), 0, "", 0);
        assert_eq!(runs, [HintRange::of(RemainderPicked, "map0")]);
    }
}
