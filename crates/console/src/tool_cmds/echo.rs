//! One way for a command to answer: the console log, the status line and the
//! diagnostics log all get the same text.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::PromptOutputRow;
use crate::PromptPanelOptions;
use crate::PromptPanelStatus;

#[derive(SystemParam)]
pub(crate) struct PromptReplyWriter<'w> {
    panel: ResMut<'w, PromptPanelStatus>,
    prefs: Res<'w, PromptPanelOptions>,
    status_line: ResMut<'w, PromptOutputRow>,
}

impl PromptReplyWriter<'_> {
    pub(crate) fn write(&mut self, text: impl Into<String>) {
        let text = text.into();
        diag::info!(Console, "{}", text);
        self.panel.post_line(text.clone(), self.prefs.backlog_limit);
        self.status_line.0 = text;
    }
}

#[cfg(test)]
mod tests {
    use crate::tool_cmds::echo::*;
    use bevy::app::Update;

    fn say(mut echo: PromptReplyWriter) {
        echo.write("one");
        echo.write(String::from("two"));
    }

    #[test]
    fn a_line_reaches_the_log_and_the_status_line() {
        let mut app = App::default();
        app.init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .add_systems(Update, say);
        app.update();
        assert_eq!(app.world().resource::<PromptOutputRow>().0, "two");
        let log = &app.world().resource::<PromptPanelStatus>().lines;
        let expected = ["one", "two"];
        assert_eq!(log.iter().map(String::as_str).collect::<Vec<_>>(), expected);
    }
}

/// What a command's tests share: an app that has the console's messages and
/// resources, and a way to feed it a line of script and read what was said.
#[cfg(test)]
pub(crate) mod testing {
    use crate::tool_cmds::echo::*;
    use crate::TypedOrder;

    pub(crate) fn console_app() -> App {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>();
        app
    }

    /// Runs one frame with every command in `typed` waiting to be read.
    pub(crate) fn feed(app: &mut App, typed: &str) {
        for command in TypedOrder::parse_script(typed) {
            app.world_mut().write_message(command);
        }
        app.update();
    }

    /// Everything the console said so far, oldest first.
    pub(crate) fn said(app: &App) -> Vec<String> {
        app.world().resource::<PromptPanelStatus>().lines.to_vec()
    }
}
