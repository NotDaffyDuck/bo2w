//! `dump [name]`: one text file holding what the authority and the screen
//! know right now, for reading after the fact.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::{ecs::system::SystemParam, prelude::*};
use frame::LaunchIdentity;
use net::{AuthorityClock, AuthorityWorld, PresentedSnapshot};

use super::{echo::ConsoleEcho, files::write_new_file, hitvol::hitvol_report};
use crate::ConsoleCommand;

pub(super) const USAGE: &[(&str, &str)] = &[(
    "dump",
    "dump [name] - atomically write the current authority + presented state to bo2mp-artifacts/dumps/<timestamp>-<name>.txt (one shot; no history or timing)",
)];

const DEFAULT_NAME: &str = "snapshot";
const LONGEST_NAME: usize = 96;
const BAD_NAME: &str = "name must be 1-96 ASCII letters, digits, '.', '-' or '_'";

/// The part of a file name the user chose: `snapshot` when none, without a
/// trailing `.txt`, and nothing that could name another folder.
fn dump_name(args: &[String]) -> Result<String, String> {
    let name = match args {
        [] => DEFAULT_NAME,
        [name] => name.strip_suffix(".txt").unwrap_or(name),
        _ => return Err("usage: dump [name]".into()),
    };
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    let fits = (1..=LONGEST_NAME).contains(&name.len())
        && name.chars().all(allowed)
        && !matches!(name, "." | "..");
    if fits {
        Ok(name.to_owned())
    } else {
        Err(BAD_NAME.into())
    }
}

fn dump_path(artifacts: &Path, captured_unix_ns: u128, name: &str) -> PathBuf {
    artifacts
        .join("dumps")
        .join(format!("{captured_unix_ns}-{name}.txt"))
}

/// What the audio side holds, as the lines of the `[audio]` section.
fn audio_section(
    ready: Option<&audio::AudioReady>,
    decisions: Option<&audio::StartDecisions>,
    gaps: Option<&audio::MissingAliasGaps>,
    clips: Option<&audio::ClipStore>,
) -> String {
    let ready = ready.is_some_and(|ready| ready.0);
    let late = clips.map_or(0, audio::ClipStore::late_prepares);
    let mut out = format!("[audio]\nAudioReady = {ready}\nlate_prepares = {late}\n");
    match gaps.filter(|gaps| !gaps.is_empty()) {
        Some(gaps) => {
            out.push_str("missing_aliases =\n");
            for alias in &gaps.aliases {
                out.push_str(&format!("  {alias}\n"));
            }
        }
        None => out.push_str("missing_aliases = []\n"),
    }
    out.push_str("starts =\n");
    match decisions {
        Some(decisions) if !decisions.is_empty() => {
            for line in decisions.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        Some(_) => out.push_str("  (none)\n"),
        None => out.push_str("  Unavailable { reason: \"StartDecisions resource absent\" }\n"),
    }
    out
}

fn authority_section(clock: Option<&AuthorityClock>, authority: Option<&AuthorityWorld>) -> String {
    match (clock, authority) {
        (Some(clock), Some(world)) => format!("{:#?}", world.0.snapshot(sim::Tick(clock.tick))),
        (None, Some(_)) => "Unavailable { reason: \"AuthorityClock resource absent\" }".into(),
        (Some(_), None) => "Unavailable { reason: \"AuthorityWorld resource absent\" }".into(),
        (None, None) => {
            "Unavailable { reason: \"AuthorityClock and AuthorityWorld resources absent\" }".into()
        }
    }
}

fn hitvol_section(authority: Option<&AuthorityWorld>) -> String {
    let Some(authority) = authority else {
        return "[hitvol]\nUnavailable { reason: \"AuthorityWorld resource absent\" }\n".into();
    };
    let mut out = String::from("[hitvol]\n");
    for line in hitvol_report(&authority.0) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("rows =\n");
    for row in authority.0.hitvol_dump() {
        out.push_str(&format!("  {row:?}\n"));
    }
    out
}

/// The whole file. `audio` is the section from [`audio_section`], or `None`
/// for a dump taken somewhere that cannot see the audio side.
pub(super) fn state_dump_body(
    identity: &LaunchIdentity,
    captured_unix_ns: u128,
    clock: Option<&AuthorityClock>,
    authority: Option<&AuthorityWorld>,
    presented: Option<&PresentedSnapshot>,
    audio: Option<&str>,
) -> String {
    let presented = presented.map_or_else(
        || "Unavailable { reason: \"PresentedSnapshot resource absent\" }".to_owned(),
        |snapshot| format!("{snapshot:#?}"),
    );
    let audio =
        audio.unwrap_or("[audio]\nUnavailable { reason: \"not captured with this dump\" }\n");
    format!(
        "format = \"bo2mp-state-dump-1\"\n\
         captured_unix_ns = {captured_unix_ns}\n\
         role = {:?}\n\
         zone = {:?}\n\
         authority_clock = {clock:#?}\n\
         \n[authority_snapshot]\n{}\n\
         \n[presented_snapshot]\n{presented}\n\
         \n{}\n{audio}",
        identity.role_label,
        identity.zone,
        authority_section(clock, authority),
        hitvol_section(authority),
    )
}

/// What a dump reads from the authority and the screen.
#[derive(SystemParam)]
pub(crate) struct Observed<'w> {
    authority: Option<Res<'w, AuthorityWorld>>,
    clock: Option<Res<'w, AuthorityClock>>,
    presented: Option<Res<'w, PresentedSnapshot>>,
}

/// What a dump reads from the audio side.
#[derive(SystemParam)]
pub(crate) struct Heard<'w> {
    ready: Option<Res<'w, audio::AudioReady>>,
    decisions: Option<Res<'w, audio::StartDecisions>>,
    gaps: Option<Res<'w, audio::MissingAliasGaps>>,
    clips: Option<Res<'w, audio::ClipStore>>,
}

fn write_dump(
    args: &[String],
    identity: Option<&LaunchIdentity>,
    observed: &Observed,
    heard: &Heard,
) -> Result<String, String> {
    let name = dump_name(args)?;
    let identity = identity.ok_or("launch identity missing (artifacts path unknown)")?;
    let captured = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system clock is before UNIX epoch: {e}"))?
        .as_nanos();
    let audio = audio_section(
        heard.ready.as_deref(),
        heard.decisions.as_deref(),
        heard.gaps.as_deref(),
        heard.clips.as_deref(),
    );
    let body = state_dump_body(
        identity,
        captured,
        observed.clock.as_deref(),
        observed.authority.as_deref(),
        observed.presented.as_deref(),
        Some(&audio),
    );
    let path = dump_path(&identity.artifacts, captured, &name);
    write_new_file(&path, &body)?;
    Ok(path.display().to_string())
}

pub(crate) fn route_state_dump_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    identity: Option<Res<LaunchIdentity>>,
    observed: Observed,
    heard: Heard,
) {
    for command in commands.read().filter(|command| command.name == "dump") {
        echo.write(
            match write_dump(&command.args, identity.as_deref(), &observed, &heard) {
                Ok(path) => format!("dump: wrote {path}"),
                Err(why) => format!("dump: {why}"),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{super::files::fresh_folder, *};
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_dump_is_called_snapshot_unless_named() {
        assert_eq!(dump_name(&[]), Ok("snapshot".into()));
        assert_eq!(dump_name(&words(&["after-kill"])), Ok("after-kill".into()));
        assert_eq!(dump_name(&words(&["a.b_c-1.txt"])), Ok("a.b_c-1".into()));
    }

    #[test]
    fn a_name_that_is_not_a_plain_file_name_is_refused() {
        for name in [
            "",
            ".",
            "..",
            "a b",
            "a/b",
            "a\\b",
            "é",
            ".txt",
            &"x".repeat(97),
        ] {
            assert_eq!(dump_name(&words(&[name])), Err(BAD_NAME.into()), "{name:?}");
        }
        assert_eq!(dump_name(&words(&[&"x".repeat(96)])), Ok("x".repeat(96)));
        assert_eq!(
            dump_name(&words(&["a", "b"])),
            Err("usage: dump [name]".into())
        );
    }

    #[test]
    fn the_file_is_named_for_when_it_was_taken() {
        assert_eq!(
            dump_path(Path::new("art"), 42, "snapshot"),
            Path::new("art").join("dumps").join("42-snapshot.txt")
        );
    }

    fn identity() -> LaunchIdentity {
        LaunchIdentity {
            role_label: "host".into(),
            zone: "mp_raid".into(),
            ..LaunchIdentity::default()
        }
    }

    #[test]
    fn a_dump_with_nothing_to_show_says_what_is_missing() {
        let body = state_dump_body(&identity(), 7, None, None, None, None);
        let expected = "format = \"bo2mp-state-dump-1\"\n\
             captured_unix_ns = 7\n\
             role = \"host\"\n\
             zone = \"mp_raid\"\n\
             authority_clock = None\n\
             \n[authority_snapshot]\n\
             Unavailable { reason: \"AuthorityClock and AuthorityWorld resources absent\" }\n\
             \n[presented_snapshot]\n\
             Unavailable { reason: \"PresentedSnapshot resource absent\" }\n\
             \n[hitvol]\n\
             Unavailable { reason: \"AuthorityWorld resource absent\" }\n\
             \n[audio]\n\
             Unavailable { reason: \"not captured with this dump\" }\n";
        assert_eq!(body, expected);
    }

    #[test]
    fn the_clock_is_printed_when_there_is_one() {
        let clock = AuthorityClock {
            tick: 9,
            time_ms: 450,
        };
        let body = state_dump_body(&identity(), 7, Some(&clock), None, None, None);
        assert!(body.contains("authority_clock = Some(\n"), "{body}");
        assert!(body.contains("    tick: 9,"), "{body}");
        assert!(
            body.contains(
                "Unavailable { reason: \"AuthorityWorld resource absent\" }\n\n[presented"
            ),
            "{body}"
        );
    }

    #[test]
    fn the_audio_section_lists_gaps_and_starts() {
        let nothing = audio_section(None, None, None, None);
        assert_eq!(
            nothing,
            "[audio]\nAudioReady = false\nlate_prepares = 0\nmissing_aliases = []\n\
             starts =\n  Unavailable { reason: \"StartDecisions resource absent\" }\n"
        );

        let ready = audio::AudioReady(true);
        let empty = audio::StartDecisions::default();
        let mut gaps = audio::MissingAliasGaps::default();
        assert_eq!(
            audio_section(Some(&ready), Some(&empty), Some(&gaps), None),
            "[audio]\nAudioReady = true\nlate_prepares = 0\nmissing_aliases = []\n\
             starts =\n  (none)\n"
        );
        gaps.aliases = vec!["door_open".into(), "door_shut".into()];
        let listed = audio_section(None, Some(&empty), Some(&gaps), None);
        assert!(
            listed.contains("missing_aliases =\n  door_open\n  door_shut\nstarts =\n  (none)\n"),
            "{listed}"
        );
    }

    fn run(line: &str, artifacts: Option<&Path>) -> App {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .add_systems(Update, route_state_dump_commands);
        if let Some(artifacts) = artifacts {
            app.insert_resource(LaunchIdentity {
                artifacts: artifacts.to_path_buf(),
                ..identity()
            });
        }
        for command in ConsoleCommand::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn said(app: &App) -> String {
        app.world().resource::<ConsoleLine>().0.clone()
    }

    #[test]
    fn dump_writes_one_file_under_dumps() {
        let folder = fresh_folder("dump-writes");
        let app = run("dump first", Some(&folder));
        let said = said(&app);
        let path = said.strip_prefix("dump: wrote ").expect(&said);
        assert!(Path::new(path).starts_with(folder.join("dumps")), "{path}");
        assert!(path.ends_with("-first.txt"), "{path}");
        let body = fs::read_to_string(path).unwrap();
        assert!(body.starts_with("format = \"bo2mp-state-dump-1\"\n"));
        assert!(body.contains("[audio]\nAudioReady = false\n"), "{body}");
        assert_eq!(fs::read_dir(folder.join("dumps")).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn dump_says_what_it_could_not_do() {
        assert_eq!(
            said(&run("dump", None)),
            "dump: launch identity missing (artifacts path unknown)"
        );
        let folder = fresh_folder("dump-refuses");
        assert_eq!(
            said(&run("dump a/b", Some(&folder))),
            format!("dump: {BAD_NAME}")
        );
        assert_eq!(
            said(&run("dump a b", Some(&folder))),
            "dump: usage: dump [name]"
        );
        assert!(!folder.join("dumps").exists());
    }
}
