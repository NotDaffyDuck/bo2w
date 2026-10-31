//! What the console remembers between frames: its size, whether it is open,
//! the text it has printed, and the lines it has been given.

use bevy::ecs::resource::Resource;
use bevy::text::Font;

use crate::PromptLineEditor;
use bevy::asset::Handle;

/// The most lines the history keeps.
const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Resource)]
pub struct PromptPanelOptions {
    /// Lines of scrollback kept.
    pub backlog_limit: usize,
    pub panel_height: f32,
    pub panel_width: f32,
}

impl Default for PromptPanelOptions {
    fn default() -> PromptPanelOptions {
        Self {
            backlog_limit: 12,
            panel_height: 360.0,
            panel_width: 780.0,
        }
    }
}

#[derive(Resource)]
pub struct PromptTypeface(pub Handle<Font>);

#[derive(Default, Resource)]
pub struct PromptPanelStatus {
    pub shown: bool,
    pub editor: PromptLineEditor,
    pub(crate) lines: Vec<String>,
    pub recalled: Vec<String>,
    /// The history line shown in the prompt while the arrow keys browse it.
    pub(super) recall_position: Option<usize>,
    /// What was in the prompt before browsing began.
    pub(super) recall_scratch_text: String,
    /// The scrollback selection: where the drag began, and where it is now.
    pub(super) scroll_anchor: Option<usize>,
    pub(super) backlog_anchor: usize,
    /// A press landed on the scrollback, so the release copies from there.
    pub(super) wheel_dragging: bool,
    /// Text for `con_feed` to type into the prompt on the next frame.
    pub(super) pending_feed: Option<String>,
    /// `con_paste` asked for the OS clipboard.
    pub(super) pending_os_paste: bool,
}

/// Puts `typed` in the prompt with the insert_column after it.
pub(super) fn set_line(editor: &mut PromptLineEditor, typed: &str) {
    editor.line = String::from(typed);
    editor.insert_column = typed.chars().count();
    editor.highlight = 0;
}

impl PromptPanelStatus {
    /// Prints a line, forgetting the oldest ones beyond `limit`.
    pub fn post_line(&mut self, text: impl Into<String>, limit: usize) {
        self.lines.push(text.into());
        let extra = self.lines.len().saturating_sub(limit);
        self.lines.drain(..extra);
    }

    /// Adds a submitted line to the history unless it repeats the last one.
    pub(super) fn remember(&mut self, submitted: &str) {
        if self.recalled.last().is_some_and(|last| last == submitted) {
            return;
        }
        self.recalled.push(submitted.to_owned());
        if self.recalled.len() > HISTORY_LIMIT {
            self.recalled.remove(0);
        }
    }

    pub(super) fn quit_recall_mode(&mut self) {
        self.recall_position = None;
        self.recall_scratch_text.clear();
    }

    /// Steps through the history (`back` goes to older lines) and puts the line in
    /// the prompt. False when the keys should do something else: nothing to
    /// browse, or a prompt with text in it that is not yet browsing.
    pub(super) fn recall_earlier_entry(&mut self, back: bool) -> bool {
        let newest = match self.recalled.len() {
            0 => return false,
            count => count - 1,
        };
        let target = match (self.recall_position, back) {
            (None, true) if self.editor.line.is_empty() => {
                self.recall_scratch_text = std::mem::take(&mut self.editor.line);
                newest
            }
            (None, _) => return false,
            (Some(0), true) => return true,
            (Some(at), true) => at - 1,
            (Some(at), false) if at >= newest => return self.restore_scratch(),
            (Some(at), false) => at + 1,
        };
        self.recall_position = Some(target);
        let line = self.recalled[target].clone();
        set_line(&mut self.editor, &line);
        true
    }

    /// Browsing ran past the newest line: the prompt gets back what it held.
    fn restore_scratch(&mut self) -> bool {
        let draft = std::mem::take(&mut self.recall_scratch_text);
        self.recall_position = None;
        set_line(&mut self.editor, &draft);
        true
    }

    pub(super) fn clear_scroll_selection(&mut self) {
        self.scroll_anchor.take();
        self.backlog_anchor = 0;
        self.wheel_dragging = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::app::state::*;

    fn with_history(lines: &[&str]) -> PromptPanelStatus {
        let mut state = PromptPanelStatus::default();
        for entry in lines {
            state.remember(entry);
        }
        state
    }

    #[test]
    fn echo_keeps_only_the_newest_lines() {
        let mut state = PromptPanelStatus::default();
        for n in 0..5 {
            state.post_line(format!("line {n}"), 3);
        }
        assert_eq!(state.lines, ["line 2", "line 3", "line 4"]);
    }

    #[test]
    fn history_skips_a_repeat_and_stops_at_a_hundred() {
        let mut state = with_history(&["map a", "map a", "map b", "map a"]);
        assert_eq!(state.recalled, ["map a", "map b", "map a"]);
        for n in 0..150 {
            state.remember(&format!("cmd {n}"));
        }
        assert_eq!(state.recalled.len(), HISTORY_LIMIT);
        assert_eq!(state.recalled.last().map(String::as_str), Some("cmd 149"));
    }

    #[test]
    fn up_arrow_browses_back_and_down_returns_to_the_draft() {
        let mut state = with_history(&["one", "two", "three"]);
        assert!(state.recall_earlier_entry(true));
        assert_eq!(state.editor.line, "three");
        assert!(state.recall_earlier_entry(true));
        assert_eq!(state.editor.line, "two");
        assert!(state.recall_earlier_entry(true));
        assert!(state.recall_earlier_entry(true), "the oldest line stays put");
        assert_eq!(state.editor.line, "one");
        assert!(state.recall_earlier_entry(false));
        assert!(state.recall_earlier_entry(false));
        assert_eq!(state.editor.line, "three");
        assert!(state.recall_earlier_entry(false));
        assert_eq!(state.editor.line, "");
        assert_eq!(state.recall_position, None);
    }

    #[test]
    fn the_arrows_leave_a_typed_line_alone() {
        let mut state = with_history(&["one"]);
        set_line(&mut state.editor, "ma");
        assert!(!state.recall_earlier_entry(true));
        assert!(!state.recall_earlier_entry(false));
        assert_eq!(state.editor.line, "ma");
        assert!(!PromptPanelStatus::default().recall_earlier_entry(true));
    }

    #[test]
    fn set_line_puts_the_caret_after_the_text() {
        let mut editor = PromptLineEditor::default();
        set_line(&mut editor, "héllo");
        assert_eq!((editor.insert_column, editor.highlight), (5, 0));
    }

    #[test]
    fn clearing_the_selection_forgets_the_drag() {
        let mut state = PromptPanelStatus {
            scroll_anchor: Some(4),
            backlog_anchor: 9,
            wheel_dragging: true,
            ..Default::default()
        };
        state.clear_scroll_selection();
        let expected = (None, 0, false);
        assert_eq!((
            state.scroll_anchor,
            state.backlog_anchor,
            state.wheel_dragging
        ), expected);
    }
}
