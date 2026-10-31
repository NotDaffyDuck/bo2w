//! `visionSetNaked` and `visionReset`: blend the screen's film look to a
//! vision preset the map loaded, or back to the map's own.

use bevy::{ecs::message::MessageReader, ecs::system::Res, ecs::system::ResMut};
use net::FrameClock;
use render_frontend::assemble::draw_items::screen_grade_view::GradeEye;
use render_frontend::prepare::scene::eye_parms::StagedStageEye;
use render_frontend::prepare::scene::world::LevelStage;

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

const HELP_BANNER: &str = "usage: visionSetNaked <preset> [seconds] | visionReset [seconds]";
const DEFAULT_SECONDS: f64 = 1.0;

pub(crate) fn enlist(book: &mut OrderCatalog) {
    let naked = OrderBlueprint::new("visionSetNaked")
        .help_line("visionSetNaked <preset> [seconds] — apply a loaded vision preset");
    let reset = OrderBlueprint::new("visionReset").help_line("visionReset [seconds] — restore the map vision");
    book.enlist(naked);
    book.enlist(reset);
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
            _ => return Err(HELP_BANNER.into()),
        };
        let seconds = match rest {
            [] => DEFAULT_SECONDS,
            [text] => text
                .parse()
                .map_err(|_| "vision: invalid transition duration".to_owned())?,
            _ => return Err(HELP_BANNER.into()),
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
    let typed = typed.to_ascii_lowercase().replace('\\', "/");
    let stem = typed.strip_prefix("vision/").unwrap_or(&typed);
    let stem = stem.strip_suffix(".vision").unwrap_or(stem);
    format!("vision/{stem}.vision")
}

fn apply(
    command: &TypedOrder,
    scene: &LevelStage,
    view: &StagedStageEye,
    time: i32,
    film: &mut GradeEye,
) -> Result<String, String> {
    if !view.primed {
        return Err("vision: no ready world view".into());
    }
    let blend = Blend::parse(&command.name, &command.words)?;
    let preset = match blend.preset {
        None => {
            if scene.colour_grade.is_none() {
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
    film.select(scene.colour_grade, preset, time, blend.millis());
    let shown = blend.preset.unwrap_or("map");
    Ok(format!("vision: {shown} transition={}s", blend.seconds))
}

pub(crate) fn route_vision_commands(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    scene: Res<LevelStage>,
    view: Res<StagedStageEye>,
    clock: Res<FrameClock>,
    mut film: ResMut<GradeEye>,
) {
    for command in commands
        .read()
        .filter(|command| matches!(command.name.as_str(), "visionSetNaked" | "visionReset"))
    {
        let now = clock.time();
        let said = apply(command, &scene, &view, now, &mut film).unwrap_or_else(|why| why);
        echo.write(said);
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use asset_world::{ScreenGradeLook, GradeVisionDecodeFault};

    use crate::vision::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use bevy::app::Update;

    fn night() -> ScreenGradeLook {
        let mut look = ScreenGradeLook::default();
        look.enable = true;
        look.contrast = 1.4;
        look
    }

    fn map_look() -> ScreenGradeLook {
        let mut look = ScreenGradeLook::default();
        look.enable = true;
        look.brightness = 0.1;
        look
    }

    /// An app whose world view is ready and whose map has a look and the
    /// presets given.
    fn app_with(presets: &[(&str, Result<ScreenGradeLook, GradeVisionDecodeFault>)]) -> App {
        let mut app = console_app();
        let mut scene = LevelStage {
            colour_grade: Some(map_look()),
            ..LevelStage::default()
        };
        for (key, preset) in presets {
            scene.film_visions.insert((*key).to_owned(), *preset);
        }
        let mut eye = StagedStageEye::default();
        eye.primed |= true;
        app.insert_resource(scene)
            .init_resource::<FrameClock>()
            .init_resource::<GradeEye>()
            .insert_resource(eye)
            .add_systems(Update, route_vision_commands);
        app
    }

    fn film(app: &App) -> GradeEye {
        *app.world().resource::<GradeEye>()
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
            let expected = [format!("vision: {typed} transition=1s")];
            assert_eq!(said(&app), expected, "{typed}");
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
        bare.world_mut().resource_mut::<LevelStage>().colour_grade = None;
        feed(&mut bare, "visionReset");
        assert_eq!(said(&bare), ["vision: map preset is unavailable"]);
        assert_eq!(film(&bare), GradeEye::default());
    }

    #[test]
    fn a_preset_that_is_missing_or_broken_changes_nothing() {
        let mut app = app_with(&[("vision/bad.vision", Err(GradeVisionDecodeFault::Decode))]);
        feed(&mut app, "visionSetNaked nowhere; visionSetNaked bad");
        let expected = [
            "vision: preset vision/nowhere.vision is not loaded",
            "vision: vision/bad.vision: Decode",
        ];
        assert_eq!(said(&app), expected);
        assert_eq!(film(&app), GradeEye::default());
    }

    #[test]
    fn without_a_ready_world_view_nothing_is_tried() {
        let mut app = app_with(&[("vision/night.vision", Ok(night()))]);
        app.world_mut().resource_mut::<StagedStageEye>().primed = false;
        feed(&mut app, "visionSetNaked night; visionReset");
        let expected = ["vision: no ready world view", "vision: no ready world view"];
        assert_eq!(said(&app), expected);
        assert_eq!(film(&app), GradeEye::default());
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
            ("visionSetNaked", HELP_BANNER),
            ("visionSetNaked night 1 2", HELP_BANNER),
            ("visionReset 1 2", HELP_BANNER),
        ] {
            feed(&mut app, line);
            let expected = Some(said_so);
            assert_eq!(said(&app).last().map(String::as_str), expected, "{line}");
        }
        assert_eq!(film(&app), GradeEye::default());
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
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        let lines = book.usage_listing();
        for name in ["visionSetNaked", "visionReset"] {
            assert!(book.look_up(name).is_some(), "{name}");
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
