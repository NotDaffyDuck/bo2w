//! Input line handling: key editing, history and completion navigation, and
//! what a submitted line does.

use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::keyboard::Key;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::keyboard::NativeKeyCode;
use bevy::input_focus::FocusCause;
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::text::FontCx;
use bevy::text::LayoutCx;
use bevy::text::TextEdit;

use super::panel::INPUT_LEAD_GLYPH;
use super::panel::PromptEntity;
use super::panel::PromptText;
use super::queue::TypedOrderBacklog;
use super::state::PromptPanelOptions;
use super::state::PromptPanelStatus;
use super::state::set_line;
use crate::TypedOrder;
use crate::PromptLineEditor;
use crate::OrderCatalog;
use bevy::input::keyboard::KeyCode;
use bevy::window::PrimaryWindow;

/// Completion candidates for the current line and insert_column.
pub(super) fn offered(state: &PromptPanelStatus, book: &OrderCatalog) -> Vec<String> {
    let PromptLineEditor { line, insert_column, .. } = &state.editor;
    book.hint_list(line, *insert_column)
}

/// Prompt actions requested by this frame's key presses.
#[derive(Debug, Default, PartialEq, Eq)]
struct Wants {
    close: bool,
    clear: bool,
    home: bool,
    end: bool,
    erase_previous_word: bool,
    tab: bool,
    submit: bool,
    up: bool,
    down: bool,
}

impl Wants {
    /// Interprets pressed keys. Under Control, W/U/A/E are line-edit key_combos
    /// and consume the key; everything else goes by its logical meaning.
    fn from_keys<'a>(control: bool, pressed: impl IntoIterator<Item = (KeyCode, &'a Key)>) -> Self {
        let mut out = Self::default();
        for (code, logical) in pressed {
            let key_combo = if control {
                match code {
                    KeyCode::KeyW => Some(&mut out.erase_previous_word),
                    KeyCode::KeyU => Some(&mut out.clear),
                    KeyCode::KeyA => Some(&mut out.home),
                    KeyCode::KeyE => Some(&mut out.end),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(flag) = key_combo {
                *flag = true;
                continue;
            }
            let flag = match logical {
                Key::Escape => &mut out.close,
                Key::ArrowUp => &mut out.up,
                Key::ArrowDown => &mut out.down,
                Key::Tab => &mut out.tab,
                Key::Enter => &mut out.submit,
                _ignored => continue,
            };
            *flag = true;
        }
        out
    }
}

/// Edits the console handles itself instead of letting the text box act.
fn reserved_for_console(edit: &TextEdit) -> bool {
    match edit {
        // Up and down walk the history and the completions.
        TextEdit::Up(_) | TextEdit::Down(_) => true,
        // Ctrl+A clears the line here; selections live in the log.
        TextEdit::SelectAll | TextEdit::CollapseSelection => true,
        _ => false,
    }
}

/// Steps the completion highlight, wrapping at both ends.
fn cycle(selected: usize, ring: usize, forward: bool) -> usize {
    match (ring, forward) {
        (0, _) => selected,
        (_, true) => (selected + 1) % ring,
        (_, false) => (selected + ring - 1) % ring,
    }
}

/// Handles an arrow key: walks the history if that applies, else moves
/// through the completions. Returns true when the history changed the line.
fn step_arrow(state: &mut PromptPanelStatus, book: &OrderCatalog, up: bool) -> bool {
    let in_history = state.recall_position.is_some();
    let recalled = match (up, in_history) {
        (true, _) => {
            let may_recall = in_history || state.editor.line.is_empty();
            may_recall && state.recall_earlier_entry(true)
        }
        (false, true) => {
            state.recall_earlier_entry(false);
            true
        }
        (false, false) => false,
    };
    if !recalled {
        let count = offered(state, book).len();
        state.editor.highlight = cycle(state.editor.highlight, count, up);
    }
    recalled
}

/// Consumes the typed line and echoes it. `help` and `clear` are built in,
/// unknown names are reported, everything else is queued.
fn submit(
    state: &mut PromptPanelStatus,
    book: &OrderCatalog,
    settings: &PromptPanelOptions,
    queue: &mut TypedOrderBacklog,
) {
    if state.recall_position.is_none() {
        let candidates = offered(state, book);
        state.editor.take_hint(&candidates);
    }
    let typed = state.editor.take_line();
    state.quit_recall_mode();
    if typed.trim().is_empty() {
        return;
    }
    let cap = settings.backlog_limit;
    state.post_line(format!("{INPUT_LEAD_GLYPH}{typed}"), cap);
    state.remember(&typed);
    for mut cmd in TypedOrder::parse_script(&typed) {
        let verb = cmd.name.as_str();
        match verb {
            "help" => {
                for row in book.usage_listing() {
                    state.post_line(row, cap);
                }
            }
            "clear" => state.lines.clear(),
            name if book.look_up(name).is_none() => {
                let msg = format!("unknown command `{name}`");
                state.post_line(msg, cap);
            }
            _ => {
                cmd.typed_by_hand = true;
                queue.0.push_back(cmd);
            }
        }
    }
}

/// The prompt's text widget plus the contexts needed to run edits on it.
#[derive(SystemParam)]
pub(super) struct Prompt<'w, 's> {
    entity: Option<Res<'w, PromptEntity>>,
    focus: ResMut<'w, InputFocus>,
    clipboard: ResMut<'w, Clipboard>,
    font_cx: ResMut<'w, FontCx>,
    layout_cx: ResMut<'w, LayoutCx>,
    texts: Query<'w, 's, &'static mut EditableText, With<PromptText>>,
}

impl<'w> Prompt<'w, '_> {
    fn entity(&self) -> Option<Entity> {
        self.entity.as_deref().map(|found| found.0)
    }

    fn widget(&mut self) -> Option<Widget<'_, 'w>> {
        let id = self.entity()?;
        let text = self.texts.get_mut(id).ok()?.into_inner();
        Some(Widget {
            text,
            font_cx: &mut self.font_cx,
            layout_cx: &mut self.layout_cx,
            clipboard: &mut self.clipboard,
        })
    }

    fn drop_pending_edits(&mut self) {
        let Some(widget) = self.widget() else {
            return;
        };
        widget.text.pending_edits.clear();
    }

    /// Hands keyboard focus to the widget when the console opens and releases
    /// it on close, copying the line in the matching direction.
    fn toggle_focus(&mut self, open: bool, editor: &mut PromptLineEditor) {
        let Some(id) = self.entity() else {
            return;
        };
        if !open {
            if self.focus.get() == Some(id) {
                self.focus.clear();
            }
            if let Some(widget) = self.widget() {
                widget.read(editor);
            }
            return;
        }
        let why = FocusCause::Navigated;
        self.focus.set(id, why);
        if let Some(mut widget) = self.widget() {
            widget.paint_line(editor);
        }
    }
}

struct Widget<'a, 'w> {
    text: &'a mut EditableText,
    font_cx: &'a mut ResMut<'w, FontCx>,
    layout_cx: &'a mut ResMut<'w, LayoutCx>,
    clipboard: &'a mut ResMut<'w, Clipboard>,
}

impl Widget<'_, '_> {
    /// Executes any edits waiting in the widget.
    fn apply(&mut self) {
        let idle = self.text.pending_edits.is_empty() && self.text.pending_paste.is_none();
        if idle {
            return;
        }
        let layout = &mut self.layout_cx.0;
        self.text
            .apply_pending_edits(self.font_cx, layout, self.clipboard, |_| true);
    }

    /// Keeps the content on one line by turning line breaks into spaces.
    fn strip_newlines(&mut self) {
        let current = self.text.value().to_string();
        if !current.contains(['\n', '\r']) {
            return;
        }
        let flat = crate::key_input::tidy_pasted_orders(&current);
        self.text.clear();
        self.text.editor.set_text(&flat);
        self.text.queue_edit(TextEdit::TextEnd(false));
    }

    /// Copies the widget's content and insert_column position into the editor.
    fn read(&self, editor: &mut PromptLineEditor) {
        let content = self.text.value().to_string();
        let caret_byte = self.text.editor().raw_selection().focus().index();
        // The column is the number of chars that start before the caret.
        let starts = content.char_indices().map(|(start, _)| start);
        editor.insert_column = starts.filter(|&start| start < caret_byte).count();
        editor.line = content;
    }

    /// Loads the editor's line and insert_column into the widget as queued edits.
    fn paint_line(&mut self, editor: &PromptLineEditor) {
        self.text.clear();
        self.text.editor.set_text(&editor.line);
        self.text.queue_edit(TextEdit::TextStart(false));
        (0..editor.insert_column).for_each(|_| self.text.queue_edit(TextEdit::Right(false)));
    }

    /// Applies typed or pasted input, normalises it, and syncs the editor.
    fn settle(&mut self, editor: &mut PromptLineEditor) {
        self.apply();
        self.strip_newlines();
        self.apply();
        self.read(editor);
    }

    /// Pushes the editor's line into the widget, then reads the insert_column back.
    fn replace(&mut self, editor: &mut PromptLineEditor) {
        self.paint_line(editor);
        self.apply();
        self.read(editor);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_console_input(
    key_state: Res<ButtonInput<KeyCode>>,
    layout_keys: Res<ButtonInput<Key>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut state: ResMut<PromptPanelStatus>,
    book: Res<OrderCatalog>,
    settings: Res<PromptPanelOptions>,
    mut queue: ResMut<TypedOrderBacklog>,
    mut prompt: Prompt<'_, '_>,
) {
    let toggled = key_state.just_pressed(KeyCode::Backquote);
    if toggled {
        state.shown = !state.shown;
        prompt.drop_pending_edits();
        prompt.toggle_focus(state.shown, &mut state.editor);
    }
    if !state.shown {
        state.clear_scroll_selection();
    }
    if toggled || !state.shown {
        return;
    }
    let Some(prompt_id) = prompt.entity() else {
        return;
    };
    if prompt.focus.get() != Some(prompt_id) {
        let why = FocusCause::Navigated;
        prompt.focus.set(prompt_id, why);
    }

    let ctrl_down = key_state.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
        || layout_keys.pressed(Key::Control);
    let pressed_now = key_events
        .read()
        .filter(|ev| ev.state.is_pressed())
        .map(|ev| (ev.key_code, &ev.logical_key));
    let wants = Wants::from_keys(ctrl_down, pressed_now);
    if wants.close {
        prompt.drop_pending_edits();
        prompt.focus.clear();
        state.shown = false;
        return;
    }

    let Some(mut widget) = prompt.widget() else {
        return;
    };
    let queued = &mut widget.text.pending_edits;
    queued.retain(|edit| !reserved_for_console(edit));

    if wants.clear {
        widget.text.clear();
        set_line(&mut state.editor, "");
        state.quit_recall_mode();
        return;
    }
    if wants.home {
        widget.text.queue_edit(TextEdit::LineStart(false));
    }
    if wants.end {
        widget.text.queue_edit(TextEdit::LineEnd(false));
    }
    widget.settle(&mut state.editor);

    if wants.erase_previous_word {
        state.editor.erase_previous_word();
        state.quit_recall_mode();
        widget.replace(&mut state.editor);
    }
    if (wants.up || wants.down) && step_arrow(&mut state, &book, wants.up) {
        widget.replace(&mut state.editor);
    }
    if wants.tab {
        let candidates = offered(&state, &book);
        if state.editor.take_hint(&candidates) {
            state.quit_recall_mode();
            widget.replace(&mut state.editor);
        }
    }
    if wants.submit {
        submit(&mut state, &book, &settings, &mut queue);
        widget.text.clear();
    }
}

/// The press/release pair of key events a keyboard would send for `ch`.
fn typed_character(ch: char, window: Entity) -> [KeyboardInput; 2] {
    let as_text = ch.to_string();
    let unknown = KeyCode::Unidentified(NativeKeyCode::Unidentified);
    let repeat = false;
    let event = |state, text| KeyboardInput {
        key_code: unknown,
        logical_key: Key::Character(ch.to_string().into()),
        state,
        text,
        repeat,
        window,
    };
    [
        event(ButtonState::Pressed, Some(as_text.into())),
        event(ButtonState::Released, None),
    ]
}

/// `con_feed`: replays the stored text as keyboard events so it takes the
/// same route as real typing.
pub(super) fn feed_console_keyboard(
    mut state: ResMut<PromptPanelStatus>,
    mut keyboard_out: MessageWriter<KeyboardInput>,
    primary: Query<Entity, With<PrimaryWindow>>,
) {
    // The text is used up even when the console is closed.
    let Some(queued) = state.pending_feed.take() else {
        return;
    };
    let Ok(window) = primary.single() else {
        return;
    };
    if state.shown {
        keyboard_out.write_batch(queued.chars().flat_map(|ch| typed_character(ch, window)));
    }
}

/// `con_paste`: drops the system clipboard into the prompt.
pub(super) fn apply_console_os_paste(
    mut state: ResMut<PromptPanelStatus>,
    settings: Res<PromptPanelOptions>,
    mut prompt: Prompt<'_, '_>,
) {
    if !std::mem::take(&mut state.pending_os_paste) {
        return;
    }
    let cap = settings.backlog_limit;
    if !state.shown {
        let closed = "con_paste: console closed";
        state.post_line(closed, cap);
        return;
    }
    let Some(mut widget) = prompt.widget() else {
        return;
    };
    let mut fetch = widget.clipboard.fetch_text();
    match fetch.poll_result() {
        None => state.post_line("clipboard read pending", cap),
        Some(Err(err)) => state.post_line(format!("clipboard read failed: {err}"), cap),
        Some(Ok(text)) if text.is_empty() => state.post_line("clipboard empty", cap),
        Some(Ok(pasted)) => {
            let char_count = pasted.chars().count();
            widget.text.queue_edit(TextEdit::Insert(pasted.into()));
            widget.settle(&mut state.editor);
            state.post_line(format!("pasted {char_count} chars"), cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::prompt::*;
    use crate::OrderBlueprint;

    fn pressed(control: bool, keys: &[(KeyCode, Key)]) -> Wants {
        Wants::from_keys(control, keys.iter().map(|(code, key)| (*code, key)))
    }

    fn book() -> OrderCatalog {
        let mut book = OrderCatalog::default();
        book.enlist(OrderBlueprint::new("map").help_line("map <name>"));
        book.enlist(OrderBlueprint::new("quit").help_line("quit"));
        book
    }

    fn typed(line: &str) -> PromptPanelStatus {
        let mut state = PromptPanelStatus::default();
        set_line(&mut state.editor, line);
        state
    }

    fn run(state: &mut PromptPanelStatus) -> Vec<TypedOrder> {
        let mut queue = TypedOrderBacklog::default();
        submit(state, &book(), &PromptPanelOptions::default(), &mut queue);
        queue.0.into_iter().collect()
    }

    #[test]
    fn plain_keys_ask_for_what_they_say() {
        let wants = pressed(
            false,
            &[
                (KeyCode::Escape, Key::Escape),
                (KeyCode::ArrowUp, Key::ArrowUp),
                (KeyCode::Tab, Key::Tab),
                (KeyCode::Enter, Key::Enter),
            ],
        );
        let expected = Wants {
            close: true,
            up: true,
            tab: true,
            submit: true,
            ..Wants::default()
        };
        assert_eq!(wants, expected);
        assert!(pressed(false, &[(KeyCode::ArrowDown, Key::ArrowDown)]).down);
    }

    #[test]
    fn control_letters_edit_the_line_and_stop_there() {
        let letter = |code, ch: &str| (code, Key::Character(ch.into()));
        let wants = pressed(
            true,
            &[
                letter(KeyCode::KeyW, "w"),
                letter(KeyCode::KeyU, "u"),
                letter(KeyCode::KeyA, "a"),
                letter(KeyCode::KeyE, "e"),
            ],
        );
        let expected = Wants {
            erase_previous_word: true,
            clear: true,
            home: true,
            end: true,
            ..Wants::default()
        };
        assert_eq!(wants, expected);
        let expected = Wants::default();
        assert_eq!(pressed(false, &[letter(KeyCode::KeyW, "w")]), expected, "without Control a w is just a w");
    }

    #[test]
    fn control_does_not_swallow_the_other_keys() {
        let wants = pressed(true, &[(KeyCode::Enter, Key::Enter)]);
        assert!(wants.submit);
    }

    #[test]
    fn the_text_boxs_vertical_keys_belong_to_the_console() {
        assert!(reserved_for_console(&TextEdit::SelectAll));
        assert!(reserved_for_console(&TextEdit::CollapseSelection));
        assert!(!reserved_for_console(&TextEdit::TextEnd(false)));
    }

    #[test]
    fn cycling_wraps_both_ways_and_ignores_an_empty_list() {
        assert_eq!(cycle(0, 3, true), 1);
        assert_eq!(cycle(2, 3, true), 0);
        assert_eq!(cycle(0, 3, false), 2);
        assert_eq!(cycle(1, 3, false), 0);
        assert_eq!(cycle(5, 0, true), 5);
        assert_eq!(cycle(0, 1, true), 0);
    }

    #[test]
    fn arrows_walk_the_history_from_an_empty_prompt() {
        let mut state = PromptPanelStatus::default();
        state.remember("map a");
        state.remember("quit");
        assert!(step_arrow(&mut state, &book(), true));
        assert_eq!(state.editor.line, "quit");
        assert!(step_arrow(&mut state, &book(), true));
        assert_eq!(state.editor.line, "map a");
        assert!(step_arrow(&mut state, &book(), false));
        assert_eq!(state.editor.line, "quit");
    }

    #[test]
    fn arrows_move_through_completions_once_something_is_typed() {
        let mut state = typed("m");
        state.remember("quit");
        let before = offered(&state, &book()).len();
        assert!(before > 0);
        assert!(!step_arrow(&mut state, &book(), true));
        assert_eq!(state.editor.highlight, cycle(0, before, true));
        assert_eq!(state.editor.line, "m", "the typed line is left alone");
    }

    #[test]
    fn a_submitted_command_is_echoed_remembered_and_queued_for_the_user() {
        let mut state = typed("map mp_nuked");
        let queued = run(&mut state);
        assert_eq!(state.lines, ["> map mp_nuked"]);
        assert_eq!(state.recalled, ["map mp_nuked"]);
        assert_eq!(state.editor.line, "");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].name, "map");
        assert_eq!(queued[0].words, ["mp_nuked"]);
        assert!(queued[0].typed_by_hand);
    }

    #[test]
    fn a_script_queues_each_command_in_order() {
        let mut state = typed("map mp_nuked; quit");
        let names: Vec<_> = run(&mut state).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["map", "quit"]);
    }

    #[test]
    fn an_unknown_command_is_reported_and_not_queued() {
        let mut state = typed("frobnicate now");
        let queued = run(&mut state);
        assert!(queued.is_empty());
        let expected = Some("unknown command `frobnicate`");
        assert_eq!(state.lines.last().map(String::as_str), expected);
    }

    #[test]
    fn help_lists_every_command_and_clear_empties_the_scrollback() {
        let mut state = typed("help");
        assert!(run(&mut state).is_empty());
        assert!(state.lines.len() > 1);
        set_line(&mut state.editor, "clear");
        assert!(run(&mut state).is_empty());
        assert!(state.lines.is_empty());
    }

    #[test]
    fn a_blank_line_does_nothing() {
        let mut state = typed("   ");
        assert!(run(&mut state).is_empty());
        assert!(state.lines.is_empty() && state.recalled.is_empty());
    }

    #[test]
    fn enter_takes_the_highlighted_completion_first() {
        let mut state = typed("ma");
        let queued = run(&mut state);
        assert_eq!(queued.first().map(|c| c.name.as_str()), Some("map"));
    }

    #[test]
    fn enter_on_a_recalled_line_runs_it_as_it_is() {
        let mut state = PromptPanelStatus::default();
        state.remember("ma");
        assert!(state.recall_earlier_entry(true));
        let queued = run(&mut state);
        assert!(
            queued.is_empty(),
            "`ma` is not a command, so it is reported"
        );
        assert_eq!(state.lines, ["> ma", "unknown command `ma`"]);
    }

    #[test]
    fn a_fed_character_is_a_key_press_with_text_then_a_release() {
        let window = Entity::PLACEHOLDER;
        let [down, up] = typed_character('é', window);
        assert_eq!(down.state, ButtonState::Pressed);
        assert_eq!(down.text.as_deref(), Some("é"));
        assert_eq!(down.logical_key, Key::Character("é".into()));
        assert_eq!(up.state, ButtonState::Released);
        assert_eq!(up.text, None);
        assert_eq!(down.window, window);
    }
}
