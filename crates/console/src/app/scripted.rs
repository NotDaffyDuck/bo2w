//! Input played on the player's behalf: inputs held or pressed from the
//! console, keys and mouse buttons held for a while, mouse motion, and the key
//! binds. The commands that ask for them are answered here.

use std::hash::Hash;

use bevy::{ecs::system::SystemParam, prelude::*};

use super::state::{ConsoleSettings, ConsoleState};
use crate::{
    BindButton, ConsoleCommand, ConsoleInputState, KeyBinds, PRESS_SECONDS, ScriptedKeys,
    binds::parse_button_name, is_bind_command,
};

/// The longest a timed press may last.
const LONGEST_PRESS_SECONDS: f32 = 60.0;

/// Mouse buttons held for a while by `key mouse1` and the like: the button,
/// the seconds left, and whether it has gone down yet.
#[derive(Resource, Default)]
pub(super) struct ScriptedMouse(pub(super) Vec<(MouseButton, f32, bool)>);

/// The keys and mouse buttons a script is holding down.
#[derive(SystemParam)]
pub(super) struct Timed<'w> {
    keys: Res<'w, ScriptedKeys>,
    mouse: Res<'w, ScriptedMouse>,
}

impl Timed<'_> {
    /// Whether a script, rather than the player, is holding `button`.
    pub(super) fn holds(&self, button: BindButton) -> bool {
        match button {
            BindButton::Key(code) => self.keys.0.iter().any(|(held, ..)| *held == code),
            BindButton::Mouse(held) => self.mouse.0.iter().any(|(mouse, ..)| *mouse == held),
            _ => false,
        }
    }
}

/// Presses what is new, and lets go of what has run out of time.
fn advance_timed<T>(dt: f32, timed: &mut Vec<(T, f32, bool)>, input: &mut ButtonInput<T>)
where
    T: Copy + Eq + Hash + Send + Sync + 'static,
{
    timed.retain_mut(|(button, left, started)| {
        if !*started {
            input.press(*button);
            *started = true;
            return true;
        }
        *left -= dt;
        if *left > 0.0 {
            return true;
        }
        input.release(*button);
        false
    });
}

pub(super) fn drive_scripted_keys(
    time: Res<Time>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut timed: ResMut<ScriptedKeys>,
) {
    advance_timed(time.delta_secs(), &mut timed.0, &mut *keys);
}

pub(super) fn drive_scripted_mouse(
    time: Res<Time>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut timed: ResMut<ScriptedMouse>,
) {
    advance_timed(time.delta_secs(), &mut timed.0, &mut *buttons);
}

pub(super) fn expire_pressed_inputs(time: Res<Time>, mut inputs: ResMut<ConsoleInputState>) {
    inputs.tick(time.delta_secs());
}

/// What an input command can change.
struct Targets<'a> {
    inputs: &'a mut ConsoleInputState,
    keys: &'a mut ScriptedKeys,
    mouse: &'a mut ScriptedMouse,
    binds: &'a mut KeyBinds,
    console: &'a mut ConsoleState,
}

fn is_console_input(word: &str) -> bool {
    is_bind_command(word)
}

fn one(line: impl Into<String>) -> Vec<String> {
    vec![line.into()]
}

/// `hold +actionslot 2` and `+actionslot 2` name a single input, "+actionslot
/// 2", that splitting on spaces has torn in two.
fn join_actionslot(mut command: ConsoleCommand) -> ConsoleCommand {
    let slot_word = |word: &str| matches!(word, "+actionslot" | "-actionslot");
    let takes_inputs = matches!(command.name.as_str(), "hold" | "press" | "release");
    if takes_inputs && command.args.len() >= 2 && slot_word(&command.args[0]) {
        let slot = command.args.remove(1);
        command.args[0].push(' ');
        command.args[0].push_str(&slot);
    } else if slot_word(&command.name) && command.args.len() == 1 {
        command.name.push(' ');
        command.name.push_str(&command.args[0]);
        command.args.clear();
    }
    command
}

/// A number of seconds, with or without a trailing `s`.
fn seconds_arg(arg: Option<&String>) -> f32 {
    arg.and_then(|text| text.trim_end_matches(['s', 'S']).parse::<f32>().ok())
        .filter(|seconds| seconds.is_finite())
        .unwrap_or(PRESS_SECONDS)
        .clamp(0.0, LONGEST_PRESS_SECONDS)
}

/// `<dx>` or `<dx> <dy>`.
fn pair(args: &[String]) -> Option<(f32, f32)> {
    match args {
        [dx] => Some((dx.parse().ok()?, 0.0)),
        [dx, dy] => Some((dx.parse().ok()?, dy.parse().ok()?)),
        _ => None,
    }
}

fn hold(inputs: &mut ConsoleInputState, args: &[String]) -> Vec<String> {
    if args.is_empty() || !args.iter().all(|input| is_console_input(input)) {
        return one("usage: hold <+input>...");
    }
    for input in args {
        inputs.hold(input);
    }
    one(format!("held: {}", args.join(" ")))
}

fn press(inputs: &mut ConsoleInputState, args: &[String]) -> Vec<String> {
    let count = args
        .iter()
        .take_while(|input| is_console_input(input))
        .count();
    let (pressed, rest) = args.split_at(count);
    if pressed.is_empty() {
        return one("usage: press <+input>... [seconds]");
    }
    let seconds = seconds_arg(rest.first());
    for input in pressed {
        inputs.press(input, seconds);
    }
    one(format!("pressed: {} ({seconds:.2}s)", pressed.join(" ")))
}

fn release(inputs: &mut ConsoleInputState, args: &[String]) -> Vec<String> {
    if let [only] = args
        && only == "all"
    {
        inputs.clear();
        return one("held inputs cleared");
    }
    if args.is_empty() || !args.iter().all(|input| is_console_input(input)) {
        return one("usage: release <+input...|all>");
    }
    for input in args {
        inputs.release(input);
    }
    one(format!("released: {}", args.join(" ")))
}

/// Holds a key, or a mouse button, for a while as if the player had.
fn key(targets: &mut Targets, args: &[String]) -> Vec<String> {
    let [name, rest @ ..] = args else {
        return one("usage: key <key> [seconds]");
    };
    let buttons: Vec<BindButton> = parse_button_name(name).into_iter().flatten().collect();
    let code = buttons.iter().find_map(|button| match button {
        BindButton::Key(code) => Some(*code),
        _ => None,
    });
    let mouse = buttons.iter().find_map(|button| match button {
        BindButton::Mouse(mouse) => Some(*mouse),
        _ => None,
    });
    let seconds = seconds_arg(rest.first());
    match (code, mouse) {
        (Some(code), _) => targets.keys.0.push((code, seconds, false)),
        (None, Some(mouse)) => targets.mouse.0.push((mouse, seconds, false)),
        (None, None) => return one(format!("key: no key `{name}`")),
    }
    one(format!("key: {name} ({seconds:.2}s)"))
}

fn mousemove(inputs: &mut ConsoleInputState, args: &[String]) -> Vec<String> {
    match pair(args) {
        Some((x, y)) => {
            inputs.queue_mouse(x, y);
            one(format!("mousemove {x} {y}"))
        }
        None => one("usage: mousemove <dx> [dy]"),
    }
}

fn mouserate(inputs: &mut ConsoleInputState, args: &[String]) -> Vec<String> {
    match pair(args) {
        Some((x, y)) => {
            inputs.set_mouse_rate(x, y);
            one(format!("mouserate {x} {y}"))
        }
        None => one("usage: mouserate <dx> [dy]"),
    }
}

fn bind(binds: &mut KeyBinds, args: &[String]) -> Vec<String> {
    if args.is_empty() {
        let lines = binds.list_lines();
        return if lines.is_empty() {
            one("no binds")
        } else {
            lines
        };
    }
    binds.apply_script(&format!("bind {}", args.join(" ")))
}

fn toggle_console(console: &mut ConsoleState) -> Vec<String> {
    console.open = !console.open;
    one(if console.open {
        "console open"
    } else {
        "console closed"
    })
}

fn feed_console(console: &mut ConsoleState, args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return one("usage: con_feed <text>");
    }
    console.pending_feed = Some(args.join(" "));
    Vec::new()
}

/// Carries out one input command and returns the lines it prints.
fn run(command: &ConsoleCommand, targets: &mut Targets) -> Vec<String> {
    let name = command.name.as_str();
    let args = command.args.as_slice();
    if name.starts_with('+') && is_console_input(name) {
        targets.inputs.hold(name);
        return Vec::new();
    }
    if let Some(action) = name.strip_prefix('-') {
        let plus = format!("+{action}");
        if is_console_input(&plus) {
            targets.inputs.release(&plus);
        }
        return Vec::new();
    }
    match name {
        "hold" => hold(targets.inputs, args),
        "press" => press(targets.inputs, args),
        "release" => release(targets.inputs, args),
        "key" => key(targets, args),
        "mousemove" => mousemove(targets.inputs, args),
        "mouserate" => mouserate(targets.inputs, args),
        "bind" => bind(targets.binds, args),
        "unbind" => targets
            .binds
            .apply_script(&format!("unbind {}", args.join(" "))),
        "unbindall" => {
            targets.binds.clear_all();
            one("unbindall")
        }
        "binddefaults" => {
            targets.binds.apply_defaults();
            one("restored default binds")
        }
        "con_toggle" => toggle_console(targets.console),
        "con_feed" => feed_console(targets.console, args),
        "con_paste" => {
            targets.console.pending_os_paste = true;
            Vec::new()
        }
        _ => Vec::new(),
    }
}

pub(super) fn handle_input_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut inputs: ResMut<ConsoleInputState>,
    mut keys: ResMut<ScriptedKeys>,
    mut mouse: ResMut<ScriptedMouse>,
    mut binds: ResMut<KeyBinds>,
    mut console: ResMut<ConsoleState>,
    settings: Res<ConsoleSettings>,
) {
    for command in commands.read() {
        let command = join_actionslot(command.clone());
        let lines = run(
            &command,
            &mut Targets {
                inputs: &mut inputs,
                keys: &mut keys,
                mouse: &mut mouse,
                binds: &mut binds,
                console: &mut console,
            },
        );
        for line in lines {
            console.echo(line, settings.log_capacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fixture {
        inputs: ConsoleInputState,
        keys: ScriptedKeys,
        mouse: ScriptedMouse,
        binds: KeyBinds,
        console: ConsoleState,
    }

    impl Fixture {
        fn say(&mut self, line: &str) -> Vec<String> {
            let mut words = line.split_whitespace();
            let command = ConsoleCommand {
                name: words.next().unwrap_or("").to_owned(),
                args: words.map(str::to_owned).collect(),
                raw: line.to_owned(),
                background: false,
                interactive: true,
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
        let mut fixture = Fixture::default();
        assert!(fixture.say("+forward").is_empty());
        assert!(fixture.inputs.held("+forward"));
        assert!(fixture.say("-forward").is_empty());
        assert!(!fixture.inputs.held("+forward"));
    }

    #[test]
    fn hold_takes_several_inputs_at_once() {
        let mut fixture = Fixture::default();
        assert_eq!(
            fixture.say("hold +forward +attack"),
            ["held: +forward +attack"]
        );
        assert!(fixture.inputs.held("+forward") && fixture.inputs.held("+attack"));
        assert_eq!(fixture.say("hold"), ["usage: hold <+input>..."]);
        assert_eq!(
            fixture.say("hold +forward nonsense"),
            ["usage: hold <+input>..."]
        );
    }

    #[test]
    fn press_holds_for_the_seconds_given_or_a_default() {
        let mut fixture = Fixture::default();
        assert_eq!(
            fixture.say("press +attack 0.5s"),
            ["pressed: +attack (0.50s)"]
        );
        assert_eq!(
            fixture.say("press +attack"),
            [format!("pressed: +attack ({PRESS_SECONDS:.2}s)")]
        );
        assert_eq!(
            fixture.say("press 2"),
            ["usage: press <+input>... [seconds]"]
        );
    }

    #[test]
    fn press_keeps_its_time_within_a_minute() {
        let mut fixture = Fixture::default();
        assert_eq!(
            fixture.say("press +attack 900"),
            ["pressed: +attack (60.00s)"]
        );
        assert_eq!(
            fixture.say("press +attack nan"),
            [format!("pressed: +attack ({PRESS_SECONDS:.2}s)")]
        );
    }

    #[test]
    fn release_lets_go_of_some_or_all() {
        let mut fixture = Fixture::default();
        fixture.say("hold +forward +attack");
        assert_eq!(fixture.say("release +forward"), ["released: +forward"]);
        assert!(!fixture.inputs.held("+forward") && fixture.inputs.held("+attack"));
        assert_eq!(fixture.say("release all"), ["held inputs cleared"]);
        assert!(!fixture.inputs.held("+attack"));
        assert_eq!(fixture.say("release"), ["usage: release <+input...|all>"]);
    }

    #[test]
    fn an_action_slot_is_one_input_though_it_has_a_space() {
        let command = |name: &str, args: &[&str]| ConsoleCommand {
            name: name.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            raw: String::new(),
            background: false,
            interactive: false,
        };
        let held = join_actionslot(command("hold", &["+actionslot", "2"]));
        assert_eq!(
            (held.name.as_str(), held.args.as_slice()),
            ("hold", ["+actionslot 2".to_owned()].as_slice())
        );
        let bare = join_actionslot(command("-actionslot", &["3"]));
        assert_eq!((bare.name.as_str(), bare.args.len()), ("-actionslot 3", 0));
        let other = join_actionslot(command("hold", &["+forward", "2"]));
        assert_eq!(other.args, ["+forward", "2"]);
    }

    #[test]
    fn key_holds_a_key_or_a_mouse_button() {
        let mut fixture = Fixture::default();
        assert_eq!(fixture.say("key w 0.5"), ["key: w (0.50s)"]);
        assert_eq!(fixture.keys.0, [(KeyCode::KeyW, 0.5, false)]);
        assert_eq!(
            fixture.say("key mouse1"),
            [format!("key: mouse1 ({PRESS_SECONDS:.2}s)")]
        );
        assert_eq!(fixture.mouse.0.len(), 1);
        assert_eq!(fixture.say("key nosuchkey"), ["key: no key `nosuchkey`"]);
        assert_eq!(fixture.say("key"), ["usage: key <key> [seconds]"]);
        assert_eq!(fixture.keys.0.len(), 1);
    }

    #[test]
    fn mouse_motion_is_queued_once_or_kept_up() {
        let mut fixture = Fixture::default();
        assert_eq!(fixture.say("mousemove 3 4"), ["mousemove 3 4"]);
        assert_eq!(fixture.inputs.take_mouse(), (3.0, 4.0));
        assert_eq!(fixture.say("mousemove 5"), ["mousemove 5 0"]);
        assert_eq!(fixture.inputs.take_mouse(), (5.0, 0.0));
        assert_eq!(fixture.say("mouserate 1 2"), ["mouserate 1 2"]);
        assert_eq!(fixture.inputs.mouse_rate(), Some((1.0, 2.0)));
        assert_eq!(fixture.say("mousemove x"), ["usage: mousemove <dx> [dy]"]);
        assert_eq!(
            fixture.say("mouserate 1 2 3"),
            ["usage: mouserate <dx> [dy]"]
        );
    }

    #[test]
    fn binds_are_cleared_restored_and_listed() {
        let mut fixture = Fixture::default();
        assert_eq!(fixture.say("binddefaults"), ["restored default binds"]);
        assert!(fixture.binds.iter().next().is_some());
        assert_eq!(fixture.say("unbindall"), ["unbindall"]);
        assert!(fixture.binds.iter().next().is_none());
        assert_eq!(fixture.say("bind"), ["no binds"]);
        fixture.binds.apply_defaults();
        assert!(fixture.say("bind").len() > 1);
    }

    #[test]
    fn the_console_toggles_and_takes_fed_text() {
        let mut fixture = Fixture::default();
        assert_eq!(fixture.say("con_toggle"), ["console open"]);
        assert!(fixture.console.open);
        assert_eq!(fixture.say("con_toggle"), ["console closed"]);
        assert!(fixture.say("con_feed map mp_hijacked").is_empty());
        assert_eq!(
            fixture.console.pending_feed.as_deref(),
            Some("map mp_hijacked")
        );
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
