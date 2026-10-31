//! Console command lines: how a typed or scripted line becomes commands.

use bevy::prelude::*;
use std::string::String;

/// One command, as the console runs it.
#[derive(Debug, Clone, PartialEq, Eq, Message)]
pub struct TypedOrder {
    pub name: String,
    pub words: Vec<String>,
    /// The line as written, without a leading `/`.
    pub as_written: String,
    /// A `&` ended the line: the command does not hold up the ones after it.
    pub detached: bool,
    /// A `!` followed the name, or ended the line.
    pub typed_by_hand: bool,
}

/// A word of a command line, and whether any of it was in quotes.
struct Word {
    spelled: String,
    quoted: bool,
}

/// Cuts a line into words. Quotes group, and `\\` and `\"` escape inside them.
fn split_words(line: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return words;
        }
        let mut word = Word {
            spelled: String::default(),
            quoted: false,
        };
        let mut inside = false;
        while let Some(c) = chars.next_if(|c| inside || !c.is_whitespace()) {
            match c {
                '"' => {
                    inside = !inside;
                    word.quoted = true;
                }
                '\\' if inside => match chars.next_if(|n| matches!(n, '\\' | '"')) {
                    Some(escaped) => word.spelled.push(escaped),
                    None => word.spelled.push('\\'),
                },
                other => word.spelled.push(other),
            }
        }
        words.push(word);
    }
}

/// Drops the last word if it is exactly `mark` and was not quoted.
fn take_mark(words: &mut Vec<Word>, mark: &str) -> bool {
    let marked = words
        .last()
        .is_some_and(|word| !word.quoted && word.spelled == mark);
    if marked {
        words.truncate(words.len() - 1);
    }
    marked
}

/// Cuts a script at each `;` that is not inside quotes.
pub(crate) fn split_script(script: &str) -> Vec<&str> {
    let mut parts = vec![];
    let mut from: usize = 0;
    let mut inside = false;
    let mut chars = script.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => inside = !inside,
            '\\' if inside => {
                chars.next_if(|&(_, next)| matches!(next, '\\' | '"'));
            }
            ';' if !inside => {
                parts.push(&script[from..at]);
                from = at + 1;
            }
            _ => {}
        }
    }
    parts.push(&script[from..]);
    parts
}

impl TypedOrder {
    /// One command from one line. A `;` is not a break here; see
    /// [`TypedOrder::parse_script`].
    pub fn parse(text: &str) -> Option<Self> {
        let line = text.trim();
        let raw = line.strip_prefix('/').unwrap_or(line).trim();
        let mut words = split_words(raw);
        let ampersand = take_mark(&mut words, "&");
        let bang_word = take_mark(&mut words, "!");
        let mut words = words.into_iter().map(|word| word.spelled);
        let mut name: String = words.next()?;
        let forced_order_name = name.len() > 1 && name.ends_with('!');
        if forced_order_name {
            name.truncate(name.len() - 1);
        }
        Some(Self {
            name,
            words: words.collect::<Vec<_>>(),
            as_written: raw.to_owned(),
            detached: ampersand,
            typed_by_hand: bang_word || forced_order_name,
        })
    }

    /// Every command of a line that holds several, split at `;`.
    pub fn parse_script(script: &str) -> Vec<Self> {
        split_script(script)
            .iter()
            .copied()
            .filter_map(Self::parse)
            .collect::<Vec<TypedOrder>>()
    }
}

/// Lines waiting to be run: the startup script, stdin, the log of a replay.
#[derive(Resource, Debug, Default)]
pub struct OrderInbox {
    pending: Vec<TypedOrder>,
}

impl OrderInbox {
    pub fn enqueue_typed_text(&mut self, typed: &str) {
        self.pending.extend(TypedOrder::parse_script(typed));
    }

    pub fn drain(&mut self) -> Vec<TypedOrder> {
        self.pending.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::command::*;

    fn parsed(line: &str) -> TypedOrder {
        TypedOrder::parse(line).expect("a command")
    }

    #[test]
    fn a_name_and_its_arguments() {
        let command = parsed("map  mp_nuketown   2");
        assert_eq!(command.name, "map");
        assert_eq!(command.words, ["mp_nuketown", "2"]);
        assert!(!command.detached && !command.typed_by_hand);
    }

    #[test]
    fn a_leading_slash_is_not_part_of_the_line() {
        let command = parsed("  /quit now ");
        assert_eq!(command.name, "quit");
        assert_eq!(command.as_written, "quit now");
    }

    #[test]
    fn nothing_to_run_is_not_a_command() {
        for line in ["", "   ", "/", " / ", "&", "!", "! &"] {
            assert_eq!(TypedOrder::parse(line), None, "{line:?}");
        }
    }

    #[test]
    fn quotes_join_words_and_escapes_work_inside_them() {
        let command = parsed(r#"say "hello   world" "a \"b\" c" "back\\slash" "tab\t""#);
        let expected = ["hello   world", r#"a "b" c"#, r"back\slash", r"tab\t"];
        assert_eq!(command.words, expected);
    }

    #[test]
    fn an_empty_pair_of_quotes_is_an_empty_argument() {
        assert_eq!(parsed(r#"name """#).words, [""]);
    }

    #[test]
    fn a_trailing_ampersand_runs_it_in_the_background() {
        let command = parsed("spawn &");
        assert!(command.detached);
        assert_eq!(command.name, "spawn");
        assert!(command.words.is_empty());
    }

    #[test]
    fn a_quoted_ampersand_is_an_argument() {
        let command = parsed(r#"say "&""#);
        assert!(!command.detached);
        assert_eq!(command.words, ["&"]);
    }

    #[test]
    fn a_bang_after_the_name_or_at_the_end_asks_for_interaction() {
        assert!(parsed("map!").typed_by_hand);
        assert_eq!(parsed("map!").name, "map");
        assert!(parsed("map a !").typed_by_hand);
        assert_eq!(parsed("map a !").words, ["a"]);
        let both = parsed("map a ! &");
        assert!(both.typed_by_hand && both.detached);
        assert_eq!(both.words, ["a"]);
    }

    #[test]
    fn a_bang_alone_is_nothing_and_a_leading_bang_stays_in_the_name() {
        assert_eq!(TypedOrder::parse("!"), None);
        assert_eq!(parsed("!x").name, "!x");
        assert!(!parsed("!x").typed_by_hand);
    }

    #[test]
    fn a_script_is_cut_at_semicolons_outside_quotes() {
        let names: Vec<String> = TypedOrder::parse_script(r#"say "a;b"; map x ;; quit"#)
            .iter()
            .map(|command| command.as_written.clone())
            .collect::<Vec<String>>();
        assert_eq!(names, [r#"say "a;b""#, "map x", "quit"]);
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_quotes_for_the_script() {
        let commands = TypedOrder::parse_script(r#"say "a \"; b"; quit"#);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].words, [r#"a "; b"#]);
    }

    #[test]
    fn parse_leaves_a_semicolon_alone() {
        assert_eq!(parsed("a; b").name, "a;");
    }

    #[test]
    fn the_queue_keeps_lines_in_order_and_empties_when_drained() {
        let mut queue = OrderInbox::default();
        queue.enqueue_typed_text("map a; wait 2");
        queue.enqueue_typed_text("quit");
        let names: Vec<String> = queue.drain().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["map", "wait", "quit"]);
        assert!(queue.drain().is_empty());
    }
}
