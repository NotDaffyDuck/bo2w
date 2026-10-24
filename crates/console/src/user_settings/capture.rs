//! Binding a command to whatever the player presses next, when the controls
//! menu asks for one.

use bevy::{
    ecs::system::SystemParam,
    input::{
        ButtonInput,
        gamepad::{Gamepad, GamepadButton},
        keyboard::KeyCode,
        mouse::{MouseButton, MouseWheel},
    },
    prelude::*,
};
use frame::{ActivePad, GameSettings, UiBindRequest, UiBindingCapture};

use crate::{BindButton, KeyBinds, PadButton, binds::wheel_button};

/// The command waiting for its button. `armed` is false on the frame the menu
/// asked, so the click that opened the prompt is not taken as the answer.
#[derive(Resource, Default)]
pub(crate) struct PendingMenuBinding {
    id: Option<u32>,
    armed: bool,
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
    Pad(PadButton),
    Cancel,
    Other(BindButton),
}

/// What this frame's presses mean. A pad button wins over everything but
/// Start, which (like Escape) backs out; a key, then a mouse button, then the
/// wheel come last.
fn choose(
    pad_start: bool,
    pad_button: Option<PadButton>,
    escape: bool,
    other: Option<BindButton>,
) -> Option<Press> {
    if let Some(button) = pad_button.filter(|_| !pad_start) {
        return Some(Press::Pad(button));
    }
    if escape || pad_start {
        return Some(Press::Cancel);
    }
    other.map(Press::Other)
}

fn stop_listening(
    pending: &mut PendingMenuBinding,
    capture: &mut UiBindingCapture,
    view: &mut ui::BindingView,
) {
    capture.command = None;
    capture.consumed_input = true;
    pending.id = None;
    pending.armed = false;
    view.listening = None;
    view.revision = view.revision.wrapping_add(1);
}

pub(crate) fn consume_menu_binding(
    mut input: Inputs,
    mut settings: ResMut<GameSettings>,
    mut pending: ResMut<PendingMenuBinding>,
    mut capture: ResMut<UiBindingCapture>,
    mut binds: ResMut<KeyBinds>,
    mut view: ResMut<ui::BindingView>,
) {
    capture.consumed_input = false;
    if capture.command.is_none() {
        pending.id = None;
        view.listening = None;
    }
    // Every wheel event is read so none is left over for the next frame.
    let wheel = input
        .wheel
        .read()
        .fold(None, |first, event| first.or_else(|| wheel_button(event.y)));
    let mut asked = false;
    for request in input.requests.read() {
        match input_t6::command_id_lookup(&request.command) {
            Some(id) => {
                capture.command = Some(request.command.clone());
                pending.id = Some(id);
                pending.armed = false;
                view.listening = Some(id);
                view.revision = view.revision.wrapping_add(1);
                asked = true;
            }
            None => {
                capture.command = None;
                pending.id = None;
                view.listening = None;
            }
        }
    }
    let Some(id) = pending.id else { return };
    if asked || !pending.armed {
        pending.armed = true;
        return;
    }
    let pad = input.active.0.and_then(|pad| input.pads.get(pad).ok());
    let pad_start = pad.is_some_and(|pad| pad.just_pressed(GamepadButton::Start));
    let pad_button = pad.and_then(|pad| {
        pad.get_just_pressed()
            .copied()
            .filter_map(PadButton::from_gamepad_button)
            .find(|button| *button != PadButton::Start)
    });
    let other = input
        .keys
        .get_just_pressed()
        .next()
        .copied()
        .map(BindButton::Key)
        .or_else(|| {
            input
                .mouse
                .get_just_pressed()
                .next()
                .copied()
                .map(BindButton::Mouse)
        })
        .or(wheel);
    let escape = input.keys.just_pressed(KeyCode::Escape);
    match choose(pad_start, pad_button, escape, other) {
        None => {}
        Some(Press::Cancel) => stop_listening(&mut pending, &mut capture, &mut view),
        Some(Press::Pad(button)) => {
            binds.clear_command_on(id, true);
            binds.set(BindButton::Pad(button), id);
            // A pad button picked by hand is no longer one of the preset layouts.
            if settings.pad_layout != GameSettings::PAD_LAYOUT_CUSTOM {
                settings.pad_layout = GameSettings::PAD_LAYOUT_CUSTOM;
                settings.touch();
            }
            stop_listening(&mut pending, &mut capture, &mut view);
        }
        Some(Press::Other(button)) => {
            binds.clear_command_on(id, false);
            binds.set(button, id);
            stop_listening(&mut pending, &mut capture, &mut view);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::{mouse::MouseScrollUnit, touch::TouchPhase};

    use super::*;

    const GOSTAND: &str = "+gostand";

    fn gostand() -> u32 {
        input_t6::command_id_lookup(GOSTAND).expect("a known command")
    }

    fn capture_app() -> App {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ActivePad>()
            .init_resource::<GameSettings>()
            .init_resource::<PendingMenuBinding>()
            .init_resource::<UiBindingCapture>()
            .init_resource::<KeyBinds>()
            .init_resource::<ui::BindingView>()
            .add_message::<MouseWheel>()
            .add_message::<UiBindRequest>()
            .add_systems(Update, consume_menu_binding);
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

    fn bound(app: &App, button: BindButton) -> Option<u32> {
        app.world().resource::<KeyBinds>().get(button)
    }

    fn listening(app: &App) -> Option<u32> {
        app.world().resource::<ui::BindingView>().listening
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
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyJ)), None);
    }

    #[test]
    fn the_next_key_is_bound_and_listening_stops() {
        let mut app = capture_app();
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyK)), Some(gostand()));
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
        assert_eq!(bound(&app, BindButton::Key(KeyCode::Escape)), None);
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
        assert_eq!(
            bound(&app, BindButton::Mouse(MouseButton::Right)),
            Some(gostand())
        );
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
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyK)), Some(gostand()));
        assert_eq!(bound(&app, BindButton::Mouse(MouseButton::Left)), None);
    }

    #[test]
    fn rebinding_moves_the_keyboard_bind_and_leaves_the_pad_bind() {
        let mut app = capture_app();
        {
            let mut binds = app.world_mut().resource_mut::<KeyBinds>();
            binds.set(BindButton::Key(KeyCode::KeyG), gostand());
            binds.set(BindButton::Pad(PadButton::Start), gostand());
        }
        ask(&mut app, GOSTAND);
        tick(&mut app);
        press(&mut app, KeyCode::KeyK);
        tick(&mut app);
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyG)), None);
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyK)), Some(gostand()));
        assert_eq!(
            bound(&app, BindButton::Pad(PadButton::Start)),
            Some(gostand())
        );
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
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyK)), None);
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
        assert_eq!(bound(&app, BindButton::Key(KeyCode::KeyK)), None);
    }

    #[test]
    fn the_view_is_told_each_time_the_prompt_starts_and_stops() {
        let mut app = capture_app();
        let revision = |app: &App| app.world().resource::<ui::BindingView>().revision;
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
        let a = PadButton::from_gamepad_button(GamepadButton::South).expect("a pad button");
        let k = BindButton::Key(KeyCode::KeyK);
        let key = Some(k);
        assert_eq!(choose(false, Some(a), false, key), Some(Press::Pad(a)));
        assert_eq!(choose(false, Some(a), true, key), Some(Press::Pad(a)));
        assert_eq!(choose(true, Some(a), false, key), Some(Press::Cancel));
    }

    #[test]
    fn escape_and_start_cancel_before_a_key_binds() {
        let k = BindButton::Key(KeyCode::KeyK);
        let key = Some(k);
        assert_eq!(choose(false, None, true, key), Some(Press::Cancel));
        assert_eq!(choose(true, None, false, key), Some(Press::Cancel));
        assert_eq!(choose(false, None, false, key), Some(Press::Other(k)));
        assert_eq!(choose(false, None, false, None), None);
    }
}
