//! One way for a command to answer: the console log, the status line and the
//! diagnostics log all get the same text.

use bevy::{ecs::system::SystemParam, prelude::*};

use crate::{ConsoleLine, ConsoleSettings, ConsoleState};

#[derive(SystemParam)]
pub(crate) struct ConsoleEcho<'w> {
    console: ResMut<'w, ConsoleState>,
    settings: Res<'w, ConsoleSettings>,
    line: ResMut<'w, ConsoleLine>,
}

impl ConsoleEcho<'_> {
    pub(crate) fn write(&mut self, text: impl Into<String>) {
        let text = text.into();
        diag::info!(Console, "{text}");
        self.console.echo(text.clone(), self.settings.log_capacity);
        self.line.0 = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn say(mut echo: ConsoleEcho) {
        echo.write("one");
        echo.write(String::from("two"));
    }

    #[test]
    fn a_line_reaches_the_log_and_the_status_line() {
        let mut app = App::new();
        app.init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .add_systems(Update, say);
        app.update();
        assert_eq!(app.world().resource::<ConsoleLine>().0, "two");
        let log = &app.world().resource::<ConsoleState>().log;
        assert_eq!(
            log.iter().map(String::as_str).collect::<Vec<_>>(),
            ["one", "two"]
        );
    }
}

/// What a command's tests share: an app that has the console's messages and
/// resources, and a way to feed it a line of script and read what was said.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::ConsoleCommand;

    pub(crate) fn console_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>();
        app
    }

    /// Runs one frame with every command in `script` waiting to be read.
    pub(crate) fn feed(app: &mut App, script: &str) {
        for command in ConsoleCommand::parse_script(script) {
            app.world_mut().write_message(command);
        }
        app.update();
    }

    /// Everything the console said so far, oldest first.
    pub(crate) fn said(app: &App) -> Vec<String> {
        app.world().resource::<ConsoleState>().log.to_vec()
    }
}
