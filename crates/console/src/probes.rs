//! Systems that tie the console to the rest of the client: the one-line
//! network and replay readouts, lines other systems hand the console, and
//! what happens once a replay has finished.

use bevy::app::{App, Update};
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::system::{Res, ResMut};
use frame::ShellScreen;
use net::{ClientStep, LinkDiagnosticsReport};
use replay::RecordingPlayer;
use replay::RecordingStatusInfo;
use session::BootConsoleScript;
use session::SessionHandoverOrder;
use session::StagedConsoleText;

use crate::showpos::update_overlay;
use crate::tool_cmds::CoordinateReadout;
use crate::OrderInbox;
use crate::PromptOutputRow;
use crate::TypedOrderBacklog;

/// Copies a freshly written network or replay status into the readout row.
/// When both changed this frame the replay's line is the one kept.
fn show_status_line(
    net: Option<Res<LinkDiagnosticsReport>>,
    replay: Option<Res<RecordingStatusInfo>>,
    mut line: ResMut<PromptOutputRow>,
) {
    let replay_line = match replay {
        Some(status) if status.is_changed() => status.line.clone(),
        _ => None,
    };
    let net_line = match net {
        Some(status) if status.is_changed() => status.line.clone(),
        _ => None,
    };
    if let Some(newest) = replay_line.or(net_line) {
        line.0 = newest;
    }
}

/// Feeds the console every line left for it. Lines from the start-up script
/// join them only once the main menu is up.
fn queue_waiting_lines(
    mut waiting: ResMut<StagedConsoleText>,
    startup: Option<ResMut<BootConsoleScript>>,
    shown_page: Option<Res<ShellScreen>>,
    mut queue: ResMut<OrderInbox>,
) {
    let menu_is_up = matches!(shown_page.as_deref(), Some(ShellScreen::MainMenu));
    if menu_is_up && let Some(mut script) = startup {
        waiting.0.append(&mut script.commands);
    }
    let lines = std::mem::take(&mut waiting.0);
    for text in &lines {
        queue.enqueue_typed_text(text);
    }
}

/// Once per finished replay: queues `quit` when the replay was started to
/// exit afterwards, and otherwise asks the session to drop back to the menu.
fn leave_ended_replay(
    playback: Option<ResMut<RecordingPlayer>>,
    mut commands: ResMut<TypedOrderBacklog>,
    swap: Option<ResMut<SessionHandoverOrder>>,
) {
    let Some(mut playback) = playback else { return };
    let finished_now = playback.over && !playback.shutdown_requested;
    if !finished_now {
        return;
    }
    playback.shutdown_requested = true;
    let ticks = playback.ticks_played();
    if playback.exit_when_finished {
        commands.enqueue_script_text("quit");
        diag::info!(Console, "play: replay over after {ticks} ticks, quit queued");
        return;
    }
    let Some(mut swap) = swap else {
        diag::warn!(Console, "demo: {ticks} ticks played, no SessionSwapRequest to leave with");
        return;
    };
    match swap.request_leave() {
        Ok(id) => diag::info!(Console, "demo: {ticks} ticks played, back to the menu (swap #{id})"),
        Err(why) => diag::info!(Console, "demo: {ticks} ticks played, staying put: {why}"),
    }
}

/// Adds the readout resource and the systems above, in the client's
/// diagnostics set.
pub(crate) fn install_metrics_echo(app: &mut App) {
    app.init_resource::<CoordinateReadout>();
    let systems = (show_status_line, queue_waiting_lines, leave_ended_replay, update_overlay);
    app.add_systems(Update, systems.in_set(ClientStep::Diag));
}

#[cfg(test)]
mod tests {
    use bevy::app::{App, Update};
    use frame::ShellScreen;
    use net::LinkDiagnosticsReport;
    use replay::RecordingStatusInfo;
    use session::BootConsoleScript;
    use session::StagedConsoleText;

    use crate::probes::{queue_waiting_lines, show_status_line};
    use crate::OrderInbox;
    use crate::PromptOutputRow;

    fn console_app() -> App {
        let mut app = App::default();
        app.init_resource::<PromptOutputRow>();
        app.init_resource::<OrderInbox>();
        app.init_resource::<StagedConsoleText>();
        app
    }

    fn readout(app: &App) -> &str {
        &app.world().resource::<PromptOutputRow>().0
    }

    fn sent(app: &mut App) -> Vec<String> {
        let drained = app.world_mut().resource_mut::<OrderInbox>().drain();
        drained.into_iter().map(|order| order.as_written).collect::<Vec<String>>()
    }

    #[test]
    fn a_new_network_status_shows_once() {
        let mut app = console_app();
        app.add_systems(Update, show_status_line);
        let line = Some("net: connected".to_owned());
        app.insert_resource(LinkDiagnosticsReport { line });
        app.update();
        assert_eq!(readout(&app), "net: connected");
        app.world_mut().resource_mut::<PromptOutputRow>().0 = "typed since".to_owned();
        app.update();
        assert_eq!(readout(&app), "typed since", "an unchanged status is not shown again");
    }

    #[test]
    fn the_replay_status_beats_the_network_in_one_frame() {
        let mut app = console_app();
        app.add_systems(Update, show_status_line);
        app.insert_resource(LinkDiagnosticsReport { line: Some("net".to_owned()) });
        app.insert_resource(RecordingStatusInfo { line: Some("replay".to_owned()) });
        app.update();
        assert_eq!(readout(&app), "replay");
    }

    #[test]
    fn a_status_without_a_line_leaves_the_readout_empty() {
        let mut app = console_app();
        app.add_systems(Update, show_status_line);
        app.init_resource::<LinkDiagnosticsReport>();
        app.update();
        assert_eq!(readout(&app), "");
    }

    #[test]
    fn lines_left_for_the_console_are_sent_straight_away() {
        let mut app = console_app();
        app.add_systems(Update, queue_waiting_lines);
        let lines = vec!["map a".to_owned(), "quit".to_owned()];
        app.world_mut().resource_mut::<StagedConsoleText>().0 = lines;
        app.update();
        assert_eq!(sent(&mut app), ["map a", "quit"]);
        assert_eq!(app.world().resource::<StagedConsoleText>().0.len(), 0);
    }

    #[test]
    fn the_startup_script_holds_until_the_main_menu() {
        let mut app = console_app();
        app.add_systems(Update, queue_waiting_lines);
        app.insert_resource(BootConsoleScript { commands: vec!["map a".to_owned()] });
        app.insert_resource(ShellScreen::Loading);
        app.update();
        assert_eq!(sent(&mut app).len(), 0);
        *app.world_mut().resource_mut::<ShellScreen>() = ShellScreen::MainMenu;
        app.update();
        assert_eq!(sent(&mut app), ["map a"]);
        assert_eq!(app.world().resource::<BootConsoleScript>().commands.len(), 0);
    }
}
