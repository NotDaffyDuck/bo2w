//! Input played on the player's behalf: inputs held or pressed from the
//! console, keys and mouse buttons held for a while, mouse motion, and the key
//! binds. The commands that ask for them are answered here.


use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::state::PromptPanelOptions;
use super::state::PromptPanelStatus;
use crate::ActionBinding;
use crate::TypedOrder;
use crate::ScriptedControlsStatus;
use crate::BindingTable;
use crate::TAP_HOLD_SECONDS;
use crate::ScriptedKeys;
use crate::keymap::button_from_name;
use crate::names_control_order;
use std::hash::Hash;

/// The longest a timed press may last.
const LONGEST_PRESS_SECONDS: f32 = 60.0;

/// Mouse buttons held for a while by `key mouse1` and the like: the button,
/// the seconds left, and whether it has gone down yet.
#[derive(Default, Resource)]
pub(super) struct ScriptedMouse(pub(super) Vec<(MouseButton, f32, bool)>);

/// The keys and mouse buttons a script is holding down.
#[derive(SystemParam)]
pub(super) struct Timed<'w> {
    keys: Res<'w, ScriptedKeys>,
    mouse: Res<'w, ScriptedMouse>,
}

impl Timed<'_> {
    /// Whether a script, rather than the player, is holding `control`.
    pub(super) fn is_down(&self, control: ActionBinding) -> bool {
        match control {
            ActionBinding::Board(code) => self.keys.0.iter().any(|(held, ..)| *held == code),
            ActionBinding::Pointer(held) => self.mouse.0.iter().any(|(mouse, ..)| *mouse == held),
            _ => false,
        }
    }
}

/// Presses what is new, keeps down what the window let go of (a hidden or
/// unfocused window releases every key), and lets go of what has run out of
/// time.
fn advance_timed<T>(dt: f32, timed: &mut Vec<(T, f32, bool)>, input: &mut ButtonInput<T>)
where
    T: Copy + Eq + Hash + Send + Sync + 'static,
{
    timed.retain_mut(|(button, left, started)| {
        let first_frame = !*started;
        if first_frame {
            input.press(*button);
            *started = true;
            true
        } else {
            *left -= dt;
            let keep = *left > 0.0;
            if !keep {
                input.release(*button);
            } else if !input.pressed(*button) {
                input.press(*button);
            }
            keep
        }
    });
}

pub(super) fn drive_scripted_keys(
    clock: Res<Time>,
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut scripted: ResMut<ScriptedKeys>,
) {
    advance_timed(clock.delta_secs(), &mut scripted.0, keyboard.as_mut());
}

pub(super) fn drive_scripted_mouse(
    clock: Res<Time>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    mut scripted: ResMut<ScriptedMouse>,
) {
    advance_timed(clock.delta_secs(), &mut scripted.0, mouse_buttons.as_mut());
}

pub(super) fn expire_pressed_inputs(clock: Res<Time>, mut inputs: ResMut<ScriptedControlsStatus>) {
    inputs.step_timers(clock.delta_secs());
}

/// What an input command can change.
struct Targets<'a> {
    inputs: &'a mut ScriptedControlsStatus,
    keys: &'a mut ScriptedKeys,
    mouse: &'a mut ScriptedMouse,
    binds: &'a mut BindingTable,
    console: &'a mut PromptPanelStatus,
}

fn one(line: impl Into<String>) -> Vec<String> {
    vec![line.into()]
}

/// `hold +actionslot 2` and `+actionslot 2` name a single input, "+actionslot
/// 2", that splitting on spaces has torn in two.
fn join_actionslot(mut command: TypedOrder) -> TypedOrder {
    let slot_word = |word: &str| matches!(word, "+actionslot" | "-actionslot");
    let takes_inputs = matches!(command.name.as_str(), "hold" | "press" | "release");
    if takes_inputs && command.words.len() >= 2 && slot_word(&command.words[0]) {
        let slot = command.words.remove(1);
        command.words[0] = format!("{} {slot}", command.words[0]);
    } else if slot_word(&command.name) && command.words.len() == 1 {
        let slot = command.words.pop().unwrap_or_default();
        command.name.push(' ');
        command.name.push_str(&slot);
    }
    command
}

/// A number of seconds, with or without a trailing `s`.
fn seconds_arg(arg: Option<&String>) -> f32 {
    let parsed = arg.map(|text| text.trim_end_matches(['s', 'S'])).and_then(|digits| digits.parse::<f32>().ok());
    let seconds = match parsed {
        Some(value) if value.is_finite() => value,
        _ => TAP_HOLD_SECONDS,
    };
    seconds.clamp(0.0, LONGEST_PRESS_SECONDS)
}

/// `<dx>` or `<dx> <dy>`; a missing `dy` is 0.
fn pair(args: &[String]) -> Option<(f32, f32)> {
    let (dx, dy) = match args {
        [dx] => (dx, None),
        [dx, dy] => (dx, Some(dy)),
        _other => return None,
    };
    let across = dx.parse().ok()?;
    let down = dy.map_or(Some(0.0), |text| text.parse().ok())?;
    Some((across, down))
}

/// Every word names a console input (and there is at least one).
fn only_inputs(args: &[String]) -> bool {
    !args.is_empty() && args.iter().all(|word| names_control_order(word))
}

fn hold(held: &mut ScriptedControlsStatus, args: &[String]) -> Vec<String> {
    if !only_inputs(args) {
        return one("usage: hold <+input>...");
    }
    args.iter().for_each(|word| {
        held.keep_down(word);
    });
    one(format!("held: {}", args.join(" ")))
}

fn press(held: &mut ScriptedControlsStatus, args: &[String]) -> Vec<String> {
    let first_other = args.iter().position(|word| !names_control_order(word)).unwrap_or(args.len());
    if first_other == 0 {
        return one("usage: press <+input>... [seconds]");
    }
    let seconds = seconds_arg(args.get(first_other));
    let names = &args[..first_other];
    names.iter().for_each(|word| {
        held.tap(word, seconds);
    });
    one(format!("pressed: {} ({seconds:.2}s)", names.join(" ")))
}

fn release(status: &mut ScriptedControlsStatus, args: &[String]) -> Vec<String> {
    if let [only] = args
        && only == "all"
    {
        status.release_all();
        return one("held inputs cleared");
    }
    if !only_inputs(args) {
        return one("usage: release <+input...|all>");
    }
    for word in args {
        status.let_go(word);
    }
    one(format!("released: {}", args.join(" ")))
}

/// The key `name` stands for, or failing that its mouse button. Anything
/// else it might bind to (a pad button, a wheel) cannot be held from here.
fn holdable_button(name: &str) -> Option<ActionBinding> {
    let bound: Vec<ActionBinding> = button_from_name(name).into_iter().flatten().collect();
    let first_key = bound.iter().copied().find(|b| matches!(b, ActionBinding::Board(_)));
    first_key.or_else(|| bound.iter().copied().find(|b| matches!(b, ActionBinding::Pointer(_))))
}

/// The three ways `key`, `keydown` and `keyup` change a held button.
#[derive(Copy, Clone)]
enum KeyAction {
    /// `key <name> [seconds]`: one more timed hold.
    Timed,
    /// `keydown <name>`: held with no end until `keyup`.
    Down,
    /// `keyup <name>`: every hold of it runs out, so the next frame lets go.
    Up,
}

impl KeyAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Timed => "key",
            Self::Down => "keydown",
            Self::Up => "keyup",
        }
    }

    fn usage(self) -> &'static str {
        match self {
            Self::Timed => "usage: key <key> [seconds]",
            Self::Down => "usage: keydown <key>",
            Self::Up => "usage: keyup <key>",
        }
    }

    /// Changes the holds in `timed`; for a timed hold returns its length.
    fn apply<T: PartialEq>(self, timed: &mut Vec<(T, f32, bool)>, button: T, length: Option<&String>) -> f32 {
        match self {
            Self::Timed => {
                let seconds = seconds_arg(length);
                timed.push((button, seconds, false));
                seconds
            }
            Self::Down => {
                set_time_left(timed, button, f32::INFINITY, true);
                f32::INFINITY
            }
            Self::Up => {
                set_time_left(timed, button, 0.0, false);
                0.0
            }
        }
    }
}

fn key_command(targets: &mut Targets, words: &[String], action: KeyAction) -> Vec<String> {
    let verb = action.verb();
    let name_ok = match action {
        KeyAction::Timed => !words.is_empty(),
        KeyAction::Down | KeyAction::Up => words.len() == 1,
    };
    if !name_ok {
        return one(action.usage());
    }
    let name = words[0].as_str();
    let length = words.get(1);
    let seconds = match holdable_button(name) {
        Some(ActionBinding::Board(code)) => action.apply(&mut targets.keys.0, code, length),
        Some(ActionBinding::Pointer(button)) => action.apply(&mut targets.mouse.0, button, length),
        _ => return one(format!("{verb}: no key `{name}`")),
    };
    match (action, seconds) {
        (KeyAction::Timed, secs) => one(format!("{verb}: {name} ({secs:.2}s)")),
        (KeyAction::Down | KeyAction::Up, _) => one(format!("{verb}: {name}")),
    }
}

/// Gives every hold of `button` `seconds` left. With `add`, only the first
/// hold changes, and a button not held yet gets a new hold.
fn set_time_left<T: PartialEq>(timed: &mut Vec<(T, f32, bool)>, button: T, seconds: f32, add: bool) {
    if !add {
        for (held, left, _) in timed.iter_mut() {
            if *held == button {
                *left = seconds;
            }
        }
        return;
    }
    if let Some(hold) = timed.iter_mut().find(|(held, ..)| *held == button) {
        hold.1 = seconds;
    } else {
        timed.push((button, seconds, false));
    }
}

/// `mousemove` and `mouserate`: a motion pair handed to `apply`.
fn mouse_pair(verb: &str, words: &[String], apply: impl FnOnce(f32, f32)) -> Vec<String> {
    let Some((x, y)) = pair(words) else {
        return one(format!("usage: {verb} <dx> [dy]"));
    };
    apply(x, y);
    one(format!("{verb} {x} {y}"))
}

fn bind(binds: &mut BindingTable, args: &[String]) -> Vec<String> {
    if !args.is_empty() {
        return binds.run_control_script(&format!("bind {}", args.join(" ")));
    }
    let listed = binds.control_listing();
    if listed.is_empty() { one("no binds") } else { listed }
}

fn toggle_console(console: &mut PromptPanelStatus) -> Vec<String> {
    console.shown ^= true;
    let state = if console.shown { "open" } else { "closed" };
    one(format!("console {state}"))
}

/// `con_feed <text>` types the text into the prompt on the next frame.
fn feed_console(console: &mut PromptPanelStatus, typed: &[String]) -> Vec<String> {
    if typed.is_empty() {
        return one("usage: con_feed <text>");
    }
    console.pending_feed = Some(typed.join(" "));
    vec![]
}

/// `con_paste` puts the system clipboard into the prompt on the next frame.
fn paste_into_console(panel: &mut PromptPanelStatus) -> Vec<String> {
    panel.pending_os_paste |= true;
    vec![]
}

fn clear_binds(map: &mut BindingTable) -> Vec<String> {
    map.forget_all();
    one("unbindall")
}

fn restore_binds(binds: &mut BindingTable) -> Vec<String> {
    binds.restore_stock_controls();
    one("restored default binds")
}

/// `+input` holds it and `-input` lets it go; both print nothing. None when
/// the name is neither.
fn plus_minus(name: &str, held: &mut ScriptedControlsStatus) -> Option<()> {
    match name.as_bytes().first()? {
        b'+' if names_control_order(name) => {
            held.keep_down(name);
        }
        b'+' => return None,
        b'-' => {
            let plus = format!("+{}", &name[1..]);
            if names_control_order(&plus) {
                held.let_go(&plus);
            }
        }
        _other => return None,
    }
    Some(())
}

/// Carries out one input command and returns the lines it prints.
fn run(command: &TypedOrder, targets: &mut Targets) -> Vec<String> {
    let (name, args) = (command.name.as_str(), command.words.as_slice());
    if plus_minus(name, targets.inputs).is_some() {
        return vec![];
    }
    let inputs = &mut *targets.inputs;
    match name {
        "hold" => hold(inputs, args),
        "press" => press(inputs, args),
        "release" => release(inputs, args),
        "mousemove" => mouse_pair(name, args, |x, y| inputs.stage_pointer_move(x, y)),
        "mouserate" => mouse_pair(name, args, |x, y| inputs.assign_pointer_velocity(x, y)),
        "key" => key_command(targets, args, KeyAction::Timed),
        "keydown" => key_command(targets, args, KeyAction::Down),
        "keyup" => key_command(targets, args, KeyAction::Up),
        "bind" => bind(targets.binds, args),
        "unbind" => targets.binds.run_control_script(&format!("unbind {}", args.join(" "))),
        "unbindall" => clear_binds(targets.binds),
        "binddefaults" => restore_binds(targets.binds),
        "con_toggle" => toggle_console(targets.console),
        "con_feed" => feed_console(targets.console, args),
        "con_paste" => paste_into_console(targets.console),
        _ => vec![],
    }
}

pub(super) fn handle_input_commands(
    mut commands: MessageReader<TypedOrder>,
    mut inputs: ResMut<ScriptedControlsStatus>,
    mut keys: ResMut<ScriptedKeys>,
    mut mouse: ResMut<ScriptedMouse>,
    mut binds: ResMut<BindingTable>,
    mut console: ResMut<PromptPanelStatus>,
    settings: Res<PromptPanelOptions>,
) {
    let mut targets = Targets {
        inputs: &mut inputs,
        keys: &mut keys,
        mouse: &mut mouse,
        binds: &mut binds,
        console: &mut console,
    };
    let capacity = settings.backlog_limit;
    for asked in commands.read() {
        let printed = run(&join_actionslot(asked.clone()), &mut targets);
        printed.into_iter().for_each(|line| targets.console.post_line(line, capacity));
    }
}

#[cfg(test)]
mod tests {
    use crate::app::scripted::*;

    #[derive(Default)]
    struct ScriptedSetup {
        inputs: ScriptedControlsStatus,
        keys: ScriptedKeys,
        mouse: ScriptedMouse,
        binds: BindingTable,
        console: PromptPanelStatus,
    }

    impl ScriptedSetup {
        fn say(&mut self, line: &str) -> Vec<String> {
            let mut words = line.split_whitespace();
            let command = TypedOrder {
                name: words.next().unwrap_or("").to_owned(),
                words: words.map(str::to_owned).collect(),
                as_written: line.to_owned(),
                detached: false,
                typed_by_hand: true,
            };
            run(
                &join_actionslot(command),
                &mut Targets {
                    inputs: &mut self.inputs,
                    keys: &mut self.keys,
                    mouse: &mut self.mouse,
                    binds: &mut self.binds,
                    console: &mut self.console,
                },
            )
        }
    }

    #[test]
    fn a_plus_command_holds_and_its_minus_lets_go() {
        let mut fixture = ScriptedSetup::default();
        assert!(fixture.say("+forward").is_empty());
        assert!(fixture.inputs.is_down("+forward"));
        assert!(fixture.say("-forward").is_empty());
        assert!(!fixture.inputs.is_down("+forward"));
    }

    #[test]
    fn hold_takes_several_inputs_at_once() {
        let mut fixture = ScriptedSetup::default();
        let expected = ["held: +forward +attack"];
        assert_eq!(fixture.say("hold +forward +attack"), expected);
        assert!(fixture.inputs.is_down("+forward") && fixture.inputs.is_down("+attack"));
        assert_eq!(fixture.say("hold"), ["usage: hold <+input>..."]);
        let expected = ["usage: hold <+input>..."];
        assert_eq!(fixture.say("hold +forward nonsense"), expected);
    }

    #[test]
    fn press_holds_for_the_seconds_given_or_a_default() {
        let mut fixture = ScriptedSetup::default();
        let actual = fixture.say("press +attack 0.5s");
        assert_eq!(actual, ["pressed: +attack (0.50s)"]);
        let actual = fixture.say("press +attack");
        assert_eq!(actual, [format!("pressed: +attack ({TAP_HOLD_SECONDS:.2}s)")]);
        let expected = ["usage: press <+input>... [seconds]"];
        assert_eq!(fixture.say("press 2"), expected);
    }

    #[test]
    fn press_keeps_its_time_within_a_minute() {
        let mut fixture = ScriptedSetup::default();
        let actual = fixture.say("press +attack 900");
        assert_eq!(actual, ["pressed: +attack (60.00s)"]);
        let actual = fixture.say("press +attack nan");
        assert_eq!(actual, [format!("pressed: +attack ({TAP_HOLD_SECONDS:.2}s)")]);
    }

    #[test]
    fn release_lets_go_of_some_or_all() {
        let mut fixture = ScriptedSetup::default();
        fixture.say("hold +forward +attack");
        assert_eq!(fixture.say("release +forward"), ["released: +forward"]);
        assert!(!fixture.inputs.is_down("+forward") && fixture.inputs.is_down("+attack"));
        assert_eq!(fixture.say("release all"), ["held inputs cleared"]);
        assert!(!fixture.inputs.is_down("+attack"));
        assert_eq!(fixture.say("release"), ["usage: release <+input...|all>"]);
    }

    #[test]
    fn an_action_slot_is_one_input_though_it_has_a_space() {
        let command = |name: &str, args: &[&str]| TypedOrder {
            name: String::from(name),
            words: args.iter().map(|arg| (*arg).to_owned()).collect(),
            as_written: String::new(),
            detached: false,
            typed_by_hand: false,
        };
        let held = join_actionslot(command("hold", &["+actionslot", "2"]));
        let actual = (held.name.as_str(), held.words.as_slice());
        assert_eq!(actual, ("hold", ["+actionslot 2".to_owned()].as_slice()));
        let bare = join_actionslot(command("-actionslot", &["3"]));
        assert_eq!((bare.name.as_str(), bare.words.len()), ("-actionslot 3", 0));
        let other = join_actionslot(command("hold", &["+forward", "2"]));
        assert_eq!(other.words, ["+forward", "2"]);
    }

    #[test]
    fn key_holds_a_key_or_a_mouse_button() {
        let mut fixture = ScriptedSetup::default();
        assert_eq!(fixture.say("key w 0.5"), ["key: w (0.50s)"]);
        assert_eq!(fixture.keys.0, [(KeyCode::KeyW, 0.5, false)]);
        let actual = fixture.say("key mouse1");
        assert_eq!(actual, [format!("key: mouse1 ({TAP_HOLD_SECONDS:.2}s)")]);
        assert_eq!(fixture.mouse.0.len(), 1);
        assert_eq!(fixture.say("key nosuchkey"), ["key: no key `nosuchkey`"]);
        assert_eq!(fixture.say("key"), ["usage: key <key> [seconds]"]);
        assert_eq!(fixture.keys.0.len(), 1);
    }

    #[test]
    fn keydown_holds_a_key_until_keyup_runs_its_time_out() {
        let mut fixture = ScriptedSetup::default();
        assert_eq!(fixture.say("keydown shift"), ["keydown: shift"]);
        assert_eq!(fixture.say("keydown shift"), ["keydown: shift"]);
        assert_eq!(fixture.keys.0.len(), 1, "one hold per key");
        assert_eq!(fixture.keys.0[0].1, f32::INFINITY);
        assert_eq!(fixture.say("keyup shift"), ["keyup: shift"]);
        assert_eq!(fixture.keys.0[0].1, 0.0);
        assert_eq!(fixture.say("keydown mouse1"), ["keydown: mouse1"]);
        assert_eq!(fixture.mouse.0.len(), 1);
        assert_eq!(fixture.say("keydown nosuchkey"), ["keydown: no key `nosuchkey`"]);
        assert_eq!(fixture.say("keyup"), ["usage: keyup <key>"]);
        assert_eq!(fixture.say("keydown"), ["usage: keydown <key>"]);
    }

    #[test]
    fn a_held_key_stays_down_through_a_focus_loss_and_comes_up_on_keyup() {
        let mut input = ButtonInput::<KeyCode>::default();
        let mut timed = vec![(KeyCode::ShiftLeft, f32::INFINITY, false)];
        advance_timed(0.1, &mut timed, &mut input);
        assert!(input.pressed(KeyCode::ShiftLeft));
        input.release_all();
        assert!(!input.pressed(KeyCode::ShiftLeft), "the window let go");
        advance_timed(10.0, &mut timed, &mut input);
        assert!(input.pressed(KeyCode::ShiftLeft), "put back down");
        set_time_left(&mut timed, KeyCode::ShiftLeft, 0.0, false);
        advance_timed(0.016, &mut timed, &mut input);
        assert!(!input.pressed(KeyCode::ShiftLeft));
        assert!(timed.is_empty());
    }

    #[test]
    fn mouse_motion_is_queued_once_or_kept_up() {
        let mut fixture = ScriptedSetup::default();
        assert_eq!(fixture.say("mousemove 3 4"), ["mousemove 3 4"]);
        assert_eq!(fixture.inputs.take_pointer_move(), (3.0, 4.0));
        assert_eq!(fixture.say("mousemove 5"), ["mousemove 5 0"]);
        assert_eq!(fixture.inputs.take_pointer_move(), (5.0, 0.0));
        assert_eq!(fixture.say("mouserate 1 2"), ["mouserate 1 2"]);
        assert_eq!(fixture.inputs.pointer_velocity(), Some((1.0, 2.0)));
        assert_eq!(fixture.say("mousemove x"), ["usage: mousemove <dx> [dy]"]);
        let expected = ["usage: mouserate <dx> [dy]"];
        assert_eq!(fixture.say("mouserate 1 2 3"), expected);
    }

    #[test]
    fn binds_are_cleared_restored_and_listed() {
        let mut fixture = ScriptedSetup::default();
        assert_eq!(fixture.say("binddefaults"), ["restored default binds"]);
        assert!(fixture.binds.iter().next().is_some());
        assert_eq!(fixture.say("unbindall"), ["unbindall"]);
        assert!(fixture.binds.iter().next().is_none());
        assert_eq!(fixture.say("bind"), ["no binds"]);
        fixture.binds.restore_stock_controls();
        assert!(fixture.say("bind").len() > 1);
    }

    #[test]
    fn the_console_toggles_and_takes_fed_text() {
        let mut fixture = ScriptedSetup::default();
        assert_eq!(fixture.say("con_toggle"), ["console open"]);
        assert!(fixture.console.shown);
        assert_eq!(fixture.say("con_toggle"), ["console closed"]);
        assert!(fixture.say("con_feed map mp_hijacked").is_empty());
        let expected = Some("map mp_hijacked");
        assert_eq!(fixture.console.pending_feed.as_deref(), expected);
        assert_eq!(fixture.say("con_feed"), ["usage: con_feed <text>"]);
        assert!(fixture.say("con_paste").is_empty());
        assert!(fixture.console.pending_os_paste);
    }

    #[test]
    fn a_timed_press_goes_down_at_once_and_up_when_its_time_is_out() {
        let mut input = ButtonInput::<KeyCode>::default();
        let mut timed = vec![(KeyCode::KeyW, 0.5, false)];
        advance_timed(0.1, &mut timed, &mut input);
        assert!(input.pressed(KeyCode::KeyW));
        advance_timed(0.4, &mut timed, &mut input);
        assert!(input.pressed(KeyCode::KeyW), "0.1 s are left");
        advance_timed(0.2, &mut timed, &mut input);
        assert!(!input.pressed(KeyCode::KeyW));
        assert!(timed.is_empty());
    }
}
