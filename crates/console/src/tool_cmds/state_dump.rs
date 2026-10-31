//! `dump [name]`: writes a single text file with the authority's and the
//! screen's current state, to be read afterwards.

use std::{path::Path, path::PathBuf, time::SystemTime, time::UNIX_EPOCH};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::StartupPersona;
use net::{HostTimeKeeper, HostSimulationWorld, ShownWorldFrame};

use super::echo::PromptReplyWriter;
use super::files::write_new_file;
use super::hitvol::hitvol_report;
use crate::TypedOrder;

pub(super) const HELP_BANNER: &[(&str, &str)] = &[(
    "dump",
    "dump [name] - write the current authority + presented state in one go to bo2mp-artifacts/dumps/<timestamp>-<name>.txt (one snapshot only, with no history or timing)",
)];

const DEFAULT_NAME: &str = "snapshot";
const LONGEST_NAME: usize = 96;
const BAD_NAME: &str = "name must be 1-96 ASCII letters, digits, '.', '-' or '_'";

/// User-chosen part of the file name: `snapshot` by default, minus any `.txt`
/// suffix, and never something that could point at another directory.
fn dump_name(args: &[String]) -> Result<String, String> {
    let chosen = match args {
        [] => DEFAULT_NAME,
        [given] => given.strip_suffix(".txt").unwrap_or(given),
        _ => return Err(String::from("usage: dump [name]")),
    };
    let plain_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_');
    let length_ok = (1..=LONGEST_NAME).contains(&chosen.len());
    if length_ok && chosen.chars().all(plain_char) && chosen != "." && chosen != ".." {
        return Ok(chosen.to_owned());
    }
    Err(BAD_NAME.into())
}

fn dump_path(folder: &Path, stamp: u128, label: &str) -> PathBuf {
    let file = format!("{stamp}-{label}.txt");
    folder.join("dumps").join(file)
}

/// The `[audio]` section: what the audio side currently holds.
fn audio_section(
    ready: Option<&audio::SoundSystemLive>,
    decisions: Option<&audio::PlaybackVerdictLog>,
    gaps: Option<&audio::UnknownCueHoles>,
    clips: Option<&audio::ClipRepository>,
) -> String {
    let indented = |lines: &mut dyn Iterator<Item = String>| lines.map(|line| format!("  {line}\n")).collect::<String>();
    let is_ready = ready.is_some_and(|flag| flag.0);
    let late = clips.map_or(0, |store| store.tardy_preloads());
    let missing = match gaps {
        Some(found) if !found.is_empty() => {
            format!("missing_aliases =\n{}", indented(&mut found.aliases.iter().map(ToString::to_string)))
        }
        _ => "missing_aliases = []\n".to_owned(),
    };
    let starts = match decisions {
        None => "  Unavailable { reason: \"StartDecisions resource absent\" }\n".to_owned(),
        Some(seen) if seen.is_empty() => "  (none)\n".to_owned(),
        Some(seen) => indented(&mut seen.lines().map(|line| line.to_string())),
    };
    format!("[audio]\nAudioReady = {is_ready}\nlate_prepares = {late}\n{missing}starts =\n{starts}")
}

/// What stands in for a section whose source is missing.
fn unavailable(reason: &str) -> String {
    format!("Unavailable {{ reason: \"{reason}\" }}")
}

fn authority_section(clock: Option<&HostTimeKeeper>, authority: Option<&HostSimulationWorld>) -> String {
    let pair = (authority, clock);
    if let (Some(held), Some(timer)) = pair {
        return format!("{:#?}", held.0.snapshot(sim::Tick(timer.tick)));
    }
    let missing = match (clock.is_none(), authority.is_none()) {
        (true, true) => "AuthorityClock and AuthorityWorld resources absent",
        (true, false) => "AuthorityClock resource absent",
        _ => "AuthorityWorld resource absent",
    };
    unavailable(missing)
}

fn hitvol_section(authority: Option<&HostSimulationWorld>) -> String {
    let mut text = String::from("[hitvol]\n");
    let Some(held) = authority else {
        text += &unavailable("AuthorityWorld resource absent");
        text.push_str("\n");
        return text;
    };
    let world = &held.0;
    let report = hitvol_report(world);
    let rows = world.hitvol_dump();
    report.iter().for_each(|entry| text.extend([entry.as_str(), "\n"]));
    text += "rows =\n";
    rows.iter().for_each(|row| text += &format!("  {row:?}\n"));
    text
}

/// Assembles the file contents. `sound` is the output of [`audio_section`],
/// or `None` when the dump was taken where the audio side is out of reach.
pub(super) fn state_dump_body(
    identity: &StartupPersona,
    recorded_epoch_nanos: u128,
    clock: Option<&HostTimeKeeper>,
    authority: Option<&HostSimulationWorld>,
    presented: Option<&ShownWorldFrame>,
    sound: Option<&str>,
) -> String {
    let header = [
        ("format", "\"bo2mp-state-dump-1\"".to_owned()),
        ("captured_unix_ns", recorded_epoch_nanos.to_string()),
        ("role", format!("{:?}", identity.duty_caption)),
        ("zone", format!("{:?}", identity.zone)),
        ("authority_clock", format!("{clock:#?}")),
    ];
    let screen = presented.map_or_else(|| unavailable("PresentedSnapshot resource absent"), |shown| format!("{shown:#?}"));
    let snapshots = [("authority_snapshot", authority_section(clock, authority)), ("presented_snapshot", screen)];

    let mut body = String::new();
    for (key, value) in header {
        body += &format!("{key} = {value}\n");
    }
    for (title, section) in snapshots {
        body += &format!("\n[{title}]\n{section}\n");
    }
    body.push('\n');
    body += &hitvol_section(authority);
    body.push('\n');
    match sound {
        Some(captured) => body += captured,
        None => {
            body += "[audio]\n";
            body += &unavailable("not captured with this dump");
            body.push('\n');
        }
    }
    body
}

/// Authority and screen state read by a dump.
#[derive(SystemParam)]
pub(crate) struct Observed<'w> {
    authority: Option<Res<'w, HostSimulationWorld>>,
    clock: Option<Res<'w, HostTimeKeeper>>,
    shown: Option<Res<'w, ShownWorldFrame>>,
}

/// Audio state read by a dump.
#[derive(SystemParam)]
pub(crate) struct Heard<'w> {
    ready: Option<Res<'w, audio::SoundSystemLive>>,
    decisions: Option<Res<'w, audio::PlaybackVerdictLog>>,
    gaps: Option<Res<'w, audio::UnknownCueHoles>>,
    clips: Option<Res<'w, audio::ClipRepository>>,
}

impl Heard<'_> {
    fn section(&self) -> String {
        audio_section(self.ready.as_deref(), self.decisions.as_deref(), self.gaps.as_deref(), self.clips.as_deref())
    }
}

/// Writes one dump; the path written, or why not.
fn write_dump(
    words: &[String],
    identity: Option<&StartupPersona>,
    observed: &Observed,
    heard: &Heard,
) -> Result<String, String> {
    let name = dump_name(words)?;
    let Some(launch) = identity else {
        return Err("launch identity missing (artifacts path unknown)".to_owned());
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let stamp = now.map_err(|why| format!("system clock is before UNIX epoch: {why}"))?.as_nanos();
    let (clock, authority, presented) =
        (observed.clock.as_deref(), observed.authority.as_deref(), observed.shown.as_deref());
    let audio_text = heard.section();
    let contents = state_dump_body(launch, stamp, clock, authority, presented, Some(audio_text.as_str()));
    let file = dump_path(&launch.artifacts, stamp, &name);
    write_new_file(&file, &contents).map(|()| file.display().to_string())
}

pub(crate) fn steer_snapshot_orders(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    identity: Option<Res<StartupPersona>>,
    observed: Observed,
    heard: Heard,
) {
    let dumps = commands.read().filter(|asked| asked.name == "dump");
    for asked in dumps {
        let outcome = write_dump(&asked.words, Option::as_deref(&identity), &observed, &heard);
        echo.write(outcome.map_or_else(|why| format!("dump: {why}"), |path| format!("dump: wrote {path}")));
    }
}
#[cfg(test)]
mod tests {

    use super::super::files::fresh_folder;
    use crate::tool_cmds::state_dump::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
use bevy::app::Update;

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
        for bad in [
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
            assert_eq!(dump_name(&words(&[bad])), Err(BAD_NAME.into()), "{bad:?}");
        }
        assert_eq!(dump_name(&words(&[&"x".repeat(96)])), Ok("x".repeat(96)));
        let want = Err("usage: dump [name]".into());
        assert_eq!(dump_name(&words(&["a", "b"])), want);
    }

    #[test]
    fn the_file_is_named_for_when_it_was_taken() {
        let actual = dump_path(Path::new("art"), 42, "snapshot");
        assert_eq!(actual, Path::new("art").join("dumps").join("42-snapshot.txt"));
    }

    fn identity() -> StartupPersona {
        StartupPersona {
            duty_caption: "host".into(),
            zone: "mp_raid".into(),
            ..StartupPersona::default()
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
        let clock = HostTimeKeeper {
            tick: 9,
            clock_stamp_ms: 450,
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
        let want = "[audio]\nAudioReady = false\nlate_prepares = 0\nmissing_aliases = []\n\
         starts =\n  Unavailable { reason: \"StartDecisions resource absent\" }\n";
        assert_eq!(nothing, want);

        let ready = audio::SoundSystemLive(true);
        let empty = audio::PlaybackVerdictLog::default();
        let mut gaps = audio::UnknownCueHoles::default();
        let actual = audio_section(Some(&ready), Some(&empty), Some(&gaps), None);
        assert_eq!(actual, "[audio]\nAudioReady = true\nlate_prepares = 0\nmissing_aliases = []\n\
         starts =\n  (none)\n");
        gaps.aliases = vec!["door_open".into(), "door_shut".into()];
        let listed = audio_section(None, Some(&empty), Some(&gaps), None);
        assert!(
            listed.contains("missing_aliases =\n  door_open\n  door_shut\nstarts =\n  (none)\n"),
            "{listed}"
        );
    }

    fn run(line: &str, artifacts: Option<&Path>) -> App {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .add_systems(Update, steer_snapshot_orders);
        if let Some(artifacts) = artifacts {
            app.insert_resource(StartupPersona {
                artifacts: artifacts.to_path_buf(),
                ..identity()
            });
        }
        for command in TypedOrder::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn said(app: &App) -> String {
        app.world().resource::<PromptOutputRow>().0.clone()
    }

    #[test]
    fn dump_writes_one_file_under_dumps() {
        let folder = fresh_folder("dump-writes");
        let app = run("dump first", Some(&folder));
        let said = said(&app);
        let path = said.strip_prefix("dump: wrote ").expect(&said);
        assert!(Path::new(path).starts_with(folder.join("dumps")), "{path}");
        assert!(path.ends_with("-first.txt"), "{path}");
        let body = std::fs::read_to_string(path).unwrap();
        assert!(body.starts_with("format = \"bo2mp-state-dump-1\"\n"));
        assert!(body.contains("[audio]\nAudioReady = false\n"), "{body}");
        assert_eq!(std::fs::read_dir(folder.join("dumps")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn dump_says_what_it_could_not_do() {
        let actual = said(&run("dump", None));
        assert_eq!(actual, "dump: launch identity missing (artifacts path unknown)");
        let folder = fresh_folder("dump-refuses");
        let want = format!("dump: {BAD_NAME}");
        assert_eq!(said(&run("dump a/b", Some(&folder))), want);
        let want = "dump: usage: dump [name]";
        assert_eq!(said(&run("dump a b", Some(&folder))), want);
        assert!(!folder.join("dumps").exists());
    }
}
