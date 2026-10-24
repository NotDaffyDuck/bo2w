//! `visionSetNaked` and `visionReset`: blend the screen's film look to a
//! vision preset the map loaded, or back to the map's own.

use bevy::prelude::*;
use net::FrameClock;
use render_frontend::{
    assemble::drawsurf::film_vision_view::FilmVisionView,
    prepare::scene::{view_parms::PreparedSceneView, world::WorldScene},
};

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

const USAGE: &str = "usage: visionSetNaked <preset> [seconds] | visionReset [seconds]";
const DEFAULT_SECONDS: f64 = 1.0;

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    registry.register(
        CommandSpec::new("visionSetNaked")
            .usage("visionSetNaked <preset> [seconds] — apply a loaded vision preset"),
    );
    registry.register(
        CommandSpec::new("visionReset").usage("visionReset [seconds] — restore the map vision"),
    );
}

/// What was asked for: a named preset (or the map's own, with no name) and
/// how long to blend, in seconds.
struct Blend<'a> {
    preset: Option<&'a str>,
    seconds: f64,
}

impl<'a> Blend<'a> {
    fn parse(command: &'a str, args: &'a [String]) -> Result<Self, String> {
        let (preset, rest) = match (command, args) {
            ("visionReset", rest) => (None, rest),
            ("visionSetNaked", [name, rest @ ..]) => (Some(name.as_str()), rest),
            _ => return Err(USAGE.into()),
        };
        let seconds = match rest {
            [] => DEFAULT_SECONDS,
            [text] => text
                .parse()
                .map_err(|_| "vision: invalid transition duration".to_owned())?,
            _ => return Err(USAGE.into()),
        };
        if !seconds.is_finite() || seconds < 0.0 || seconds * 1000.0 > f64::from(i32::MAX) {
            return Err("vision: duration must be finite, nonnegative and fit milliseconds".into());
        }
        Ok(Self { preset, seconds })
    }

    fn millis(&self) -> i32 {
        (self.seconds * 1000.0).round() as i32
    }
}

/// How the map stores a preset: `vision/<name>.vision`, lower case, with the
/// folder and the extension optional in what is typed.
fn preset_key(typed: &str) -> String {
    let typed = typed.replace('\\', "/").to_ascii_lowercase();
    let stem = typed.strip_prefix("vision/").unwrap_or(&typed);
    let stem = stem.strip_suffix(".vision").unwrap_or(stem);
    format!("vision/{stem}.vision")
}

fn apply(
    command: &ConsoleCommand,
    scene: &WorldScene,
    view: &PreparedSceneView,
    time: i32,
    film: &mut FilmVisionView,
) -> Result<String, String> {
    if !view.ready {
        return Err("vision: no ready world view".into());
    }
    let blend = Blend::parse(&command.name, &command.args)?;
    let preset = match blend.preset {
        None => {
            if scene.film_vision.is_none() {
                return Err("vision: map preset is unavailable".into());
            }
            None
        }
        Some(typed) => {
            let key = preset_key(typed);
            match scene.film_visions.get(&key) {
                Some(Ok(vision)) => Some(*vision),
                Some(Err(error)) => return Err(format!("vision: {key}: {error:?}")),
                None => return Err(format!("vision: preset {key} is not loaded")),
            }
        }
    };
    film.select(scene.film_vision, preset, time, blend.millis());
    let shown = blend.preset.unwrap_or("map");
    Ok(format!("vision: {shown} transition={}s", blend.seconds))
}

pub(crate) fn route_vision_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    scene: Res<WorldScene>,
    view: Res<PreparedSceneView>,
    clock: Res<FrameClock>,
    mut film: ResMut<FilmVisionView>,
) {
    for command in commands
        .read()
        .filter(|command| matches!(command.name.as_str(), "visionSetNaked" | "visionReset"))
    {
        let said = apply(command, &scene, &view, clock.time(), &mut film).unwrap_or_else(|why| why);
        echo.write(said);
    }
}

#[cfg(test)]
mod tests {
    use asset_world::{FilmVision, FilmVisionParseError};

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn night() -> FilmVision {
        FilmVision {
            enable: true,
            contrast: 1.4,
            ..FilmVision::default()
        }
    }

    fn map_look() -> FilmVision {
        FilmVision {
            enable: true,
            brightness: 0.1,
            ..FilmVision::default()
        }
    }

    /// An app whose world view is ready and whose map has a look and the
    /// presets given.
    fn app_with(presets: &[(&str, Result<FilmVision, FilmVisionParseError>)]) -> App {
        let mut app = console_app();
        let mut scene = WorldScene {
            film_vision: Some(map_look()),
            ..WorldScene::default()
        };
        for (key, preset) in presets {
            scene.film_visions.insert((*key).to_owned(), *preset);
        }
        app.insert_resource(scene)
            .init_resource::<FrameClock>()
            .init_resource::<FilmVisionView>()
            .insert_resource(PreparedSceneView {
                ready: true,
                ..PreparedSceneView::default()
            })
            .add_systems(Update, route_vision_commands);
        app
    }

    fn film(app: &App) -> FilmVisionView {
        *app.world().resource::<FilmVisionView>()
    }

    #[test]
    fn a_loaded_preset_is_selected_under_any_spelling() {
        for typed in [
            "night",
            "NIGHT",
            "vision/night",
            "night.vision",
            "Vision\\Night.vision",
        ] {
            let mut app = app_with(&[("vision/night.vision", Ok(night()))]);
            let before = film(&app);
            feed(&mut app, &format!("visionSetNaked {typed}"));
            assert_eq!(
                said(&app),
                [format!("vision: {typed} transition=1s")],
                "{typed}"
            );
            assert_ne!(film(&app), before, "{typed} selected nothing");
        }
    }

    #[test]
    fn the_blend_length_is_kept_with_the_selection() {
        let mut quick = app_with(&[("vision/night.vision", Ok(night()))]);
        let mut slow = app_with(&[("vision/night.vision", Ok(night()))]);
        feed(&mut quick, "visionSetNaked night 0");
        feed(&mut slow, "visionSetNaked night 2.5");
        assert_eq!(said(&quick), ["vision: night transition=0s"]);
        assert_eq!(said(&slow), ["vision: night transition=2.5s"]);
        assert_ne!(film(&quick), film(&slow), "the blend length is kept");
    }

    #[test]
    fn a_reset_goes_back_to_the_map_and_needs_the_map_to_have_a_look() {
        let mut app = app_with(&[]);
        let before = film(&app);
        feed(&mut app, "visionReset 0.5");
        assert_eq!(said(&app), ["vision: map transition=0.5s"]);
        assert_ne!(film(&app), before);

        let mut bare = app_with(&[]);
        bare.world_mut().resource_mut::<WorldScene>().film_vision = None;
        feed(&mut bare, "visionReset");
        assert_eq!(said(&bare), ["vision: map preset is unavailable"]);
        assert_eq!(film(&bare), FilmVisionView::default());
    }

    #[test]
    fn a_preset_that_is_missing_or_broken_changes_nothing() {
        let mut app = app_with(&[("vision/bad.vision", Err(FilmVisionParseError::Decode))]);
        feed(&mut app, "visionSetNaked nowhere; visionSetNaked bad");
        assert_eq!(
            said(&app),
            [
                "vision: preset vision/nowhere.vision is not loaded",
                "vision: vision/bad.vision: Decode",
            ]
        );
        assert_eq!(film(&app), FilmVisionView::default());
    }

    #[test]
    fn without_a_ready_world_view_nothing_is_tried() {
        let mut app = app_with(&[("vision/night.vision", Ok(night()))]);
        app.world_mut().resource_mut::<PreparedSceneView>().ready = false;
        feed(&mut app, "visionSetNaked night; visionReset");
        assert_eq!(
            said(&app),
            ["vision: no ready world view", "vision: no ready world view"]
        );
        assert_eq!(film(&app), FilmVisionView::default());
    }

    #[test]
    fn the_blend_length_has_to_be_a_sane_number_of_seconds() {
        let mut app = app_with(&[("vision/night.vision", Ok(night()))]);
        for (line, said_so) in [
            (
                "visionSetNaked night soon",
                "vision: invalid transition duration",
            ),
            (
                "visionSetNaked night -1",
                "vision: duration must be finite, nonnegative and fit milliseconds",
            ),
            (
                "visionSetNaked night inf",
                "vision: duration must be finite, nonnegative and fit milliseconds",
            ),
            (
                "visionReset 3000000",
                "vision: duration must be finite, nonnegative and fit milliseconds",
            ),
            ("visionSetNaked", USAGE),
            ("visionSetNaked night 1 2", USAGE),
            ("visionReset 1 2", USAGE),
        ] {
            feed(&mut app, line);
            assert_eq!(
                said(&app).last().map(String::as_str),
                Some(said_so),
                "{line}"
            );
        }
        assert_eq!(film(&app), FilmVisionView::default());
    }

    #[test]
    fn the_longest_blend_that_fits_milliseconds_is_taken() {
        let mut app = app_with(&[]);
        feed(&mut app, "visionReset 2147483.647");
        assert_eq!(said(&app), ["vision: map transition=2147483.647s"]);
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app_with(&[]);
        feed(&mut app, "visionSet night; fog");
        assert_eq!(said(&app), Vec::<String>::new());
    }

    #[test]
    fn both_commands_are_registered_with_a_usage_line() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        let lines = registry.help_lines();
        for name in ["visionSetNaked", "visionReset"] {
            assert!(registry.resolve(name).is_some(), "{name}");
            assert!(lines.iter().any(|line| line.starts_with(name)), "{name}");
        }
    }

    #[test]
    fn a_key_is_the_folder_name_and_extension_in_lower_case() {
        assert_eq!(preset_key("Night"), "vision/night.vision");
        assert_eq!(preset_key("vision\\Night.VISION"), "vision/night.vision");
        assert_eq!(preset_key("a/b"), "vision/a/b.vision");
    }
}
