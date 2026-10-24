//! The line typed at the console prompt.

/// The line, the caret in it, and which completion is highlighted. The caret
/// counts characters, not bytes.
#[derive(Debug, Default, Clone)]
pub struct ConsoleEditor {
    pub line: String,
    pub caret: usize,
    pub selected: usize,
}

/// The byte where the `nth` character starts, or the end of the string.
fn byte_of(text: &str, nth: usize) -> usize {
    text.char_indices()
        .nth(nth)
        .map_or(text.len(), |(at, _)| at)
}

/// Where the token touching `caret` starts and ends, in characters. A token
/// is a run of characters that are not blank.
fn token_span(line: &str, caret: usize) -> (usize, usize) {
    let head: Vec<char> = line.chars().take(caret).collect();
    let back = head.iter().rev().take_while(|c| !c.is_whitespace()).count();
    let forward = line
        .chars()
        .skip(head.len())
        .take_while(|c| !c.is_whitespace())
        .count();
    (head.len() - back, head.len() + forward)
}

impl ConsoleEditor {
    /// Types `text` at the caret.
    pub fn insert(&mut self, text: &str) {
        self.line.insert_str(byte_of(&self.line, self.caret), text);
        self.caret += text.chars().count();
        self.selected = 0;
    }

    /// Deletes the word before the caret, and the blanks between it and the caret.
    pub fn kill_word(&mut self) {
        let head: Vec<char> = self.line.chars().take(self.caret).collect();
        let blanks = head.iter().rev().take_while(|c| c.is_whitespace()).count();
        let word = head
            .iter()
            .rev()
            .skip(blanks)
            .take_while(|c| !c.is_whitespace())
            .count();
        let start = head.len() - blanks - word;
        if start == head.len() {
            return;
        }
        self.line.replace_range(
            byte_of(&self.line, start)..byte_of(&self.line, head.len()),
            "",
        );
        self.caret = start;
        self.selected = 0;
    }

    /// Where the token under the caret starts and ends, and what it says.
    pub fn token_at_caret(&self) -> (usize, usize, String) {
        let (start, end) = token_span(&self.line, self.caret);
        (
            start,
            end,
            self.line.chars().skip(start).take(end - start).collect(),
        )
    }

    /// Puts the highlighted completion in place of the token under the caret,
    /// and a blank after it unless one is already there.
    pub fn accept(&mut self, completions: &[String]) -> bool {
        if completions.is_empty() {
            return false;
        }
        let chosen = &completions[self.selected % completions.len()];
        let (start, end) = token_span(&self.line, self.caret);
        self.line
            .replace_range(byte_of(&self.line, start)..byte_of(&self.line, end), chosen);
        self.caret = start + chosen.chars().count();
        if self.line.chars().nth(self.caret) != Some(' ') {
            self.insert(" ");
        }
        self.selected = 0;
        true
    }

    /// Hands over the line and starts an empty one.
    pub fn take(&mut self) -> String {
        self.caret = 0;
        self.selected = 0;
        std::mem::take(&mut self.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(line: &str, caret: usize) -> ConsoleEditor {
        ConsoleEditor {
            line: line.to_owned(),
            caret,
            selected: 0,
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn typing_goes_in_at_the_caret_by_characters() {
        let mut e = editor("é€x", 2);
        e.insert("ab");
        assert_eq!(e.line, "é€abx");
        assert_eq!(e.caret, 4);
    }

    #[test]
    fn typing_puts_the_highlight_back_at_the_top() {
        let mut e = editor("m", 1);
        e.selected = 3;
        e.insert("a");
        assert_eq!(e.selected, 0);
    }

    #[test]
    fn kill_word_takes_the_word_and_the_blanks_before_the_caret() {
        let mut e = editor("map  nuketown  ", 15);
        e.kill_word();
        assert_eq!(e.line, "map  ");
        assert_eq!(e.caret, 5);
        e.kill_word();
        assert_eq!(e.line, "");
        assert_eq!(e.caret, 0);
    }

    #[test]
    fn kill_word_in_the_middle_leaves_the_rest() {
        let mut e = editor("map nuketown now", 12);
        e.kill_word();
        assert_eq!(e.line, "map  now");
        assert_eq!(e.caret, 4);
    }

    #[test]
    fn kill_word_at_the_start_does_nothing() {
        let mut e = editor("map", 0);
        e.kill_word();
        assert_eq!(e.line, "map");
    }

    #[test]
    fn the_token_is_the_word_the_caret_touches() {
        assert_eq!(
            editor("map nuketown", 6).token_at_caret(),
            (4, 12, "nuketown".into())
        );
        assert_eq!(
            editor("map nuketown", 3).token_at_caret(),
            (0, 3, "map".into())
        );
        assert_eq!(editor("map ", 4).token_at_caret(), (4, 4, String::new()));
        assert_eq!(editor("", 0).token_at_caret(), (0, 0, String::new()));
    }

    #[test]
    fn accepting_replaces_the_token_and_adds_a_blank() {
        let mut e = editor("ma", 2);
        assert!(e.accept(&words(&["map", "maps"])));
        assert_eq!(e.line, "map ");
        assert_eq!(e.caret, 4);
    }

    #[test]
    fn accepting_takes_the_highlighted_one_and_wraps() {
        let mut e = editor("map n", 5);
        e.selected = 3;
        e.accept(&words(&["a", "b"]));
        assert_eq!(e.line, "map b ");
        assert_eq!(e.selected, 0);
    }

    #[test]
    fn accepting_in_the_middle_does_not_double_a_blank() {
        let mut e = editor("ma x", 2);
        e.accept(&words(&["map"]));
        assert_eq!(e.line, "map x");
        assert_eq!(e.caret, 3);
    }

    #[test]
    fn accepting_with_nothing_to_offer_changes_nothing() {
        let mut e = editor("ma", 2);
        assert!(!e.accept(&[]));
        assert_eq!(e.line, "ma");
    }

    #[test]
    fn taking_the_line_empties_the_editor() {
        let mut e = editor("quit", 4);
        e.selected = 2;
        assert_eq!(e.take(), "quit");
        assert_eq!((e.line.as_str(), e.caret, e.selected), ("", 0, 0));
    }
}
