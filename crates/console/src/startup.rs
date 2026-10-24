//! Commands that arrive with the process: `--cmds`, `BO2MP_CMDS`, and stdin.

use std::{
    io::BufRead,
    sync::{
        Mutex, OnceLock,
        mpsc::{Receiver, SyncSender, sync_channel},
    },
};

use bevy::prelude::{App, Res, ResMut, Resource};

use crate::{ConsoleQueue, command::split_script};

/// The most stdin lines the console takes in one frame.
const STDIN_LINES_PER_FRAME: usize = 256;

/// The values of every `--cmds`, as `--cmds <script>` or `--cmds=<script>`,
/// and the arguments that are left.
fn split_cmds_flag(args: impl Iterator<Item = String>) -> (Vec<String>, Vec<String>) {
    let mut args = args;
    let (mut scripts, mut rest) = (Vec::new(), Vec::new());
    while let Some(arg) = args.next() {
        if arg == "--cmds" {
            scripts.extend(args.next());
        } else if let Some(inline) = arg.strip_prefix("--cmds=") {
            scripts.push(inline.to_owned());
        } else {
            rest.push(arg);
        }
    }
    (scripts, rest)
}

/// `args` without `--cmds` and what it carries.
pub fn strip_cmds_flag(args: impl Iterator<Item = String>) -> Vec<String> {
    split_cmds_flag(args).1
}

/// The commands the process was started with: those of `--cmds` in order,
/// then those of `env`, each script cut at the `;` outside quotes.
fn commands_from(args: impl Iterator<Item = String>, env: Option<String>) -> Vec<String> {
    let (mut scripts, _) = split_cmds_flag(args);
    scripts.extend(env);
    scripts
        .iter()
        .flat_map(|script| split_script(script))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The commands this process was started with.
pub fn startup_commands() -> Vec<String> {
    commands_from(std::env::args(), std::env::var("BO2MP_CMDS").ok())
}

/// Whether a script drives this run. Such a run never grabs the cursor and
/// ignores the real mouse, so someone using the PC while it runs neither steers
/// it nor loses their cursor to it. (Its window still takes focus: an
/// unfocused window's screenshots read back black.)
pub fn scripted_run() -> bool {
    static SCRIPTED: OnceLock<bool> = OnceLock::new();
    *SCRIPTED.get_or_init(|| !startup_commands().is_empty())
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
pub(crate) fn drain_stdin(input: Option<Res<StdinCommands>>, mut queue: ResMut<ConsoleQueue>) {
    let Some(input) = input else { return };
    let Ok(receiver) = input.0.lock() else { return };
    for line in receiver.try_iter().take(STDIN_LINES_PER_FRAME) {
        queue.push_line(&line);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use bevy::prelude::Update;

    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|arg| (*arg).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn the_flag_takes_the_next_argument_or_an_equals_value() {
        assert_eq!(
            commands_from(args(&["bo2mp", "--cmds", "map a; quit"]), None),
            ["map a", "quit"]
        );
        assert_eq!(
            commands_from(args(&["bo2mp", "--cmds=map a"]), None),
            ["map a"]
        );
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
        let mut app = App::new();
        app.init_resource::<ConsoleQueue>()
            .insert_resource(StdinCommands(Mutex::new(receiver)))
            .add_systems(Update, drain_stdin);
        app.update();
        let names: Vec<String> = app
            .world_mut()
            .resource_mut::<ConsoleQueue>()
            .drain()
            .into_iter()
            .map(|command| command.name)
            .collect();
        assert_eq!(names, ["map", "wait", "quit"]);
    }

    #[test]
    fn without_stdin_the_system_does_nothing() {
        let mut app = App::new();
        app.init_resource::<ConsoleQueue>()
            .add_systems(Update, drain_stdin);
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<ConsoleQueue>()
                .drain()
                .is_empty()
        );
    }
}
