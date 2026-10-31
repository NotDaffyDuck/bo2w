//! Binding a command to whatever the player presses next, when the controls
//! menu asks for one.

use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::{Gamepad, GamepadButton};
use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use frame::ActivePad;
use frame::PlayerPrefsBundle;
use frame::UiBindRequest;
use frame::UiBindingCapture;

use crate::ActionBinding;
use crate::BindingTable;
use crate::GamepadKey;
use crate::keymap::wheel_button;

/// The command waiting for its button, by action number. It goes `live` one
/// frame after the menu asked, so the click that opened the prompt is not
/// taken as the answer.
#[derive(Resource, Default)]
pub(crate) struct AwaitedMenuControl {
    action: Option<u32>,
    live: bool,
}

#[derive(SystemParam)]
pub(crate) struct Inputs<'w, 's> {
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    wheel: MessageReader<'w, 's, MouseWheel>,
    requests: MessageReader<'w, 's, UiBindRequest>,
    pads: Query<'w, 's, &'static Gamepad>,
    active: Res<'w, ActivePad>,
}

#[derive(Debug, PartialEq)]
enum Press {
    Pad(GamepadKey),
    Cancel,
    Other(ActionBinding),
}

/// What this frame's presses mean. A pad button wins over everything but
/// Start, which (like Escape) backs out; a key, then a mouse button, then the
/// wheel come last.
fn choose(
    pad_start: bool,
    pad_button: Option<GamepadKey>,
    escape: bool,
    other: Option<ActionBinding>,
) -> Option<Press> {
    match (pad_start, pad_button) {
        (false, Some(button)) => Some(Press::Pad(button)),
        (true, _) => Some(Press::Cancel),
        (false, None) if escape => Some(Press::Cancel),
        (false, None) => other.map(Press::Other),
    }
}

/// The menu's half of the capture: what it shows as listening, and the
/// request it is waiting on.
struct MenuSide<'a> {
    capture: &'a mut UiBindingCapture,
    view: &'a mut ui::KeymapSnapshot,
}

impl MenuSide<'_> {
    /// A new revision makes the menu draw the row again.
    fn redraw(&mut self) {
        self.view.revision = self.view.revision.wrapping_add(1);
    }

    /// Waits for a button for `verb` (action number `action`); the wait
    /// goes live next frame.
    fn listen(&mut self, pending: &mut AwaitedMenuControl, verb: &str, action: u32) {
        *pending = AwaitedMenuControl { action: Some(action), live: false };
        self.capture.command = Some(verb.to_owned());
        self.view.listening = Some(action);
        self.redraw();
    }

    /// Stops waiting without drawing anything new: the menu asked for
    /// something that is not a command, or stopped asking.
    fn forget(&mut self, pending: &mut AwaitedMenuControl, clear_command: bool) {
        pending.action.take();
        if clear_command {
            self.capture.command = None;
        }
        self.view.listening = None;
    }

    /// The wait is over, answered or backed out of; the menu has used the
    /// press, so play must not see it.
    fn finish(&mut self, pending: &mut AwaitedMenuControl) {
        *pending = AwaitedMenuControl::default();
        self.capture.command = None;
        self.capture.consumed_input = true;
        self.view.listening = None;
        self.redraw();
    }
}

/// The active pad's presses this frame: Start, and the first other button.
fn pad_presses(input: &Inputs) -> (bool, Option<GamepadKey>) {
    let Some(pad) = input.active.0.and_then(|entity| input.pads.get(entity).ok()) else {
        return (false, None);
    };
    let start = pad.just_pressed(GamepadButton::Start);
    let mut fresh = pad.get_just_pressed().filter_map(|button| GamepadKey::from_gamepad_button(*button));
    (start, fresh.find(|button| *button != GamepadKey::Start))
}

/// The first key, else the first mouse button, pressed this frame.
fn key_or_mouse(input: &Inputs) -> Option<ActionBinding> {
    if let Some(key) = input.keys.get_just_pressed().next() {
        return Some(ActionBinding::Board(*key));
    }
    input.mouse.get_just_pressed().next().map(|button| ActionBinding::Pointer(*button))
}

pub(crate) fn take_menu_control(
    mut input: Inputs,
    mut prefs: ResMut<PlayerPrefsBundle>,
    mut pending: ResMut<AwaitedMenuControl>,
    mut capture: ResMut<UiBindingCapture>,
    mut binds: ResMut<BindingTable>,
    mut view: ResMut<ui::KeymapSnapshot>,
) {
    let mut menu = MenuSide { capture: &mut capture, view: &mut view };
    menu.capture.consumed_input = false;
    if menu.capture.command.is_none() {
        menu.forget(&mut pending, false);
    }
    // Every wheel event is drained so none spills into the next frame; the
    // first one that makes a button is the one that counts.
    let mut wheel = None;
    for event in input.wheel.read() {
        wheel = wheel.or_else(|| wheel_button(event.y));
    }
    let mut asked_now = false;
    for request in input.requests.read() {
        match input_t6::action_number_for(&request.command) {
            Some(action) => {
                menu.listen(&mut pending, &request.command, action);
                asked_now = true;
            }
            None => menu.forget(&mut pending, true),
        }
    }
    let Some(action) = pending.action else {
        return;
    };
    if asked_now || !pending.live {
        pending.live = true;
        return;
    }
    let (pad_start, pad_button) = pad_presses(&input);
    let escape = input.keys.just_pressed(KeyCode::Escape);
    let fallback = key_or_mouse(&input).or(wheel);
    let Some(press) = choose(pad_start, pad_button, escape, fallback) else {
        return;
    };
    match press {
        Press::Cancel => {}
        Press::Other(button) => {
            binds.clear_command_on(action, false);
            binds.assign(button, action);
        }
        Press::Pad(button) => {
            binds.clear_command_on(action, true);
            binds.assign(ActionBinding::Pad(button), action);
            // Picking a pad button by hand leaves the preset layouts behind.
            if prefs.pad_layout != PlayerPrefsBundle::PAD_LAYOUT_CUSTOM {
                prefs.pad_layout = PlayerPrefsBundle::PAD_LAYOUT_CUSTOM;
                prefs.touch();
            }
        }
    }
    menu.finish(&mut pending);
}

#[cfg(test)]
mod tests {
    use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};

    use crate::prefs::capture::*;
    use bevy::app::Update;

    const GOSTAND: &str = "+gostand";

    fn gostand() -> u32 {
        input_t6::action_number_for(GOSTAND).expect("a known command")
    }

    fn capture_app() -> App {
        let mut app = App::default();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ActivePad>()
            .init_resource::<PlayerPrefsBundle>()
            .init_resource::<AwaitedMenuControl>()
            .init_resource::<UiBindingCapture>()
            .init_resource::<BindingTable>()
            .init_resource::<ui::KeymapSnapshot>()
            .add_message::<MouseWheel>()
            .add_message::<UiBindRequest>()
            .add_systems(Update, take_menu_control);
        app
    }

    /// One frame, then the "just pressed" marks are cleared as the input
    /// plugin would at the start of the next.
    fn tick(app: &mut App) {
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
    }

    fn ask(app: &mut App, command: &str) {
        app.world_mut().write_message(UiBindRequest {
            command: command.to_owned(),
        });
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn bound(app: &App, button: ActionBinding) -> Option<u32> {
        app.world().resource::<BindingTable>().bound_to(button)
    }

    fn listening(app: &App) -> Option<u32> {
        app.world().resource::<ui::KeymapSnapshot>().listening
    }

    fn asking(app: &App) -> Option<String> {
        app.world().resource::<UiBindingCapture>().command.clone()
    }

    fn consumed(app: &App) -> bool {
        app.world().resource::<UiBindingCapture>().consumed_input
    }

    #[test]
    fn the_press_that_comes_with_the_request_is_not_the_answer() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        press(&mut app, KeyCode::KeyJ);
        tick(&mut app);
        assert_eq!(listening(&app), Some(gostand()));
        assert_eq!(asking(&app).as_deref(), Some(GOSTAND));
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyJ)), None);
    }

    #[test]
    fn the_next_key_is_bound_and_listening_stops() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyK)), Some(gostand()));
        assert_eq!(listening(&app), None);
        assert_eq!(asking(&app), None);
        assert!(consumed(&app), "the menu must not also act on that key");
        tick(&mut app);
        assert!(!consumed(&app), "only the frame of the press is consumed");
    }

    #[test]
    fn nothing_pressed_keeps_listening() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        for _ in 0..4 {
            tick(&mut app);
        }
        assert_eq!(listening(&app), Some(gostand()));
        assert!(!consumed(&app));
    }

    #[test]
    fn escape_backs_out_without_binding() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::Escape);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::Escape)), None);
        assert_eq!(listening(&app), None);
        assert!(consumed(&app));
    }

    #[test]
    fn a_mouse_button_can_be_the_answer() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        tick(&mut app);
        let actual = bound(&app, ActionBinding::Pointer(MouseButton::Right));
        assert_eq!(actual, Some(gostand()));
    }

    #[test]
    fn a_turn_of_the_wheel_can_be_the_answer() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        app.world_mut().write_message(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: 1.0,
            window: Entity::PLACEHOLDER,
            phase: TouchPhase::Moved,
        });
        tick(&mut app);
        let wheel = wheel_button(1.0).expect("a turn up is a button");
        assert_eq!(bound(&app, wheel), Some(gostand()));
    }

    #[test]
    fn a_key_is_preferred_to_a_mouse_button_pressed_in_the_same_frame() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::KeyK);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyK)), Some(gostand()));
        assert_eq!(bound(&app, ActionBinding::Pointer(MouseButton::Left)), None);
    }

    #[test]
    fn rebinding_moves_the_keyboard_bind_and_leaves_the_pad_bind() {
        let mut app = capture_app();
        {
            let mut binds = app.world_mut().resource_mut::<BindingTable>();
            binds.assign(ActionBinding::Board(KeyCode::KeyG), gostand());
            binds.assign(ActionBinding::Pad(GamepadKey::Start), gostand());
        }
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyG)), None);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyK)), Some(gostand()));
        let actual = bound(&app, ActionBinding::Pad(GamepadKey::Start));
        assert_eq!(actual, Some(gostand()));
    }

    #[test]
    fn an_unknown_command_stops_a_prompt_in_progress() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        ask(&mut app, "no_such_command");
        tick(&mut app);
        assert_eq!(listening(&app), None);
        assert_eq!(asking(&app), None);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyK)), None);
    }

    #[test]
    fn closing_the_menu_drops_the_prompt() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        app.world_mut().resource_mut::<UiBindingCapture>().command = None;
        tick(&mut app);
        assert_eq!(listening(&app), None);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, ActionBinding::Board(KeyCode::KeyK)), None);
    }

    #[test]
    fn the_view_is_told_each_time_the_prompt_starts_and_stops() {
        let mut app = capture_app();
        let revision = |app: &App| app.world().resource::<ui::KeymapSnapshot>().revision;
        let before = revision(&app);
        ask(&mut app, GOSTAND);
        tick(&mut app);
        let started = revision(&app);
        assert_ne!(started, before);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_ne!(revision(&app), started);
    }

    #[test]
    fn a_pad_button_beats_everything_but_start() {
        let a = GamepadKey::from_gamepad_button(GamepadButton::South).expect("a pad button");
        let k = ActionBinding::Board(KeyCode::KeyK);
        let key = Some(k);
        assert_eq!(choose(false, Some(a), false, key), Some(Press::Pad(a)));
        assert_eq!(choose(false, Some(a), true, key), Some(Press::Pad(a)));
        assert_eq!(choose(true, Some(a), false, key), Some(Press::Cancel));
    }

    #[test]
    fn escape_and_start_cancel_before_a_key_binds() {
        let k = ActionBinding::Board(KeyCode::KeyK);
        let key = Some(k);
        assert_eq!(choose(false, None, true, key), Some(Press::Cancel));
        assert_eq!(choose(true, None, false, key), Some(Press::Cancel));
        assert_eq!(choose(false, None, false, key), Some(Press::Other(k)));
        assert_eq!(choose(false, None, false, None), None);
    }
}
