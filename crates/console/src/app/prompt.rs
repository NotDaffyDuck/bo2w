//! The prompt line: the keys that edit it, the history and completions behind
//! the arrows and Tab, and what happens when a line is submitted.

use bevy::{
    ecs::system::SystemParam,
    input::{
        ButtonState,
        keyboard::{Key, KeyCode, KeyboardInput, NativeKeyCode},
    },
    input_focus::{FocusCause, InputFocus},
    prelude::*,
    text::{EditableText, FontCx, LayoutCx, TextEdit},
    window::PrimaryWindow,
};

use super::{
    panel::{PROMPT, PromptEntity, PromptText},
    queue::ConsoleCommandQueue,
    state::{ConsoleSettings, ConsoleState, set_line},
};
use crate::{ConsoleCommand, ConsoleEditor, ConsoleRegistry};

/// The completions on offer for the line as it stands.
pub(super) fn offered(state: &ConsoleState, registry: &ConsoleRegistry) -> Vec<String> {
    registry.suggestions(&state.editor.line, state.editor.caret)
}

/// What the keys pressed this frame ask the prompt to do.
#[derive(Debug, Default, PartialEq, Eq)]
struct Wants {
    close: bool,
    clear: bool,
    home: bool,
    end: bool,
    kill_word: bool,
    tab: bool,
    submit: bool,
    up: bool,
    down: bool,
}

impl Wants {
    /// Reads the pressed keys. With Control held, W, U, A and E are line
    /// editing and stop there; every other key is read by what it types.
    fn from_keys<'a>(control: bool, pressed: impl IntoIterator<Item = (KeyCode, &'a Key)>) -> Self {
        let mut wants = Self::default();
        for (code, logical) in pressed {
            if control {
                match code {
                    KeyCode::KeyW => {
                        wants.kill_word = true;
                        continue;
                    }
                    KeyCode::KeyU => {
                        wants.clear = true;
                        continue;
                    }
                    KeyCode::KeyA => {
                        wants.home = true;
                        continue;
                    }
                    KeyCode::KeyE => {
                        wants.end = true;
                        continue;
                    }
                    _ => {}
                }
            }
            match logical {
                Key::Escape => wants.close = true,
                Key::ArrowUp => wants.up = true,
                Key::ArrowDown => wants.down = true,
                Key::Tab => wants.tab = true,
                Key::Enter => wants.submit = true,
                _ => {}
            }
        }
        wants
    }
}

/// The text box's own handling of these keys is replaced by the console's.
fn reserved_for_console(edit: &TextEdit) -> bool {
    matches!(
        edit,
        TextEdit::Up(_) | TextEdit::Down(_) | TextEdit::SelectAll | TextEdit::CollapseSelection
    )
}

/// Moves the highlighted completion on by one, wrapping round. `forward`
/// goes to the next entry.
fn cycle(selected: usize, len: usize, forward: bool) -> usize {
    match (len, forward) {
        (0, _) => selected,
        (_, true) => (selected + 1) % len,
        (_, false) => (selected + len - 1) % len,
    }
}

/// An arrow key: browses the history when that applies, otherwise moves
/// through the completions. True when the history replaced the line.
fn step_arrow(state: &mut ConsoleState, registry: &ConsoleRegistry, up: bool) -> bool {
    let browsing = state.history_cursor.is_some();
    if up {
        if (browsing || state.editor.line.is_empty()) && state.history_recall(true) {
            return true;
        }
    } else if browsing {
        state.history_recall(false);
        return true;
    }
    let len = offered(state, registry).len();
    state.editor.selected = cycle(state.editor.selected, len, up);
    false
}

/// Takes the typed line, prints it, and acts on it: `help` and `clear` are
/// the console's own, an unknown command is reported, the rest are queued.
fn submit(
    state: &mut ConsoleState,
    registry: &ConsoleRegistry,
    settings: &ConsoleSettings,
    queue: &mut ConsoleCommandQueue,
) {
    if state.history_cursor.is_none() {
        let completions = offered(state, registry);
        state.editor.accept(&completions);
    }
    let line = state.editor.take();
    state.leave_history_browse();
    if line.trim().is_empty() {
        return;
    }
    let capacity = settings.log_capacity;
    state.echo(format!("{PROMPT}{line}"), capacity);
    state.remember(&line);
    for mut command in ConsoleCommand::parse_script(&line) {
        let known = registry.resolve(&command.name).is_some();
        match command.name.as_str() {
            "help" => {
                for text in registry.help_lines() {
                    state.echo(text, capacity);
                }
            }
            "clear" => state.log.clear(),
            _ if !known => {
                state.echo(format!("unknown command `{}`", command.name), capacity);
            }
            _ => {
                command.interactive = true;
                queue.0.push_back(command);
            }
        }
    }
}

/// The prompt's text box and everything needed to run edits on it.
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
        self.entity.as_deref().map(|prompt| prompt.0)
    }

    fn widget(&mut self) -> Option<Widget<'_, 'w>> {
        let entity = self.entity.as_deref()?.0;
        let text = self.texts.get_mut(entity).ok()?.into_inner();
        Some(Widget {
            text,
            font_cx: &mut self.font_cx,
            layout_cx: &mut self.layout_cx,
            clipboard: &mut self.clipboard,
        })
    }

    fn drop_pending_edits(&mut self) {
        if let Some(widget) = self.widget() {
            widget.text.pending_edits.clear();
        }
    }

    /// Gives the box the keyboard while the console is open and takes it
    /// back when it closes, carrying the line across either way.
    fn set_focus(&mut self, open: bool, editor: &mut ConsoleEditor) {
        let Some(entity) = self.entity() else {
            return;
        };
        if open {
            self.focus.set(entity, FocusCause::Navigated);
            if let Some(mut widget) = self.widget() {
                widget.write(editor);
            }
        } else {
            if self.focus.get() == Some(entity) {
                self.focus.clear();
            }
            if let Some(widget) = self.widget() {
                widget.read(editor);
            }
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
    /// Runs the edits the box has queued.
    fn apply(&mut self) {
        if self.text.pending_edits.is_empty() && self.text.pending_paste.is_none() {
            return;
        }
        self.text
            .apply_pending_edits(self.font_cx, &mut self.layout_cx.0, self.clipboard, |_| {
                true
            });
    }

    /// A line is one line: pasted line breaks become spaces.
    fn strip_newlines(&mut self) {
        let value = self.text.value().to_string();
        if !value.bytes().any(|byte| byte == b'\n' || byte == b'\r') {
            return;
        }
        let cleaned = crate::input::normalize_command_paste(&value);
        self.text.clear();
        self.text.editor.set_text(&cleaned);
        self.text.queue_edit(TextEdit::TextEnd(false));
    }

    /// Copies the box's line and caret into the editor.
    fn read(&self, editor: &mut ConsoleEditor) {
        let line = self.text.value().to_string();
        let byte = self.text.editor().raw_selection().focus().index();
        editor.caret = line
            .char_indices()
            .take_while(|(index, _)| *index < byte)
            .count();
        editor.line = line;
    }

    /// Queues the editor's line and caret into the box.
    fn write(&mut self, editor: &ConsoleEditor) {
        self.text.clear();
        self.text.editor.set_text(&editor.line);
        self.text.queue_edit(TextEdit::TextStart(false));
        for _ in 0..editor.caret {
            self.text.queue_edit(TextEdit::Right(false));
        }
    }

    /// Runs what was typed or pasted and reads the result back.
    fn settle(&mut self, editor: &mut ConsoleEditor) {
        self.apply();
        self.strip_newlines();
        self.apply();
        self.read(editor);
    }

    /// Puts the editor's line in the box and reads the caret back.
    fn replace(&mut self, editor: &mut ConsoleEditor) {
        self.write(editor);
        self.apply();
        self.read(editor);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_console_input(
    keys: Res<ButtonInput<KeyCode>>,
    logical_keys: Res<ButtonInput<Key>>,
    mut events: MessageReader<KeyboardInput>,
    mut state: ResMut<ConsoleState>,
    registry: Res<ConsoleRegistry>,
    settings: Res<ConsoleSettings>,
    mut queue: ResMut<ConsoleCommandQueue>,
    mut prompt: Prompt<'_, '_>,
) {
    if keys.just_pressed(KeyCode::Backquote) {
        state.open = !state.open;
        prompt.drop_pending_edits();
        prompt.set_focus(state.open, &mut state.editor);
        if !state.open {
            state.clear_scroll_selection();
        }
        return;
    }
    if !state.open {
        state.clear_scroll_selection();
        return;
    }
    let Some(entity) = prompt.entity() else {
        return;
    };
    if prompt.focus.get() != Some(entity) {
        prompt.focus.set(entity, FocusCause::Navigated);
    }

    let control = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || logical_keys.pressed(Key::Control);
    let wants = Wants::from_keys(
        control,
        events
            .read()
            .filter(|event| event.state.is_pressed())
            .map(|event| (event.key_code, &event.logical_key)),
    );
    if wants.close {
        state.open = false;
        prompt.drop_pending_edits();
        prompt.focus.clear();
        return;
    }

    let Some(mut widget) = prompt.widget() else {
        return;
    };
    widget
        .text
        .pending_edits
        .retain(|edit| !reserved_for_console(edit));

    if wants.clear {
        widget.text.clear();
        set_line(&mut state.editor, "");
        state.leave_history_browse();
        return;
    }
    if wants.home {
        widget.text.queue_edit(TextEdit::LineStart(false));
    }
    if wants.end {
        widget.text.queue_edit(TextEdit::LineEnd(false));
    }
    widget.settle(&mut state.editor);

    if wants.kill_word {
        state.editor.kill_word();
        state.leave_history_browse();
        widget.replace(&mut state.editor);
    }
    if (wants.up || wants.down) && step_arrow(&mut state, &registry, wants.up) {
        widget.replace(&mut state.editor);
    }
    if wants.tab {
        let completions = offered(&state, &registry);
        if state.editor.accept(&completions) {
            state.leave_history_browse();
            widget.replace(&mut state.editor);
        }
    }
    if wants.submit {
        submit(&mut state, &registry, &settings, &mut queue);
        widget.text.clear();
    }
}

/// One typed character as the pair of key events a keyboard would send.
fn typed_character(ch: char, window: Entity) -> [KeyboardInput; 2] {
    let text = ch.to_string();
    let key = |state, text| KeyboardInput {
        key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
        logical_key: Key::Character(ch.to_string().into()),
        state,
        text,
        repeat: false,
        window,
    };
    [
        key(ButtonState::Pressed, Some(text.into())),
        key(ButtonState::Released, None),
    ]
}

/// `con_feed`: types the text the command left, as keyboard input, so it
/// goes through the same path as real typing.
pub(super) fn feed_console_keyboard(
    mut state: ResMut<ConsoleState>,
    mut writer: MessageWriter<KeyboardInput>,
    window: Query<Entity, With<PrimaryWindow>>,
) {
    let Some(text) = state.pending_feed.take() else {
        return;
    };
    if !state.open {
        return;
    }
    let Ok(window) = window.single() else {
        return;
    };
    for ch in text.chars() {
        for event in typed_character(ch, window) {
            writer.write(event);
        }
    }
}

/// `con_paste`: inserts the OS clipboard into the prompt.
pub(super) fn apply_console_os_paste(
    mut state: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
    mut prompt: Prompt<'_, '_>,
) {
    if !std::mem::take(&mut state.pending_os_paste) {
        return;
    }
    let capacity = settings.log_capacity;
    if !state.open {
        state.echo("con_paste: console closed", capacity);
        return;
    }
    let Some(mut widget) = prompt.widget() else {
        return;
    };
    let mut read = widget.clipboard.fetch_text();
    match read.poll_result() {
        Some(Ok(text)) if text.is_empty() => state.echo("clipboard empty", capacity),
        Some(Ok(text)) => {
            let count = text.chars().count();
            widget.text.queue_edit(TextEdit::Insert(text.into()));
            widget.settle(&mut state.editor);
            state.echo(format!("pasted {count} chars"), capacity);
        }
        Some(Err(error)) => state.echo(format!("clipboard read failed: {error}"), capacity),
        None => state.echo("clipboard read pending", capacity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandSpec;

    fn pressed(control: bool, keys: &[(KeyCode, Key)]) -> Wants {
        Wants::from_keys(control, keys.iter().map(|(code, key)| (*code, key)))
    }

    fn registry() -> ConsoleRegistry {
        let mut registry = ConsoleRegistry::default();
        registry.register(CommandSpec::new("map").usage("map <name>"));
        registry.register(CommandSpec::new("quit").usage("quit"));
        registry
    }

    fn typed(line: &str) -> ConsoleState {
        let mut state = ConsoleState::default();
        set_line(&mut state.editor, line);
        state
    }

    fn run(state: &mut ConsoleState) -> Vec<ConsoleCommand> {
        let mut queue = ConsoleCommandQueue::default();
        submit(state, &registry(), &ConsoleSettings::default(), &mut queue);
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
        assert_eq!(
            wants,
            Wants {
                close: true,
                up: true,
                tab: true,
                submit: true,
                ..Wants::default()
            }
        );
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
        assert_eq!(
            wants,
            Wants {
                kill_word: true,
                clear: true,
                home: true,
                end: true,
                ..Wants::default()
            }
        );
        assert_eq!(
            pressed(false, &[letter(KeyCode::KeyW, "w")]),
            Wants::default(),
            "without Control a w is just a w"
        );
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
        let mut state = ConsoleState::default();
        state.remember("map a");
        state.remember("quit");
        assert!(step_arrow(&mut state, &registry(), true));
        assert_eq!(state.editor.line, "quit");
        assert!(step_arrow(&mut state, &registry(), true));
        assert_eq!(state.editor.line, "map a");
        assert!(step_arrow(&mut state, &registry(), false));
        assert_eq!(state.editor.line, "quit");
    }

    #[test]
    fn arrows_move_through_completions_once_something_is_typed() {
        let mut state = typed("m");
        state.remember("quit");
        let before = offered(&state, &registry()).len();
        assert!(before > 0);
        assert!(!step_arrow(&mut state, &registry(), true));
        assert_eq!(state.editor.selected, cycle(0, before, true));
        assert_eq!(state.editor.line, "m", "the typed line is left alone");
    }

    #[test]
    fn a_submitted_command_is_echoed_remembered_and_queued_for_the_user() {
        let mut state = typed("map mp_nuked");
        let queued = run(&mut state);
        assert_eq!(state.log, ["> map mp_nuked"]);
        assert_eq!(state.history, ["map mp_nuked"]);
        assert_eq!(state.editor.line, "");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].name, "map");
        assert_eq!(queued[0].args, ["mp_nuked"]);
        assert!(queued[0].interactive);
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
        assert_eq!(
            state.log.last().map(String::as_str),
            Some("unknown command `frobnicate`")
        );
    }

    #[test]
    fn help_lists_every_command_and_clear_empties_the_scrollback() {
        let mut state = typed("help");
        assert!(run(&mut state).is_empty());
        assert!(state.log.len() > 1);
        set_line(&mut state.editor, "clear");
        assert!(run(&mut state).is_empty());
        assert!(state.log.is_empty());
    }

    #[test]
    fn a_blank_line_does_nothing() {
        let mut state = typed("   ");
        assert!(run(&mut state).is_empty());
        assert!(state.log.is_empty() && state.history.is_empty());
    }

    #[test]
    fn enter_takes_the_highlighted_completion_first() {
        let mut state = typed("ma");
        let queued = run(&mut state);
        assert_eq!(queued.first().map(|c| c.name.as_str()), Some("map"));
    }

    #[test]
    fn enter_on_a_recalled_line_runs_it_as_it_is() {
        let mut state = ConsoleState::default();
        state.remember("ma");
        assert!(state.history_recall(true));
        let queued = run(&mut state);
        assert!(
            queued.is_empty(),
            "`ma` is not a command, so it is reported"
        );
        assert_eq!(state.log, ["> ma", "unknown command `ma`"]);
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
