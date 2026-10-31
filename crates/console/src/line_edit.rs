//! Editing state for the text the player types at the prompt.

/// What is typed so far, where the caret sits in it (counted in characters,
/// not bytes), and the index of the completion that is highlighted.
#[derive(Clone, Default, Debug)]
pub struct PromptLineEditor {
    pub line: String,
    pub insert_column: usize,
    pub highlight: usize,
}

/// Byte offset of character number `chars_in` of `text`; past the last
/// character this is the length of `text`.
fn char_to_byte(text: &str, chars_in: usize) -> usize {
    match text.char_indices().nth(chars_in) {
        Some((offset, _)) => offset,
        None => text.len(),
    }
}

/// The characters of `line` that come before `column`.
fn before_caret(line: &str, column: usize) -> Vec<char> {
    line.chars().take(column).collect()
}

/// Character range of the word (a run of non-blank characters) that the caret
/// at `column` is in or next to; an empty range at the caret when it is
/// between blanks.
fn word_around(line: &str, column: usize) -> (usize, usize) {
    let behind = before_caret(line, column);
    let caret = behind.len();
    let not_blank = |ch: &char| !ch.is_whitespace();
    let left = behind.iter().rev().copied().take_while(not_blank).count();
    let right = line.chars().skip(caret).take_while(not_blank).count();
    (caret - left, caret + right)
}

impl PromptLineEditor {
    /// Removes the characters in `from..to` and puts `with` there.
    fn splice(&mut self, from: usize, to: usize, with: &str) {
        let bytes = char_to_byte(&self.line, from)..char_to_byte(&self.line, to);
        self.line.replace_range(bytes, with);
    }

    /// Puts `typed` in at the caret and moves the caret past it.
    pub fn type_text(&mut self, typed: &str) {
        let column = self.insert_column;
        self.splice(column, column, typed);
        self.insert_column = column + typed.chars().count();
        self.highlight = 0;
    }

    /// Ctrl+Backspace: removes the blanks right before the caret and the word
    /// before those. Does nothing when the caret is at the start.
    pub fn erase_previous_word(&mut self) {
        let behind = before_caret(&self.line, self.insert_column);
        let caret = behind.len();
        let mut back = behind.iter().rev().peekable();
        let mut cut = 0;
        while back.next_if(|ch| ch.is_whitespace()).is_some() {
            cut += 1;
        }
        while back.next_if(|ch| !ch.is_whitespace()).is_some() {
            cut += 1;
        }
        if cut > 0 {
            let start = caret - cut;
            self.splice(start, caret, "");
            self.insert_column = start;
            self.highlight = 0;
        }
    }

    /// The word at the caret: its first character, the character after its
    /// last, and its text.
    pub fn word_at_caret(&self) -> (usize, usize, String) {
        let (start, end) = word_around(&self.line, self.insert_column);
        let said: String = self.line.chars().skip(start).take(end - start).collect();
        (start, end, said)
    }

    /// Tab: swaps the word at the caret for the highlighted completion (the
    /// highlight wraps round the list) and leaves the caret after it, with a
    /// blank typed there unless the line has one already. False when the list
    /// is empty.
    pub fn take_hint(&mut self, completions: &[String]) -> bool {
        let pick = match completions.len() {
            0 => return false,
            offered => completions[self.highlight % offered].as_str(),
        };
        let (start, end) = word_around(&self.line, self.insert_column);
        self.splice(start, end, pick);
        self.insert_column = start + pick.chars().count();
        let blank_follows = self.line.chars().nth(self.insert_column) == Some(' ');
        if !blank_follows {
            self.type_text(" ");
        }
        self.highlight = 0;
        true
    }

    /// Empties the editor and gives back what was typed.
    pub fn take_line(&mut self) -> String {
        let typed = std::mem::take(&mut self.line);
        self.insert_column = 0;
        self.highlight = 0;
        typed
    }
}

#[cfg(test)]
mod tests {
    use crate::line_edit::PromptLineEditor;

    fn at(line: &str, insert_column: usize) -> PromptLineEditor {
        let line = line.to_owned();
        PromptLineEditor { line, insert_column, ..Default::default() }
    }

    fn owned(list: &[&str]) -> Vec<String> {
        list.iter().copied().map(String::from).collect()
    }

    #[test]
    fn text_goes_in_at_the_caret_counted_in_characters() {
        let mut ed = at("é€x", 2);
        ed.type_text("ab");
        assert_eq!((ed.line.as_str(), ed.insert_column), ("é€abx", 4));
    }

    #[test]
    fn typing_moves_the_highlight_to_the_first_completion() {
        let mut ed = at("m", 1);
        ed.highlight = 3;
        ed.type_text("a");
        assert_eq!(ed.highlight, 0);
    }

    #[test]
    fn erasing_a_word_takes_the_blanks_before_the_caret_too() {
        let mut ed = at("map  nuketown  ", 15);
        ed.erase_previous_word();
        assert_eq!((ed.line.as_str(), ed.insert_column), ("map  ", 5));
        ed.erase_previous_word();
        assert_eq!((ed.line.as_str(), ed.insert_column), ("", 0));
    }

    #[test]
    fn erasing_a_word_mid_line_keeps_what_follows_the_caret() {
        let mut ed = at("map nuketown now", 12);
        ed.erase_previous_word();
        assert_eq!((ed.line.as_str(), ed.insert_column), ("map  now", 4));
    }

    #[test]
    fn erasing_at_the_start_of_the_line_does_nothing() {
        let mut ed = at("map", 0);
        ed.erase_previous_word();
        assert_eq!(ed.line, "map");
    }

    #[test]
    fn the_token_is_the_word_next_to_the_caret() {
        let word = |line: &str, column: usize| at(line, column).word_at_caret();
        assert_eq!(word("map nuketown", 6), (4, 12, "nuketown".to_owned()));
        assert_eq!(word("map nuketown", 3), (0, 3, "map".to_owned()));
        assert_eq!(word("map ", 4), (4, 4, "".to_owned()));
        assert_eq!(word("", 0), (0, 0, "".to_owned()));
    }

    #[test]
    fn accepting_swaps_in_the_completion_and_a_blank() {
        let mut ed = at("ma", 2);
        assert!(ed.take_hint(&owned(&["map", "maps"])));
        assert_eq!((ed.line.as_str(), ed.insert_column), ("map ", 4));
    }

    #[test]
    fn accepting_uses_the_highlight_wrapped_to_the_list() {
        let mut ed = at("map n", 5);
        ed.highlight = 3;
        ed.take_hint(&owned(&["a", "b"]));
        assert_eq!((ed.line.as_str(), ed.highlight), ("map b ", 0));
    }

    #[test]
    fn accepting_before_a_blank_adds_no_second_one() {
        let mut ed = at("ma x", 2);
        ed.take_hint(&owned(&["map"]));
        assert_eq!((ed.line.as_str(), ed.insert_column), ("map x", 3));
    }

    #[test]
    fn accepting_from_an_empty_list_leaves_the_line() {
        let mut ed = at("ma", 2);
        assert!(!ed.take_hint(&[]));
        assert_eq!(ed.line, "ma");
    }

    #[test]
    fn taking_the_line_resets_the_editor() {
        let mut ed = at("quit", 4);
        ed.highlight = 2;
        assert_eq!(ed.take_line(), "quit");
        assert_eq!((ed.line.as_str(), ed.insert_column, ed.highlight), ("", 0, 0));
    }
}
