//! Commands that arrive with the process: `--cmds`, `BO2MP_CMDS`, and stdin.

use std::{io::BufRead, sync::mpsc::Receiver, sync::mpsc::sync_channel, sync::mpsc::SyncSender};
use std::{sync::Mutex, sync::OnceLock};

use bevy::{prelude::App, prelude::Res, prelude::ResMut, prelude::Resource};

use crate::OrderInbox;
use crate::command::split_script;

/// The most stdin lines the console takes in one frame.
const STDIN_LINES_PER_FRAME: usize = 256;

/// The arguments split into the values of every `--cmds` and the rest.
struct CmdsSplit {
    /// Each `--cmds <script>` or `--cmds=<script>`, in order.
    scripts: Vec<String>,
    /// The arguments that are left.
    rest: Vec<String>,
}

fn split_cmds_flag(args: impl Iterator<Item = String>) -> CmdsSplit {
    let mut args = args;
    let mut scripts: Vec<String> = vec![];
    let mut rest: Vec<String> = vec![];
    while let Some(arg) = args.next() {
        if arg == "--cmds" {
            scripts.extend(args.next());
        } else if let Some(inline) = arg.strip_prefix("--cmds=") {
            scripts.push(inline.to_owned());
        } else {
            rest.push(arg);
        }
    }
    CmdsSplit { scripts, rest }
}

/// `argv` without `--cmds` and what it carries.
pub fn strip_cmds_flag(argv: impl Iterator<Item = String>) -> Vec<String> {
    split_cmds_flag(argv).rest
}

/// The commands the process was started with: those of `--cmds` in order,
/// then those of `from_env`, each script cut at the `;` outside quotes.
fn commands_from(args: impl Iterator<Item = String>, from_env: Option<String>) -> Vec<String> {
    let mut scripts = split_cmds_flag(args).scripts;
    scripts.extend(from_env);
    scripts
        .iter()
        .flat_map(|script| split_script(script))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<String>>()
}

/// `BO2MP_INPUT_SCRIPT`'s file as a script: a command a line, `#` lines left
/// out. The commands are the console's own, so `wait spawn`, `wait 2s`,
/// `keydown shift` and `keyup shift` time the keys a run presses.
fn input_script(file: &str) -> String {
    file.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(";")
}

/// `env`'s script followed by the input script's.
fn with_input_script(env: Option<String>, file: Option<String>) -> Option<String> {
    match (env, file.map(|text| input_script(&text))) {
        (Some(env), Some(file)) => Some(format!("{env};{file}")),
        (env, file) => env.or(file),
    }
}

/// The commands this process was started with: `--cmds`, `BO2MP_CMDS`, then
/// the commands in the file `BO2MP_INPUT_SCRIPT` names.
pub fn boot_orders() -> Vec<String> {
    let file = std::env::var_os("BO2MP_INPUT_SCRIPT").and_then(|path| {
        std::fs::read_to_string(&path)
            .map_err(|err| eprintln!("BO2MP_INPUT_SCRIPT {path:?}: {err}"))
            .ok()
    });
    commands_from(
        std::env::args(),
        with_input_script(std::env::var("BO2MP_CMDS").ok(), file),
    )
}

/// Whether a script drives this run. Such a run never grabs the cursor and
/// ignores the real mouse, so someone using the PC while it runs neither steers
/// it nor loses their cursor to it. (Its window still takes focus: an
/// unfocused window's screenshots read back black.)
pub fn scripted_run() -> bool {
    static SCRIPTED: OnceLock<bool> = OnceLock::new();
    *SCRIPTED.get_or_init(|| !boot_orders().is_empty())
}

/// Lines read from stdin, waiting for the console to take them.
#[derive(Resource)]
pub(crate) struct StdinCommands(Mutex<Receiver<String>>);

/// Sends each line `reader` yields, until it ends, fails, or nobody listens.
fn forward_lines(reader: impl BufRead, sender: &SyncSender<String>) {
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if sender.send(line).is_err() {
            return;
        }
    }
}

/// Reads stdin on a thread of its own when `BO2MP_CONSOLE_STDIN` is `1`.
pub(crate) fn install_stdin(app: &mut App) {
    if std::env::var("BO2MP_CONSOLE_STDIN").as_deref() != Ok("1") {
        return;
    }
    let (sender, receiver) = sync_channel(STDIN_LINES_PER_FRAME);
    std::thread::spawn(move || forward_lines(std::io::stdin().lock(), &sender));
    app.insert_resource(StdinCommands(Mutex::new(receiver)));
}

/// Hands the stdin lines that have arrived to the console.
pub(crate) fn drain_stdin(input: Option<Res<StdinCommands>>, mut queue: ResMut<OrderInbox>) {
    let Some(input) = input else { return };
    let Ok(receiver) = input.0.lock() else { return };
    for line in receiver.try_iter().take(STDIN_LINES_PER_FRAME) {
        queue.enqueue_typed_text(&line);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::startup::*;
    use bevy::app::Update;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|arg| (*arg).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn the_flag_takes_the_next_argument_or_an_equals_value() {
        let expected = ["map a", "quit"];
        assert_eq!(commands_from(args(&["bo2mp", "--cmds", "map a; quit"]), None), expected);
        let expected = ["map a"];
        assert_eq!(commands_from(args(&["bo2mp", "--cmds=map a"]), None), expected);
    }

    #[test]
    fn every_flag_counts_and_the_environment_comes_last() {
        let lines = commands_from(
            args(&["bo2mp", "--cmds", "a", "--cmds=b"]),
            Some("c;d".to_owned()),
        );
        assert_eq!(lines, ["a", "b", "c", "d"]);
    }

    #[test]
    fn the_input_script_follows_the_other_commands_a_line_each() {
        let file = "# hold shift while running\nwait spawn\n\n  keydown shift \nwait 2s\nkeyup shift\n";
        let expected = "wait spawn;keydown shift;wait 2s;keyup shift";
        assert_eq!(input_script(file), expected);
        let lines = commands_from(
            args(&["bo2mp", "--cmds", "map a"]),
            with_input_script(Some("hold +forward".to_owned()), Some(file.to_owned())),
        );
        let waited = "wait spawn";
        let expected = [
            "map a",
            "hold +forward",
            waited,
            "keydown shift",
            "wait 2s",
            "keyup shift"
        ];
        assert_eq!(lines, expected);
        assert_eq!(with_input_script(None, None), None);
        let expected = Some("quit");
        assert_eq!(with_input_script(None, Some("quit".to_owned())).as_deref(), expected);
    }

    #[test]
    fn no_flag_and_no_environment_is_no_commands() {
        assert!(commands_from(args(&["bo2mp", "--windowed"]), None).is_empty());
        assert!(commands_from(args(&["bo2mp", "--cmds"]), None).is_empty());
    }

    #[test]
    fn a_script_is_cut_at_semicolons_outside_quotes_and_trimmed() {
        let lines = commands_from(args(&["--cmds", r#" say "a;b" ;; ; quit "#]), None);
        assert_eq!(lines, [r#"say "a;b""#, "quit"]);
    }

    #[test]
    fn stripping_removes_the_flag_with_its_value_and_nothing_else() {
        let kept = strip_cmds_flag(args(&[
            "bo2mp", "--cmds", "map a", "--w", "--cmds=b", "--cmds",
        ]));
        assert_eq!(kept, ["bo2mp", "--w"]);
        let kept = strip_cmds_flag(args(&["bo2mp", "map a"]));
        assert_eq!(kept, ["bo2mp", "map a"], "a value alone is not the flag's");
    }

    #[test]
    fn lines_are_forwarded_until_the_reader_ends() {
        let (sender, receiver) = sync_channel(8);
        forward_lines(Cursor::new("map a\nquit\n"), &sender);
        assert_eq!(receiver.try_iter().collect::<Vec<_>>(), ["map a", "quit"]);
    }

    #[test]
    fn forwarding_stops_when_nobody_is_listening() {
        let (sender, receiver) = sync_channel(8);
        drop(receiver);
        forward_lines(Cursor::new("a\nb\n"), &sender);
    }

    #[test]
    fn the_console_takes_the_lines_that_have_arrived() {
        let (sender, receiver) = sync_channel(8);
        sender.send("map a; wait 1".to_owned()).unwrap();
        sender.send("quit".to_owned()).unwrap();
        let mut app = App::default();
        app.init_resource::<OrderInbox>()
            .insert_resource(StdinCommands(Mutex::new(receiver)))
            .add_systems(Update, drain_stdin);
        app.update();
        let names: Vec<String> = app
            .world_mut()
            .resource_mut::<OrderInbox>()
            .drain()
            .into_iter()
            .map(|command| command.name)
            .collect::<Vec<String>>();
        assert_eq!(names, ["map", "wait", "quit"]);
    }

    #[test]
    fn without_stdin_the_system_does_nothing() {
        let mut app = App::default();
        app.init_resource::<OrderInbox>()
            .add_systems(Update, drain_stdin);
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<OrderInbox>()
                .drain()
                .is_empty()
        );
    }
}
