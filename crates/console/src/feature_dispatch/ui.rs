//! `ui`: hide or show the game's interface. The console stays either way.

use ::ui::UiDraw;
use bevy::prelude::*;

use super::echo::ConsoleEcho;
use crate::ConsoleCommand;

pub(super) const USAGE: &[(&str, &str)] = &[(
    "ui",
    "ui [0|1] — hide or show the game interface; the console stays",
)];

/// What the arguments of `ui` ask for: `None` is a question, `Some` a switch.
fn switch_of(args: &[String]) -> Result<Option<bool>, &'static str> {
    match args {
        [] => Ok(None),
        [word] if word == "0" || word.eq_ignore_ascii_case("off") => Ok(Some(false)),
        [word] if word == "1" || word.eq_ignore_ascii_case("on") => Ok(Some(true)),
        _ => Err("usage: ui [0|1]"),
    }
}

pub(crate) fn route_ui_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut draw: ResMut<UiDraw>,
) {
    for command in commands.read().filter(|command| command.name == "ui") {
        match switch_of(&command.args) {
            Ok(None) => echo.write(format!("ui = {}", u8::from(draw.0))),
            Ok(Some(on)) => {
                draw.0 = on;
                echo.write(format!("ui {}", u8::from(on)));
            }
            Err(usage) => echo.write(usage),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_words_for_off_and_on_are_understood_in_any_case() {
        for word in ["0", "off", "OFF"] {
            assert_eq!(switch_of(&args(&[word])), Ok(Some(false)), "{word}");
        }
        for word in ["1", "on", "On"] {
            assert_eq!(switch_of(&args(&[word])), Ok(Some(true)), "{word}");
        }
        assert_eq!(switch_of(&[]), Ok(None));
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        assert!(switch_of(&args(&["2"])).is_err());
        assert!(switch_of(&args(&["on", "off"])).is_err());
    }

    fn run(lines: &[&[&str]], start: bool) -> (bool, String) {
        let mut app = App::new();
        app.init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .insert_resource(UiDraw(start))
            .add_message::<ConsoleCommand>()
            .add_systems(Update, route_ui_commands);
        for line in lines {
            app.world_mut().write_message(ConsoleCommand {
                name: "ui".into(),
                args: args(line),
                raw: String::new(),
                background: false,
                interactive: false,
            });
        }
        app.update();
        let shown = app.world().resource::<UiDraw>().0;
        (shown, app.world().resource::<ConsoleLine>().0.clone())
    }

    #[test]
    fn the_command_switches_the_interface_and_says_so() {
        assert_eq!(run(&[&["0"]], true), (false, "ui 0".into()));
        assert_eq!(run(&[&["1"]], false), (true, "ui 1".into()));
        assert_eq!(run(&[&[]], true), (true, "ui = 1".into()));
        assert_eq!(run(&[&["maybe"]], true), (true, "usage: ui [0|1]".into()));
    }

    #[test]
    fn commands_for_other_features_are_left_alone() {
        let mut app = App::new();
        app.init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .insert_resource(UiDraw(true))
            .add_message::<ConsoleCommand>()
            .add_systems(Update, route_ui_commands);
        app.world_mut().write_message(ConsoleCommand {
            name: "map".into(),
            args: args(&["0"]),
            raw: String::new(),
            background: false,
            interactive: false,
        });
        app.update();
        assert!(app.world().resource::<UiDraw>().0);
        assert!(app.world().resource::<ConsoleLine>().0.is_empty());
    }
}
