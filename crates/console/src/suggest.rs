//! The list of completions under the prompt, as pieces of text to draw.

/// How many completions show at once.
pub const SUGGEST_WINDOW: usize = 8;

/// How a piece of the list is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestTone {
    /// Spaces that line the list up under the prompt, and the breaks between rows.
    Indent,
    /// The part of a completion the player has already typed.
    Matched,
    /// The rest of the highlighted completion.
    RestSelected,
    /// The rest of any other completion.
    RestMuted,
    /// The mark that says more completions follow.
    Ellipsis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestSpan {
    pub text: String,
    pub tone: SuggestTone,
}

fn span(text: impl Into<String>, tone: SuggestTone) -> SuggestSpan {
    SuggestSpan {
        text: text.into(),
        tone,
    }
}

/// The typed part of `value` and what is left, when `value` starts with
/// `token` in either case.
fn split_typed<'a>(value: &'a str, token: &str) -> (&'a str, &'a str) {
    let cut = value
        .char_indices()
        .nth(token.chars().count())
        .map_or(value.len(), |(at, _)| at);
    if !token.is_empty() && value[..cut].eq_ignore_ascii_case(token) {
        value.split_at(cut)
    } else {
        ("", value)
    }
}

/// The list as drawn from the top: the highlighted completion and up to
/// [`SUGGEST_WINDOW`] − 1 after it, printed bottom-up so the highlighted one
/// sits next to the prompt, under an ellipsis when more follow.
pub fn suggestion_spans(
    suggestions: &[String],
    selected: usize,
    token: &str,
    indent_cols: usize,
) -> Vec<SuggestSpan> {
    if suggestions.is_empty() {
        return Vec::new();
    }
    let first = selected % suggestions.len();
    let last = (first + SUGGEST_WINDOW).min(suggestions.len());
    let indent = " ".repeat(indent_cols);
    let mut spans = Vec::new();
    if last < suggestions.len() {
        spans.push(span(format!("{indent}…\n"), SuggestTone::Ellipsis));
    }
    for index in (first..last).rev() {
        if !indent.is_empty() {
            spans.push(span(indent.as_str(), SuggestTone::Indent));
        }
        let (typed, rest) = split_typed(&suggestions[index], token);
        if !typed.is_empty() {
            spans.push(span(typed, SuggestTone::Matched));
        }
        if !rest.is_empty() {
            let tone = if index == first {
                SuggestTone::RestSelected
            } else {
                SuggestTone::RestMuted
            };
            spans.push(span(rest, tone));
        }
        if index != first {
            spans.push(span("\n", SuggestTone::Indent));
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::SuggestTone::*;
    use super::*;

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("map{n}")).collect()
    }

    fn text(spans: &[SuggestSpan]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn no_completions_draws_nothing() {
        assert!(suggestion_spans(&[], 3, "m", 2).is_empty());
    }

    #[test]
    fn the_highlighted_row_comes_last_so_it_sits_next_to_the_prompt() {
        let spans = suggestion_spans(&names(3), 0, "map", 0);
        assert_eq!(text(&spans), "map2\nmap1\nmap0");
        assert_eq!(spans.last().map(|s| s.tone), Some(RestSelected));
    }

    #[test]
    fn the_typed_part_is_marked_and_the_rest_is_muted_unless_highlighted() {
        let spans = suggestion_spans(&["quit".to_owned(), "quote".to_owned()], 0, "QUI", 0);
        let tones: Vec<_> = spans.iter().map(|s| (s.text.as_str(), s.tone)).collect();
        assert_eq!(
            tones,
            [
                ("quote", RestMuted),
                (
                    "
", Indent
                ),
                ("qui", Matched),
                ("t", RestSelected),
            ]
        );
    }

    #[test]
    fn a_completion_keeps_its_own_case_where_it_matches() {
        let spans = suggestion_spans(&["Quit".to_owned()], 0, "qu", 0);
        assert_eq!(spans[0].text, "Qu");
        assert_eq!(spans[0].tone, Matched);
    }

    #[test]
    fn a_long_list_shows_a_window_and_an_ellipsis() {
        let spans = suggestion_spans(&names(20), 0, "map", 2);
        assert_eq!(spans[0], span("  …\n", Ellipsis));
        let rows = text(&spans).lines().count();
        assert_eq!(rows, SUGGEST_WINDOW + 1);
    }

    #[test]
    fn the_window_ends_at_the_list_without_an_ellipsis() {
        let spans = suggestion_spans(&names(20), 15, "map", 0);
        assert!(spans.iter().all(|s| s.tone != Ellipsis));
        assert_eq!(text(&spans).lines().count(), 5);
        assert!(text(&spans).ends_with("map15"));
    }

    #[test]
    fn a_selection_past_the_end_wraps() {
        let spans = suggestion_spans(&names(3), 4, "map", 0);
        assert!(text(&spans).ends_with("map1"));
    }

    #[test]
    fn an_indent_starts_every_row() {
        let spans = suggestion_spans(&names(2), 0, "", 3);
        assert_eq!(text(&spans), "   map1\n   map0");
        assert_eq!(spans[0], span("   ", Indent));
    }

    #[test]
    fn an_empty_token_marks_nothing_as_typed() {
        let spans = suggestion_spans(&names(1), 0, "", 0);
        assert_eq!(spans, [span("map0", RestSelected)]);
    }
}
