//! What the console remembers between frames: its size, whether it is open,
//! the text it has printed, and the lines it has been given.

use bevy::prelude::*;

use crate::ConsoleEditor;

/// The most lines the history keeps.
const HISTORY_LIMIT: usize = 100;

#[derive(Resource, Clone)]
pub struct ConsoleSettings {
    /// Lines of scrollback kept.
    pub log_capacity: usize,
    pub height: f32,
    pub width: f32,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            log_capacity: 12,
            height: 360.0,
            width: 780.0,
        }
    }
}

#[derive(Resource)]
pub struct ConsoleFont(pub Handle<Font>);

#[derive(Resource, Default)]
pub struct ConsoleState {
    pub open: bool,
    pub editor: ConsoleEditor,
    pub log: Vec<String>,
    pub history: Vec<String>,
    /// The history line shown in the prompt while the arrow keys browse it.
    pub(super) history_cursor: Option<usize>,
    /// What was in the prompt before browsing began.
    pub(super) history_draft: String,
    /// The scrollback selection: where the drag began, and where it is now.
    pub(super) scroll_anchor: Option<usize>,
    pub(super) scroll_focus: usize,
    /// A press landed on the scrollback, so the release copies from there.
    pub(super) scroll_gesture: bool,
    /// Text for `con_feed` to type into the prompt on the next frame.
    pub(super) pending_feed: Option<String>,
    /// `con_paste` asked for the OS clipboard.
    pub(super) pending_os_paste: bool,
}

/// Puts `line` in the prompt with the caret after it.
pub(super) fn set_line(editor: &mut ConsoleEditor, line: &str) {
    editor.line = line.to_owned();
    editor.caret = line.chars().count();
    editor.selected = 0;
}

impl ConsoleState {
    /// Prints a line, forgetting the oldest ones beyond `capacity`.
    pub fn echo(&mut self, line: impl Into<String>, capacity: usize) {
        self.log.push(line.into());
        let extra = self.log.len().saturating_sub(capacity);
        self.log.drain(..extra);
    }

    /// Adds a submitted line to the history unless it repeats the last one.
    pub(super) fn remember(&mut self, line: &str) {
        if self.history.last().is_some_and(|last| last == line) {
            return;
        }
        self.history.push(line.to_owned());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }

    pub(super) fn leave_history_browse(&mut self) {
        self.history_cursor = None;
        self.history_draft.clear();
    }

    /// Steps through the history (`older` goes back) and puts the line in
    /// the prompt. False when the keys should do something else: nothing to
    /// browse, or a prompt with text in it that is not yet browsing.
    pub(super) fn history_recall(&mut self, older: bool) -> bool {
        let Some(newest) = self.history.len().checked_sub(1) else {
            return false;
        };
        let target = match (self.history_cursor, older) {
            (None, true) if self.editor.line.is_empty() => {
                self.history_draft = std::mem::take(&mut self.editor.line);
                newest
            }
            (None, _) => return false,
            (Some(0), true) => return true,
            (Some(at), true) => at - 1,
            (Some(at), false) if at >= newest => {
                let draft = std::mem::take(&mut self.history_draft);
                self.history_cursor = None;
                set_line(&mut self.editor, &draft);
                return true;
            }
            (Some(at), false) => at + 1,
        };
        self.history_cursor = Some(target);
        let line = self.history[target].clone();
        set_line(&mut self.editor, &line);
        true
    }

    pub(super) fn clear_scroll_selection(&mut self) {
        self.scroll_anchor = None;
        self.scroll_focus = 0;
        self.scroll_gesture = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_history(lines: &[&str]) -> ConsoleState {
        let mut state = ConsoleState::default();
        for line in lines {
            state.remember(line);
        }
        state
    }

    #[test]
    fn echo_keeps_only_the_newest_lines() {
        let mut state = ConsoleState::default();
        for n in 0..5 {
            state.echo(format!("line {n}"), 3);
        }
        assert_eq!(state.log, ["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn history_skips_a_repeat_and_stops_at_a_hundred() {
        let mut state = with_history(&["map a", "map a", "map b", "map a"]);
        assert_eq!(state.history, ["map a", "map b", "map a"]);
        for n in 0..150 {
            state.remember(&format!("cmd {n}"));
        }
        assert_eq!(state.history.len(), HISTORY_LIMIT);
        assert_eq!(state.history.last().map(String::as_str), Some("cmd 149"));
    }

    #[test]
    fn up_arrow_browses_back_and_down_returns_to_the_draft() {
        let mut state = with_history(&["one", "two", "three"]);
        assert!(state.history_recall(true));
        assert_eq!(state.editor.line, "three");
        assert!(state.history_recall(true));
        assert_eq!(state.editor.line, "two");
        assert!(state.history_recall(true));
        assert!(state.history_recall(true), "the oldest line stays put");
        assert_eq!(state.editor.line, "one");
        assert!(state.history_recall(false));
        assert!(state.history_recall(false));
        assert_eq!(state.editor.line, "three");
        assert!(state.history_recall(false));
        assert_eq!(state.editor.line, "");
        assert_eq!(state.history_cursor, None);
    }

    #[test]
    fn the_arrows_leave_a_typed_line_alone() {
        let mut state = with_history(&["one"]);
        set_line(&mut state.editor, "ma");
        assert!(!state.history_recall(true));
        assert!(!state.history_recall(false));
        assert_eq!(state.editor.line, "ma");
        assert!(!ConsoleState::default().history_recall(true));
    }

    #[test]
    fn set_line_puts_the_caret_after_the_text() {
        let mut editor = ConsoleEditor::default();
        set_line(&mut editor, "héllo");
        assert_eq!((editor.caret, editor.selected), (5, 0));
    }

    #[test]
    fn clearing_the_selection_forgets_the_drag() {
        let mut state = ConsoleState {
            scroll_anchor: Some(4),
            scroll_focus: 9,
            scroll_gesture: true,
            ..Default::default()
        };
        state.clear_scroll_selection();
        assert_eq!(
            (
                state.scroll_anchor,
                state.scroll_focus,
                state.scroll_gesture
            ),
            (None, 0, false)
        );
    }
}
