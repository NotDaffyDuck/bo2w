//! `screenshot`, and the commands that end the process: `quit`, `exit` and
//! `finish_run`.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use bevy::prelude::*;
use frame::LaunchIdentity;
use render::diag::capture::{CaptureQueue, CaptureRequest, exit_is_user_quit};

use super::echo::ConsoleEcho;
use crate::ConsoleCommand;

pub(super) const USAGE: &[(&str, &str)] = &[
    ("screenshot", "screenshot [name] — capture the backbuffer"),
    (
        "exit",
        "exit — quit the process, abandoning unfinished screenshots",
    ),
    (
        "quit",
        "quit — quit the process, abandoning unfinished screenshots",
    ),
    (
        "finish_run",
        "finish_run — finish the run's screenshots, then quit",
    ),
];

/// Where `screenshot [name]` writes: under `screenshots` in the artifacts
/// folder, named for the map when no name is given, with `.png` added when the
/// name has no extension.
fn screenshot_path(artifacts: &Path, zone: &str, name: Option<&str>) -> Result<PathBuf, String> {
    let name = Path::new(name.unwrap_or(zone));
    let stays_inside = name.file_name().is_some()
        && name
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if !stays_inside {
        return Err("name must be relative to bo2mp-artifacts/screenshots".into());
    }
    let path = artifacts.join("screenshots").join(name);
    match name.extension() {
        None => Ok(path.with_extension("png")),
        Some(extension) if extension.eq_ignore_ascii_case("png") => Ok(path),
        Some(_) => Err("name must use a .png extension".into()),
    }
}

/// Queues one screenshot, and says where it will be.
fn take_screenshot(
    args: &[String],
    identity: Option<&LaunchIdentity>,
    queue: Option<&mut CaptureQueue>,
) -> Result<String, String> {
    let name = match args {
        [] => None,
        [name] => Some(name.as_str()),
        _ => return Err("usage: screenshot [name]".into()),
    };
    let identity =
        identity.ok_or("screenshot: launch identity missing (artifacts path unknown)")?;
    let path = screenshot_path(&identity.artifacts, &identity.zone, name)
        .map_err(|why| format!("screenshot: {why}"))?;
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)
            .map_err(|why| format!("screenshot: create {}: {why}", folder.display()))?;
    }
    let queue = queue.ok_or("screenshot: render capture queue missing (no RenderPlugin)")?;
    queue.push(CaptureRequest {
        path: path.clone(),
        exit_after_capture: false,
    });
    Ok(format!(
        "screenshot: queued {} ({} waiting)",
        path.display(),
        queue.pending()
    ))
}

/// What `quit` says when it leaves screenshots undone.
fn quit_note(queue: Option<&CaptureQueue>) -> Option<String> {
    let (queued, writing) = queue.map_or((0, 0), CaptureQueue::owed_at_exit);
    (queued + writing > 0).then(|| {
        format!("quit: leaving {queued} queued and {writing} unfinished screenshot(s) behind")
    })
}

pub(crate) fn route_capture_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    identity: Option<Res<LaunchIdentity>>,
    mut queue: Option<ResMut<CaptureQueue>>,
    mut exit: MessageWriter<AppExit>,
) {
    for command in commands.read() {
        match command.name.as_str() {
            "screenshot" => {
                let result =
                    take_screenshot(&command.args, identity.as_deref(), queue.as_deref_mut());
                echo.write(match result {
                    Ok(done) | Err(done) => done,
                });
            }
            "exit" | "quit" => {
                exit_is_user_quit();
                diag::lifecycle_boundary("quit_requested", "");
                if let Some(note) = quit_note(queue.as_deref()) {
                    echo.write(note);
                }
                exit.write(AppExit::Success);
            }
            "finish_run" => {
                diag::lifecycle_boundary("quit_requested", " via=finish_run");
                if queue
                    .as_deref_mut()
                    .is_some_and(CaptureQueue::exit_after_drained)
                {
                    echo.write("finish_run: waiting for queued screenshots");
                } else {
                    exit.write(AppExit::Success);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::files::fresh_folder, *};
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    fn path_for(zone: &str, name: Option<&str>) -> Result<PathBuf, String> {
        screenshot_path(Path::new("art"), zone, name)
    }

    #[test]
    fn a_screenshot_is_named_for_the_map_unless_given_a_name() {
        let expected = Path::new("art").join("screenshots");
        assert_eq!(path_for("mp_raid", None), Ok(expected.join("mp_raid.png")));
        assert_eq!(
            path_for("mp_raid", Some("shot")),
            Ok(expected.join("shot.png"))
        );
        assert_eq!(
            path_for("mp_raid", Some("a/b.PNG")),
            Ok(expected.join("a").join("b.PNG"))
        );
        assert_eq!(
            path_for("mp_raid", Some("./c")),
            Ok(expected.join(".").join("c.png"))
        );
    }

    #[test]
    fn a_name_that_leaves_the_screenshots_folder_is_refused() {
        let outside = "name must be relative to bo2mp-artifacts/screenshots";
        for name in ["", ".", "..", "../x", "a/../../x", "/x"] {
            let answer = path_for("z", Some(name));
            assert_eq!(answer, Err(outside.to_owned()), "name {name:?}");
        }
        assert_eq!(
            path_for("", None),
            Err(outside.to_owned()),
            "no map, no name"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_is_outside_the_screenshots_folder() {
        for name in ["C:/x", "C:x"] {
            assert!(path_for("z", Some(name)).is_err(), "name {name:?}");
        }
    }

    #[test]
    fn only_png_is_a_screenshot() {
        assert_eq!(
            path_for("z", Some("x.jpg")),
            Err("name must use a .png extension".into())
        );
    }

    fn identity_in(folder: &Path) -> LaunchIdentity {
        LaunchIdentity {
            artifacts: folder.to_path_buf(),
            zone: "mp_raid".into(),
            ..LaunchIdentity::default()
        }
    }

    #[test]
    fn a_screenshot_is_queued_with_its_folder_made() {
        let folder = fresh_folder("capture-queued");
        let identity = identity_in(&folder);
        let mut queue = CaptureQueue::default();
        let args = vec!["shot".to_owned()];
        let said = take_screenshot(&args, Some(&identity), Some(&mut queue)).unwrap();
        let path = folder.join("screenshots").join("shot.png");
        assert_eq!(
            said,
            format!("screenshot: queued {} (1 waiting)", path.display())
        );
        assert!(folder.join("screenshots").is_dir());
        assert_eq!(queue.pending_paths(), [path]);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_screenshot_says_what_is_missing() {
        let folder = fresh_folder("capture-missing");
        let identity = identity_in(&folder);
        let mut queue = CaptureQueue::default();
        let two = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(
            take_screenshot(&two, Some(&identity), Some(&mut queue)),
            Err("usage: screenshot [name]".into())
        );
        assert_eq!(
            take_screenshot(&[], None, Some(&mut queue)),
            Err("screenshot: launch identity missing (artifacts path unknown)".into())
        );
        assert_eq!(
            take_screenshot(&[], Some(&identity), None),
            Err("screenshot: render capture queue missing (no RenderPlugin)".into())
        );
        let bad = vec!["../x".to_owned()];
        assert_eq!(
            take_screenshot(&bad, Some(&identity), Some(&mut queue)),
            Err("screenshot: name must be relative to bo2mp-artifacts/screenshots".into())
        );
        assert_eq!(queue.pending(), 0);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn quitting_mentions_screenshots_only_when_some_are_owed() {
        assert_eq!(quit_note(None), None);
        let mut queue = CaptureQueue::default();
        assert_eq!(quit_note(Some(&queue)), None);
        queue.push(CaptureRequest {
            path: "a.png".into(),
            exit_after_capture: false,
        });
        assert_eq!(
            quit_note(Some(&queue)).as_deref(),
            Some("quit: leaving 1 queued and 0 unfinished screenshot(s) behind")
        );
    }

    fn run(line: &str, queue: Option<CaptureQueue>) -> (App, usize) {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .add_message::<AppExit>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .add_systems(Update, route_capture_commands);
        if let Some(queue) = queue {
            app.insert_resource(queue);
        }
        for command in ConsoleCommand::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        let exits = app.world().resource::<Messages<AppExit>>().len();
        (app, exits)
    }

    #[test]
    fn quit_and_exit_end_the_process() {
        for line in ["quit", "exit"] {
            let (_, exits) = run(line, None);
            assert_eq!(exits, 1, "{line}");
        }
    }

    #[test]
    fn finish_run_waits_while_screenshots_are_owed() {
        let (app, exits) = run("finish_run", None);
        assert_eq!(exits, 1, "nothing owed: leave at once");
        assert!(app.world().resource::<ConsoleLine>().0.is_empty());

        let mut owed = CaptureQueue::default();
        owed.push(CaptureRequest {
            path: "a.png".into(),
            exit_after_capture: false,
        });
        let (app, exits) = run("finish_run", Some(owed));
        assert_eq!(exits, 0);
        assert_eq!(
            app.world().resource::<ConsoleLine>().0,
            "finish_run: waiting for queued screenshots"
        );
    }
}
