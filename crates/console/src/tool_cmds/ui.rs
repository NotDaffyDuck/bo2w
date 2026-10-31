//! `ui`: hide or show the game's interface. The console stays either way.

use ::ui::MenuPaintPass;
use bevy::{ecs::message::MessageReader, ecs::system::ResMut};

use super::echo::PromptReplyWriter;
use crate::TypedOrder;

pub(super) const HELP_BANNER: &[(&str, &str)] = &[(
    "ui",
    "ui [0|1] — hide or show the game interface; the console stays",
)];

/// What the arguments of `ui` ask for.
enum UiAsk {
    /// No arguments: say whether the menus are drawn.
    Show,
    /// Turn menu drawing on or off.
    Set(bool),
    /// Words `ui` does not take.
    Usage,
}

fn switch_of(given: &[String]) -> UiAsk {
    match given {
        [] => UiAsk::Show,
        [word] if word == "0" || word.eq_ignore_ascii_case("off") => UiAsk::Set(false),
        [word] if word == "1" || word.eq_ignore_ascii_case("on") => UiAsk::Set(true),
        _ => UiAsk::Usage,
    }
}

pub(crate) fn steer_menu_orders(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut draw: ResMut<MenuPaintPass>,
) {
    for command in commands.read().filter(|command| command.name == "ui") {
        match switch_of(&command.words) {
            UiAsk::Show => echo.write(format!("ui = {}", u8::from(draw.0))),
            UiAsk::Set(on) => {
                draw.0 = on;
                echo.write(format!("ui {}", u8::from(on)));
            }
            UiAsk::Usage => echo.write("usage: ui [0|1]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use crate::tool_cmds::ui::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
    use bevy::app::Update;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_words_for_off_and_on_are_understood_in_any_case() {
        for word in ["0", "off", "OFF"] {
            assert!(matches!(switch_of(&args(&[word])), UiAsk::Set(false)), "{word}");
        }
        for word in ["1", "on", "On"] {
            assert!(matches!(switch_of(&args(&[word])), UiAsk::Set(true)), "{word}");
        }
        assert!(matches!(switch_of(&[]), UiAsk::Show));
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        assert!(matches!(switch_of(&args(&["2"])), UiAsk::Usage));
        assert!(matches!(switch_of(&args(&["on", "off"])), UiAsk::Usage));
    }

    fn run(lines: &[&[&str]], start: bool) -> (bool, String) {
        let mut app = App::default();
        app.init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .insert_resource(MenuPaintPass(start))
            .add_message::<TypedOrder>()
            .add_systems(Update, steer_menu_orders);
        for typed in lines {
            app.world_mut().write_message(TypedOrder {
                name: "ui".into(),
                words: args(typed),
                as_written: String::new(),
                detached: false,
                typed_by_hand: false,
            });
        }
        app.update();
        let shown = app.world().resource::<MenuPaintPass>().0;
        (shown, app.world().resource::<PromptOutputRow>().0.clone())
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
        let mut app = App::default();
        app.init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .insert_resource(MenuPaintPass(true))
            .add_message::<TypedOrder>()
            .add_systems(Update, steer_menu_orders);
        app.world_mut().write_message(TypedOrder {
            name: "map".into(),
            words: args(&["0"]),
            as_written: String::new(),
            detached: false,
            typed_by_hand: false,
        });
        app.update();
        assert!(app.world().resource::<MenuPaintPass>().0);
        assert!(app.world().resource::<PromptOutputRow>().0.is_empty());
    }
}
