//! The gamepad: which stick does what, how a pull on the look stick becomes
//! turning, and which pad the player is using.

use std::collections::{HashMap, HashSet};

use bevy::{
    input::gamepad::{
        ButtonSettings, Gamepad, GamepadAxis, GamepadButton, GamepadConnection, GamepadEvent,
        GamepadSettings,
    },
    prelude::*,
    window::PrimaryWindow,
};
use frame::{ActivePad, GameSettings, InputDevices, PromptStyle};

/// What the two sticks mean once the layout has been applied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Sticks {
    /// Forward, then right.
    pub movement: Vec2,
    /// Right, then up.
    pub look: Vec2,
}

/// Which stick walks and which looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StickLayout {
    /// The left stick walks, the right looks.
    Default,
    /// The right stick walks, the left looks.
    Southpaw,
    /// The left stick walks forward and back and turns; the right looks up and
    /// down and strafes.
    Legacy,
    /// Legacy with the sticks swapped.
    LegacySouthpaw,
}

impl StickLayout {
    fn from_setting(setting: u8) -> Self {
        match setting {
            1 => Self::Southpaw,
            2 => Self::Legacy,
            3 => Self::LegacySouthpaw,
            _ => Self::Default,
        }
    }

    fn route(self, left: Vec2, right: Vec2) -> Sticks {
        let (movement, look) = match self {
            Self::Default => (Vec2::new(left.y, left.x), right),
            Self::Southpaw => (Vec2::new(right.y, right.x), left),
            Self::Legacy => (Vec2::new(left.y, right.x), Vec2::new(left.x, right.y)),
            Self::LegacySouthpaw => (Vec2::new(right.y, left.x), Vec2::new(right.x, left.y)),
        };
        Sticks { movement, look }
    }
}

/// The least the live part of a stick's travel is allowed to span, so a dead
/// zone near 1 cannot divide by nothing.
const MIN_LIVE_SPAN: f32 = 0.01;

/// `stick` with its dead zone taken out: nothing inside it, and the rest of the
/// travel stretched back over 0 to 1 along the same direction.
fn without_dead_zone(stick: Vec2, dead_zone: f32) -> Vec2 {
    let length = stick.length();
    if length <= dead_zone || length <= f32::EPSILON {
        return Vec2::ZERO;
    }
    let live = ((length - dead_zone) / (1.0 - dead_zone).max(MIN_LIVE_SPAN)).min(1.0);
    stick * (live / length)
}

/// The two sticks as the player has set the pad up.
pub(crate) fn route_sticks(left: Vec2, right: Vec2, settings: &GameSettings) -> Sticks {
    StickLayout::from_setting(settings.pad_stick_layout).route(
        without_dead_zone(left, settings.pad_deadzone_left),
        without_dead_zone(right, settings.pad_deadzone_right),
    )
}

pub(crate) fn sticks(pad: &Gamepad, settings: &GameSettings) -> Sticks {
    route_sticks(pad.left_stick(), pad.right_stick(), settings)
}

/// How far the look stick's pull is turned into turning speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LookCurve {
    /// Slow near the middle, fast at the edge.
    Standard,
    Linear,
    /// Quick to speed up, easing off at the edge.
    Quick,
}

impl LookCurve {
    fn from_setting(setting: u8) -> Self {
        match setting {
            1 => Self::Linear,
            2 => Self::Quick,
            _ => Self::Standard,
        }
    }

    /// The speed, 0 to 1, for a pull of `pull`, 0 to 1.
    fn speed(self, pull: f32) -> f32 {
        let pull = pull.clamp(0.0, 1.0);
        match self {
            Self::Standard => 0.35 * pull + 0.65 * pull.powi(3),
            Self::Linear => pull,
            Self::Quick => 1.0 - (1.0 - pull).powi(2),
        }
    }
}

/// The look stick's pull as turning, 0 to 1 each way, with the curve applied
/// and the up and down flipped if the player asked for it.
pub(crate) fn shaped_look(look: Vec2, settings: &GameSettings) -> Vec2 {
    let pull = look.length();
    if pull <= f32::EPSILON {
        return Vec2::ZERO;
    }
    let turn = look * (LookCurve::from_setting(settings.pad_curve).speed(pull) / pull);
    if settings.pad_invert {
        Vec2::new(turn.x, -turn.y)
    } else {
        turn
    }
}

/// BO2's default dead zone for a pad button: a trigger counts as pressed once
/// pulled past it.
const TRIGGER_DEAD_ZONE: f32 = 0.13;

fn bo2_trigger() -> ButtonSettings {
    ButtonSettings::new(TRIGGER_DEAD_ZONE + f32::EPSILON, TRIGGER_DEAD_ZONE)
        .expect("the release point is under the press point, both inside 0 to 1")
}

/// Gives each pad that connects BO2's trigger press point instead of the pad
/// library's, which waits for 75%.
pub(crate) fn bo2_trigger_thresholds(mut pads: Query<&mut GamepadSettings, Added<Gamepad>>) {
    for mut settings in &mut pads {
        for trigger in [GamepadButton::LeftTrigger2, GamepadButton::RightTrigger2] {
            settings.button_settings.insert(trigger, bo2_trigger());
        }
    }
}

/// A button's pull from which it counts as pressed by a person.
const BUTTON_PULL: f32 = 0.55;
/// How far past its dead zone a stick must be pushed to count.
const STICK_PUSH_MARGIN: f32 = 0.08;
/// How far a stick must have moved since it last rested to count.
const STICK_JUMP: f32 = 0.12;
/// The least dead zone judged: a worn stick drifts further than the setting.
const STICK_MIN_DEAD_ZONE: f32 = 0.15;

const STICK_AXES: [GamepadAxis; 4] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
];

/// Where each pad's buttons and sticks were last seen, to tell a person using
/// the pad from a stick that only drifts.
#[derive(Default)]
pub(crate) struct PadActivity {
    axes: HashMap<(Entity, GamepadAxis), f32>,
    buttons: HashMap<(Entity, GamepadButton), f32>,
}

impl PadActivity {
    fn forget(&mut self, pad: Entity) {
        self.axes.retain(|(seen, _), _| *seen != pad);
        self.buttons.retain(|(seen, _), _| *seen != pad);
    }

    fn keep_only(&mut self, exists: impl Fn(Entity) -> bool) {
        self.axes.retain(|(pad, _), _| exists(*pad));
        self.buttons.retain(|(pad, _), _| exists(*pad));
    }

    /// Takes `value` as where `axis` rests, so a stick held off-centre from
    /// the start is not a push.
    fn rest_at(&mut self, pad: Entity, axis: GamepadAxis, value: f32) {
        self.axes.insert((pad, axis), value);
    }

    /// Whether `button` has just been pulled past the point a person presses at.
    fn button_pulled(&mut self, pad: Entity, button: GamepadButton, value: f32) -> bool {
        let before = self.buttons.insert((pad, button), value).unwrap_or(0.0);
        value >= BUTTON_PULL && before < BUTTON_PULL
    }

    /// Whether `axis` has just been pushed clear of its dead zone and well away
    /// from where it was last at rest.
    fn axis_pushed(&mut self, pad: Entity, axis: GamepadAxis, value: f32, dead_zone: f32) -> bool {
        let dead_zone = dead_zone.max(STICK_MIN_DEAD_ZONE);
        let rest = self.axes.entry((pad, axis)).or_insert(0.0);
        let pushed =
            value.abs() > dead_zone + STICK_PUSH_MARGIN && (value - *rest).abs() > STICK_JUMP;
        if pushed || value.abs() < dead_zone {
            *rest = value;
        }
        pushed
    }
}

/// The dead zone the settings give `axis`.
fn dead_zone_of(axis: GamepadAxis, settings: &GameSettings) -> f32 {
    match axis {
        GamepadAxis::LeftStickX | GamepadAxis::LeftStickY => settings.pad_deadzone_left,
        _ => settings.pad_deadzone_right,
    }
}

/// The USB vendor ids of the makers whose pads show their own button names.
const SONY: u16 = 0x054c;
const MICROSOFT: u16 = 0x045e;

/// The button names to show: the player's choice, or the look of the pad in
/// use, or plain ones when it cannot be told.
fn prompt_style(setting: u8, pad: Option<(Option<u16>, &str)>) -> PromptStyle {
    match setting {
        1 => return PromptStyle::Xbox,
        2 => return PromptStyle::PlayStation,
        3 => return PromptStyle::Generic,
        _ => {}
    }
    let Some((vendor, name)) = pad else {
        return PromptStyle::Generic;
    };
    let name = name.to_ascii_lowercase();
    if vendor == Some(SONY)
        || name.contains("dualshock")
        || name.contains("dualsense")
        || name == "wireless controller"
    {
        PromptStyle::PlayStation
    } else if vendor == Some(MICROSOFT) || name.contains("xbox") || name.contains("xinput") {
        PromptStyle::Xbox
    } else {
        PromptStyle::Generic
    }
}

/// Whether the game takes input: while its window has focus, or when
/// `BO2MP_INPUT_UNFOCUSED` is set, which keeps the keys a test run holds
/// working with the window behind others.
fn takes_input(window_focused: Option<bool>) -> bool {
    static FORCED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    window_focused.unwrap_or(true)
        || *FORCED.get_or_init(|| std::env::var_os("BO2MP_INPUT_UNFOCUSED").is_some())
}

/// Follows which pad the player uses: the one they last pressed or pushed, and
/// the prompts to show for it.
pub(crate) fn track_active_pad(
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    mut events: MessageReader<GamepadEvent>,
    mut active: ResMut<ActivePad>,
    mut devices: ResMut<InputDevices>,
    settings: Res<GameSettings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut seen: Local<PadActivity>,
) {
    devices.focused = takes_input(windows.single().ok().map(|window| window.focused));
    if active.0.is_some_and(|pad| pads.get(pad).is_err()) {
        active.0 = None;
    }
    seen.keep_only(|pad| pads.contains(pad));
    let mut just_connected = HashSet::new();
    for event in events.read() {
        let acting = match event {
            GamepadEvent::Connection(change) => {
                match &change.connection {
                    GamepadConnection::Connected { .. } => {
                        just_connected.insert(change.gamepad);
                        if let Ok((_, pad, _)) = pads.get(change.gamepad) {
                            for axis in STICK_AXES {
                                seen.rest_at(change.gamepad, axis, pad.get(axis).unwrap_or(0.0));
                            }
                        }
                    }
                    GamepadConnection::Disconnected => {
                        seen.forget(change.gamepad);
                        if active.0 == Some(change.gamepad) {
                            active.0 = None;
                        }
                    }
                }
                None
            }
            GamepadEvent::Button(change) => seen
                .button_pulled(change.entity, change.button, change.value)
                .then_some(change.entity),
            GamepadEvent::Axis(change) => seen
                .axis_pushed(
                    change.entity,
                    change.axis,
                    change.value,
                    dead_zone_of(change.axis, &settings),
                )
                .then_some(change.entity),
        };
        if let Some(pad) = acting
            && devices.focused
            && !just_connected.contains(&pad)
            && pads.contains(pad)
        {
            active.0 = Some(pad);
            devices.pad_prompts = true;
        }
    }
    if active.0.is_none() {
        devices.pad_prompts = false;
    }
    let in_use = active
        .0
        .and_then(|pad| pads.get(pad).ok())
        .map(|(_, pad, name)| (pad.vendor_id(), name.map_or("", Name::as_str)));
    devices.style = prompt_style(settings.pad_prompts, in_use);
}

#[cfg(test)]
mod tests {
    use bevy::input::{
        ButtonState,
        gamepad::{GamepadAxisChangedEvent, GamepadButtonChangedEvent, GamepadConnectionEvent},
    };

    use super::*;

    fn settings() -> GameSettings {
        GameSettings {
            pad_deadzone_left: 0.0,
            pad_deadzone_right: 0.0,
            ..GameSettings::default()
        }
    }

    #[test]
    fn the_default_layout_walks_with_the_left_stick_and_looks_with_the_right() {
        let sticks = route_sticks(Vec2::new(0.2, 0.8), Vec2::new(-0.5, 0.4), &settings());
        assert_eq!(sticks.movement, Vec2::new(0.8, 0.2), "forward, then right");
        assert_eq!(sticks.look, Vec2::new(-0.5, 0.4));
    }

    #[test]
    fn southpaw_swaps_the_sticks() {
        let settings = GameSettings {
            pad_stick_layout: 1,
            ..settings()
        };
        let sticks = route_sticks(Vec2::new(0.2, 0.8), Vec2::new(-0.5, 0.4), &settings);
        assert_eq!(sticks.movement, Vec2::new(0.4, -0.5));
        assert_eq!(sticks.look, Vec2::new(0.2, 0.8));
    }

    #[test]
    fn legacy_splits_walking_and_looking_across_both_sticks() {
        let (left, right) = (Vec2::new(0.1, 0.2), Vec2::new(0.3, 0.4));
        let mut settings = settings();
        settings.pad_stick_layout = 2;
        let legacy = route_sticks(left, right, &settings);
        assert_eq!(
            legacy.movement,
            Vec2::new(0.2, 0.3),
            "left forward, right strafe"
        );
        assert_eq!(
            legacy.look,
            Vec2::new(0.1, 0.4),
            "left turn, right up and down"
        );
        settings.pad_stick_layout = 3;
        let southpaw = route_sticks(left, right, &settings);
        assert_eq!(southpaw.movement, Vec2::new(0.4, 0.1));
        assert_eq!(southpaw.look, Vec2::new(0.3, 0.2));
    }

    #[test]
    fn a_layout_setting_out_of_range_is_the_default_one() {
        assert_eq!(StickLayout::from_setting(0), StickLayout::Default);
        assert_eq!(StickLayout::from_setting(200), StickLayout::Default);
    }

    #[test]
    fn a_stick_inside_its_dead_zone_does_nothing() {
        assert_eq!(without_dead_zone(Vec2::new(0.1, 0.0), 0.12), Vec2::ZERO);
        assert_eq!(without_dead_zone(Vec2::new(0.12, 0.0), 0.12), Vec2::ZERO);
        assert_eq!(without_dead_zone(Vec2::ZERO, 0.0), Vec2::ZERO);
    }

    #[test]
    fn the_travel_past_the_dead_zone_is_stretched_over_the_whole_range() {
        let stretched = without_dead_zone(Vec2::new(0.0, 0.56), 0.12);
        assert!((stretched.y - 0.5).abs() < 1e-5, "{stretched:?}");
        assert_eq!(stretched.x, 0.0);
        let full = without_dead_zone(Vec2::new(0.0, -1.0), 0.12);
        assert!((full.y + 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_diagonal_keeps_its_direction() {
        let out = without_dead_zone(Vec2::new(0.6, 0.6), 0.2);
        assert!((out.x - out.y).abs() < 1e-6 && out.x > 0.0);
    }

    #[test]
    fn a_dead_zone_near_one_never_divides_by_nothing() {
        let out = without_dead_zone(Vec2::new(1.0, 0.0), 0.999);
        assert!(out.is_finite() && out.length() <= 1.0, "{out:?}");
        assert_eq!(without_dead_zone(Vec2::new(1.0, 0.0), 1.0), Vec2::ZERO);
    }

    #[test]
    fn every_curve_runs_from_nothing_to_full() {
        for curve in [LookCurve::Standard, LookCurve::Linear, LookCurve::Quick] {
            assert_eq!(curve.speed(0.0), 0.0);
            assert!((curve.speed(1.0) - 1.0).abs() < 1e-6);
            assert_eq!(curve.speed(5.0), curve.speed(1.0), "a pull is capped at 1");
            let mut last = 0.0;
            for step in 1..=20 {
                let speed = curve.speed(step as f32 / 20.0);
                assert!(speed >= last, "{curve:?} dips at {step}");
                last = speed;
            }
        }
    }

    #[test]
    fn the_curves_differ_in_the_middle_in_the_order_they_are_named() {
        let half = |curve: LookCurve| curve.speed(0.5);
        assert!(half(LookCurve::Standard) < half(LookCurve::Linear));
        assert!(half(LookCurve::Linear) < half(LookCurve::Quick));
        assert_eq!(LookCurve::from_setting(1), LookCurve::Linear);
        assert_eq!(LookCurve::from_setting(2), LookCurve::Quick);
        assert_eq!(LookCurve::from_setting(0), LookCurve::Standard);
    }

    #[test]
    fn looking_keeps_the_direction_of_the_pull_and_shapes_its_size() {
        let settings = GameSettings {
            pad_curve: 1,
            ..settings()
        };
        let turn = shaped_look(Vec2::new(0.3, 0.4), &settings);
        assert!(
            (turn - Vec2::new(0.3, 0.4)).length() < 1e-6,
            "linear: as pulled"
        );
        assert_eq!(shaped_look(Vec2::ZERO, &settings), Vec2::ZERO);
    }

    #[test]
    fn inverting_flips_up_and_down_only() {
        let mut settings = settings();
        settings.pad_curve = 1;
        let upright = shaped_look(Vec2::new(0.3, 0.4), &settings);
        settings.pad_invert = true;
        let flipped = shaped_look(Vec2::new(0.3, 0.4), &settings);
        assert_eq!(flipped, Vec2::new(upright.x, -upright.y));
    }

    #[test]
    fn a_trigger_counts_once_pulled_past_bo2s_dead_zone() {
        let trigger = bo2_trigger();
        // A pad reports a trigger as a byte out of 255: 33 is under 0.13, 34 over.
        assert!(!trigger.is_pressed(33.0 / 255.0) && trigger.is_released(33.0 / 255.0));
        assert!(trigger.is_pressed(34.0 / 255.0) && !trigger.is_released(34.0 / 255.0));
        assert!(trigger.is_released(0.13));
    }

    #[test]
    fn a_pad_that_connects_gets_the_trigger_points_and_only_the_triggers() {
        let mut app = App::new();
        app.add_systems(Update, bo2_trigger_thresholds);
        let pad = app
            .world_mut()
            .spawn((Gamepad::default(), GamepadSettings::default()))
            .id();
        app.update();
        let settings = app.world().get::<GamepadSettings>(pad).unwrap();
        assert_eq!(settings.button_settings.len(), 2);
        assert!(
            settings
                .button_settings
                .contains_key(&GamepadButton::LeftTrigger2)
        );
        assert!(
            settings
                .button_settings
                .contains_key(&GamepadButton::RightTrigger2)
        );
    }

    fn pad_id(index: u32) -> Entity {
        Entity::from_raw_u32(index).unwrap()
    }

    #[test]
    fn a_button_counts_when_it_crosses_the_press_point_not_while_held() {
        let (mut seen, pad) = (PadActivity::default(), pad_id(1));
        assert!(!seen.button_pulled(pad, GamepadButton::South, 0.3));
        assert!(seen.button_pulled(pad, GamepadButton::South, 0.9));
        assert!(
            !seen.button_pulled(pad, GamepadButton::South, 1.0),
            "still held"
        );
        assert!(!seen.button_pulled(pad, GamepadButton::South, 0.0));
        assert!(
            seen.button_pulled(pad, GamepadButton::South, 1.0),
            "pressed again"
        );
    }

    #[test]
    fn a_stick_pushed_well_clear_of_its_dead_zone_counts() {
        let (mut seen, pad) = (PadActivity::default(), pad_id(1));
        assert!(!seen.axis_pushed(pad, GamepadAxis::LeftStickX, 0.2, 0.12));
        assert!(seen.axis_pushed(pad, GamepadAxis::LeftStickX, 0.9, 0.12));
    }

    #[test]
    fn a_stick_that_drifts_slowly_never_counts() {
        let (mut seen, pad) = (PadActivity::default(), pad_id(1));
        let drift = [0.05, 0.10, 0.15, 0.20, 0.22];
        assert!(
            drift
                .iter()
                .all(|v| !seen.axis_pushed(pad, GamepadAxis::RightStickY, *v, 0.12))
        );
    }

    #[test]
    fn a_stick_that_started_off_centre_is_not_a_push_until_it_moves() {
        let (mut seen, pad) = (PadActivity::default(), pad_id(1));
        seen.rest_at(pad, GamepadAxis::LeftStickY, 0.8);
        assert!(!seen.axis_pushed(pad, GamepadAxis::LeftStickY, 0.85, 0.12));
        assert!(seen.axis_pushed(pad, GamepadAxis::LeftStickY, -0.8, 0.12));
    }

    #[test]
    fn a_small_dead_zone_setting_is_raised_to_the_least_judged() {
        let (mut seen, pad) = (PadActivity::default(), pad_id(1));
        assert!(
            !seen.axis_pushed(pad, GamepadAxis::LeftStickX, 0.2, 0.0),
            "0.2 is not past 0.15 + 0.08"
        );
        assert!(seen.axis_pushed(pad, GamepadAxis::LeftStickX, 0.4, 0.0));
    }

    #[test]
    fn forgetting_a_pad_leaves_the_others() {
        let (mut seen, a, b) = (PadActivity::default(), pad_id(1), pad_id(2));
        seen.button_pulled(a, GamepadButton::South, 1.0);
        seen.button_pulled(b, GamepadButton::South, 1.0);
        seen.forget(a);
        assert!(
            seen.button_pulled(a, GamepadButton::South, 1.0),
            "a starts over"
        );
        assert!(
            !seen.button_pulled(b, GamepadButton::South, 1.0),
            "b is still held"
        );
        seen.keep_only(|pad| pad == a);
        assert!(
            seen.button_pulled(b, GamepadButton::South, 1.0),
            "b was dropped"
        );
    }

    #[test]
    fn the_prompts_follow_the_setting_then_the_pad() {
        let sony = Some((Some(SONY), "pad"));
        assert_eq!(prompt_style(1, sony), PromptStyle::Xbox);
        assert_eq!(prompt_style(2, None), PromptStyle::PlayStation);
        assert_eq!(prompt_style(3, sony), PromptStyle::Generic);
        assert_eq!(prompt_style(0, sony), PromptStyle::PlayStation);
        assert_eq!(prompt_style(0, None), PromptStyle::Generic);
    }

    #[test]
    fn a_pad_is_known_by_its_maker_or_its_name() {
        let style = |vendor, name| prompt_style(0, Some((vendor, name)));
        assert_eq!(style(Some(MICROSOFT), "Controller"), PromptStyle::Xbox);
        assert_eq!(style(None, "Xbox Wireless Controller"), PromptStyle::Xbox);
        assert_eq!(style(None, "XInput Gamepad"), PromptStyle::Xbox);
        assert_eq!(style(None, "DualSense Wireless"), PromptStyle::PlayStation);
        assert_eq!(style(None, "Wireless Controller"), PromptStyle::PlayStation);
        assert_eq!(style(None, "Wireless Controller 2"), PromptStyle::Generic);
        assert_eq!(style(Some(0x1234), "Some Pad"), PromptStyle::Generic);
    }

    #[test]
    fn a_missing_window_counts_as_focused() {
        assert!(takes_input(None) && takes_input(Some(true)));
    }

    fn tracking_app() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_message::<GamepadEvent>()
            .init_resource::<ActivePad>()
            .init_resource::<InputDevices>()
            .insert_resource(settings())
            .add_systems(Update, track_active_pad);
        let first = app.world_mut().spawn(Gamepad::default()).id();
        let second = app.world_mut().spawn(Gamepad::default()).id();
        (app, first, second)
    }

    fn press(app: &mut App, pad: Entity) {
        app.world_mut()
            .write_message(GamepadEvent::Button(GamepadButtonChangedEvent::new(
                pad,
                GamepadButton::South,
                ButtonState::Pressed,
                1.0,
            )));
        app.update();
    }

    #[test]
    fn the_pad_a_button_was_pressed_on_becomes_the_active_one() {
        let (mut app, first, second) = tracking_app();
        app.update();
        assert_eq!(app.world().resource::<ActivePad>().0, None);
        assert!(!app.world().resource::<InputDevices>().pad_prompts);
        press(&mut app, second);
        assert_eq!(app.world().resource::<ActivePad>().0, Some(second));
        assert!(app.world().resource::<InputDevices>().pad_prompts);
        press(&mut app, first);
        assert_eq!(app.world().resource::<ActivePad>().0, Some(first));
    }

    #[test]
    fn a_stick_pushed_hard_makes_the_pad_active() {
        let (mut app, first, _) = tracking_app();
        app.world_mut()
            .write_message(GamepadEvent::Axis(GamepadAxisChangedEvent::new(
                first,
                GamepadAxis::LeftStickX,
                1.0,
            )));
        app.update();
        assert_eq!(app.world().resource::<ActivePad>().0, Some(first));
    }

    #[test]
    fn a_pad_that_is_unplugged_stops_being_active() {
        let (mut app, first, _) = tracking_app();
        press(&mut app, first);
        app.world_mut().despawn(first);
        app.update();
        assert_eq!(app.world().resource::<ActivePad>().0, None);
        assert!(!app.world().resource::<InputDevices>().pad_prompts);
    }

    #[test]
    fn a_pad_that_reports_disconnected_stops_being_active() {
        let (mut app, first, _) = tracking_app();
        press(&mut app, first);
        app.world_mut()
            .write_message(GamepadEvent::Connection(GamepadConnectionEvent::new(
                first,
                GamepadConnection::Disconnected,
            )));
        app.update();
        assert_eq!(app.world().resource::<ActivePad>().0, None);
    }

    #[test]
    fn the_prompt_style_follows_the_setting_for_the_active_pad() {
        let (mut app, first, _) = tracking_app();
        app.world_mut().resource_mut::<GameSettings>().pad_prompts = 1;
        press(&mut app, first);
        assert_eq!(
            app.world().resource::<InputDevices>().style,
            PromptStyle::Xbox
        );
    }
}
