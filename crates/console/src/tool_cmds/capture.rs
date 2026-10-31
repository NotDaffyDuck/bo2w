//! `screenshot`, and the commands that end the process: `quit`, `exit` and
//! `finish_run`.

use std::{fs, path::Component, path::Path, path::PathBuf};

use bevy::{app::AppExit, ecs::message::MessageReader, ecs::message::MessageWriter, ecs::system::Res, ecs::system::ResMut};
use frame::StartupPersona;
use render::checks::capture::ScreenshotBacklog;
use render::checks::capture::ScreenshotOrder;
use render::checks::capture::exit_is_user_quit;

use super::echo::PromptReplyWriter;
use crate::TypedOrder;

/// The command that takes a picture.
const SHOT: &str = "screenshot";

pub(super) const HELP_BANNER: &[(&str, &str)] = &[
    (SHOT, "screenshot [name] — write the backbuffer out as a picture"),
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

fn shot_file(artifacts: &Path, zone: &str, given: Option<&str>) -> Result<PathBuf, String> {
    let name = Path::new(given.unwrap_or(zone));
    let plain = |part: Component<'_>| matches!(part, Component::Normal(_) | Component::CurDir);
    let stays_inside = name.file_name().is_some() && name.components().all(plain);
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
    words: &[String],
    identity: Option<&StartupPersona>,
    queue: Option<&mut ScreenshotBacklog>,
) -> Result<String, String> {
    let name = match words {
        [] => None,
        [name] => Some(name.as_str()),
        _ => return Err("usage: screenshot [name]".into()),
    };
    let launch =
        identity.ok_or("screenshot: launch identity missing (artifacts path unknown)")?;
    let path = shot_file(&launch.artifacts, &launch.zone, name)
        .map_err(|why| format!("screenshot: {why}"))?;
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)
            .map_err(|why| format!("screenshot: create {}: {why}", folder.display()))?;
    }
    let queue = queue.ok_or("screenshot: render capture queue missing (no RenderPlugin)")?;
    queue.push(ScreenshotOrder {
        path: PathBuf::clone(&path),
        quit_when_shot_taken: false,
    });
    let waiting = queue.pending();
    let shown = path.display();
    let answer = format!("screenshot: queued {shown} ({waiting} waiting)");
    Ok(answer)
}

/// What `quit` says when it leaves screenshots undone.
fn quit_note(queue: Option<&ScreenshotBacklog>) -> Option<String> {
    let (queued, writing) = queue.map_or((0, 0), ScreenshotBacklog::owed_at_exit);
    (queued + writing > 0).then(|| {
        format!("quit: leaving {queued} queued and {writing} unfinished screenshot(s) behind")
    })
}

pub(crate) fn steer_screenshot_orders(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    identity: Option<Res<StartupPersona>>,
    mut queue: Option<ResMut<ScreenshotBacklog>>,
    mut quit: MessageWriter<AppExit>,
) {
    for order in commands.read() {
        match order.name.as_str() {
            "screenshot" => echo.write(
                take_screenshot(&order.words, Option::as_deref(&identity), Option::as_deref_mut(&mut queue))
                    .unwrap_or_else(|refused| refused),
            ),
            "quit" | "exit" => {
                exit_is_user_quit();
                diag::lifecycle_boundary("quit_requested", "");
                if let Some(note) = quit_note(queue.as_deref()) {
                    echo.write(note);
                }
                quit.write(AppExit::Success);
            }
            "finish_run" => {
                diag::lifecycle_boundary("quit_requested", " via=finish_run");
                let waiting = queue.as_deref_mut().is_some_and(ScreenshotBacklog::finish_once_emptied);
                if waiting {
                    echo.write("finish_run: waiting for queued screenshots");
                } else {
                    quit.write(AppExit::Success);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::{app::App, ecs::message::Messages};
    use super::super::files::fresh_folder;
    use crate::tool_cmds::capture::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
    use bevy::app::Update;

    fn path_for(map: &str, given: Option<&str>) -> Result<PathBuf, String> {
        shot_file(Path::new("art"), map, given)
    }

    #[test]
    fn a_screenshot_is_named_for_the_map_unless_given_a_name() {
        let expected = Path::new("art").join("screenshots");
        assert_eq!(path_for("mp_raid", None), Ok(expected.join("mp_raid.png")));
        let actual = path_for("mp_raid", Some("shot"));
        assert_eq!(actual, Ok(expected.join("shot.png")));
        let actual = path_for("mp_raid", Some("a/b.PNG"));
        assert_eq!(actual, Ok(expected.join("a").join("b.PNG")));
        let actual = path_for("mp_raid", Some("./c"));
        assert_eq!(actual, Ok(expected.join(".").join("c.png")));
    }

    #[test]
    fn a_name_that_leaves_the_screenshots_folder_is_refused() {
        let outside = "name must be relative to bo2mp-artifacts/screenshots";
        for name in ["", ".", "..", "../x", "a/../../x", "/x"] {
            let answer = path_for("z", Some(name));
            assert_eq!(answer, Err(outside.to_owned()), "name {name:?}");
        }
        let want = Err(outside.to_owned());
        assert_eq!(path_for("", None), want, "no map, no name");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn a_drive_is_outside_the_screenshots_folder() {
        for name in ["C:/x", "C:x"] {
            assert!(path_for("z", Some(name)).is_err(), "name {name:?}");
        }
    }

    #[test]
    fn only_png_is_a_screenshot() {
        let want = Err("name must use a .png extension".into());
        assert_eq!(path_for("z", Some("x.jpg")), want);
    }

    fn identity_in(folder: &Path) -> StartupPersona {
        StartupPersona {
            artifacts: folder.to_path_buf(),
            zone: "mp_raid".into(),
            ..StartupPersona::default()
        }
    }

    #[test]
    fn a_screenshot_is_queued_with_its_folder_made() {
        let folder = fresh_folder("capture-queued");
        let identity = identity_in(&folder);
        let mut queue = ScreenshotBacklog::default();
        let args = vec!["shot".to_owned()];
        let said = take_screenshot(&args, Some(&identity), Some(&mut queue)).unwrap();
        let path = folder.join("screenshots").join("shot.png");
        let shown = path.display();
        let want = format!("screenshot: queued {shown} (1 waiting)");
        assert_eq!(said, want);
        assert!(folder.join("screenshots").is_dir());
        assert_eq!(queue.queued_file_paths(), [path]);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_screenshot_says_what_is_missing() {
        let folder = fresh_folder("capture-missing");
        let identity = identity_in(&folder);
        let mut queue = ScreenshotBacklog::default();
        let two = vec!["a".to_owned(), "b".to_owned()];
        let want = Err("usage: screenshot [name]".into());
        assert_eq!(take_screenshot(&two, Some(&identity), Some(&mut queue)), want);
        let actual = take_screenshot(&[], None, Some(&mut queue));
        assert_eq!(actual, Err("screenshot: launch identity missing (artifacts path unknown)".into()));
        let actual = take_screenshot(&[], Some(&identity), None);
        assert_eq!(actual, Err("screenshot: render capture queue missing (no RenderPlugin)".into()));
        let bad = vec!["../x".to_owned()];
        let want = Err("screenshot: name must be relative to bo2mp-artifacts/screenshots".into());
        assert_eq!(take_screenshot(&bad, Some(&identity), Some(&mut queue)), want);
        assert_eq!(queue.pending(), 0);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn quitting_mentions_screenshots_only_when_some_are_owed() {
        assert_eq!(quit_note(None), None);
        let mut queue = ScreenshotBacklog::default();
        assert_eq!(quit_note(Some(&queue)), None);
        queue.push(ScreenshotOrder {
            path: "a.png".into(),
            quit_when_shot_taken: false,
        });
        let expected = Some("quit: leaving 1 queued and 0 unfinished screenshot(s) behind");
        assert_eq!(quit_note(Some(&queue)).as_deref(), expected);
    }

    fn run(line: &str, queue: Option<ScreenshotBacklog>) -> (App, usize) {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .add_message::<AppExit>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .add_systems(Update, steer_screenshot_orders);
        if let Some(queue) = queue {
            app.insert_resource(queue);
        }
        for command in TypedOrder::parse_script(line) {
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
        assert!(app.world().resource::<PromptOutputRow>().0.is_empty());

        let mut owed = ScreenshotBacklog::default();
        owed.push(ScreenshotOrder {
            path: "a.png".into(),
            quit_when_shot_taken: false,
        });
        let (app, exits) = run("finish_run", Some(owed));
        assert_eq!(exits, 0);
        let want = "finish_run: waiting for queued screenshots";
        assert_eq!(app.world().resource::<PromptOutputRow>().0, want);
    }
}
