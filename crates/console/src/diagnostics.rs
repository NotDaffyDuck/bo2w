//! What the console shows and does about the rest of the game: the network and
//! replay status lines, commands waiting for the menu, and the end of a replay.

use bevy::prelude::*;
use frame::AppScreen;
use net::{ClientSet, NetDiagnostics};
use replay::{ReplayDiagnostics, ReplayPlayback};
use session::{PendingConsoleLines, SessionSwapRequest, StartupCommands};

use crate::{
    ConsoleCommandQueue, ConsoleLine, ConsoleQueue, feature_dispatch::DebugPosOverlay,
    showpos::update_overlay,
};

/// Shows the newest status line the network or a replay has written.
fn show_status_line(
    net: Option<Res<NetDiagnostics>>,
    replay: Option<Res<ReplayDiagnostics>>,
    mut line: ResMut<ConsoleLine>,
) {
    let from_net = net
        .filter(|net| net.is_changed())
        .and_then(|net| net.line.clone());
    let from_replay = replay
        .filter(|replay| replay.is_changed())
        .and_then(|replay| replay.line.clone());
    if let Some(text) = from_replay.or(from_net) {
        line.0 = text;
    }
}

/// Runs the lines other systems left for the console. The ones the process
/// started with wait for the main menu.
fn queue_waiting_lines(
    mut waiting: ResMut<PendingConsoleLines>,
    startup: Option<ResMut<StartupCommands>>,
    screen: Option<Res<AppScreen>>,
    mut queue: ResMut<ConsoleQueue>,
) {
    let at_menu = screen.is_some_and(|screen| *screen == AppScreen::MainMenu);
    if let Some(mut startup) = startup
        && at_menu
    {
        waiting.0.append(&mut startup.lines);
    }
    for line in waiting.0.drain(..) {
        queue.push_line(&line);
    }
}

/// When a replay has played out, quits if it was asked to, or goes back to
/// the menu.
fn leave_ended_replay(
    playback: Option<ResMut<ReplayPlayback>>,
    mut commands: ResMut<ConsoleCommandQueue>,
    swap: Option<ResMut<SessionSwapRequest>>,
) {
    let Some(mut playback) = playback else { return };
    if !playback.ended || playback.exit_queued {
        return;
    }
    playback.exit_queued = true;
    let ticks = playback.ticks();
    if playback.quit_on_end {
        commands.push_script("quit");
        diag::info!(Console, "play: queued quit after {ticks} ticks");
        return;
    }
    match swap.map(|mut swap| swap.request_leave()) {
        Some(Ok(id)) => diag::info!(
            Console,
            "demo: ended after {ticks} ticks — disconnect to menu (swap #{id})"
        ),
        Some(Err(error)) => diag::info!(
            Console,
            "demo: ended after {ticks} ticks — disconnect skipped: {error}"
        ),
        None => diag::warn!(
            Console,
            "demo: ended after {ticks} ticks but SessionSwapRequest is missing"
        ),
    }
}

pub(crate) fn register_diagnostics_mirror(app: &mut App) {
    app.init_resource::<DebugPosOverlay>().add_systems(
        Update,
        (
            show_status_line,
            queue_waiting_lines,
            leave_ended_replay,
            update_overlay,
        )
            .in_set(ClientSet::Diag),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_app() -> App {
        let mut app = App::new();
        app.init_resource::<ConsoleLine>()
            .init_resource::<ConsoleQueue>()
            .init_resource::<PendingConsoleLines>();
        app
    }

    fn queued(app: &mut App) -> Vec<String> {
        let mut queue = app.world_mut().resource_mut::<ConsoleQueue>();
        queue
            .drain()
            .into_iter()
            .map(|command| command.raw)
            .collect()
    }

    #[test]
    fn the_status_line_follows_the_network_when_it_changes() {
        let mut app = base_app();
        app.add_systems(Update, show_status_line);
        app.insert_resource(NetDiagnostics {
            line: Some("net: connected".into()),
        });
        app.update();
        assert_eq!(app.world().resource::<ConsoleLine>().0, "net: connected");
        app.world_mut().resource_mut::<ConsoleLine>().0 = "typed since".into();
        app.update();
        assert_eq!(
            app.world().resource::<ConsoleLine>().0,
            "typed since",
            "an unchanged status is not shown again"
        );
    }

    #[test]
    fn a_replay_line_wins_over_a_network_line_from_the_same_frame() {
        let mut app = base_app();
        app.add_systems(Update, show_status_line);
        app.insert_resource(NetDiagnostics {
            line: Some("net".into()),
        })
        .insert_resource(ReplayDiagnostics {
            line: Some("replay".into()),
        });
        app.update();
        assert_eq!(app.world().resource::<ConsoleLine>().0, "replay");
    }

    #[test]
    fn a_status_with_no_line_shows_nothing() {
        let mut app = base_app();
        app.add_systems(Update, show_status_line);
        app.init_resource::<NetDiagnostics>();
        app.update();
        assert!(app.world().resource::<ConsoleLine>().0.is_empty());
    }

    #[test]
    fn waiting_lines_go_to_the_console_at_once() {
        let mut app = base_app();
        app.add_systems(Update, queue_waiting_lines);
        app.world_mut().resource_mut::<PendingConsoleLines>().0 =
            vec!["map a".into(), "quit".into()];
        app.update();
        assert_eq!(queued(&mut app), ["map a", "quit"]);
        assert!(app.world().resource::<PendingConsoleLines>().0.is_empty());
    }

    #[test]
    fn startup_lines_wait_for_the_main_menu() {
        let mut app = base_app();
        app.add_systems(Update, queue_waiting_lines);
        app.insert_resource(StartupCommands {
            lines: vec!["map a".into()],
        })
        .insert_resource(AppScreen::Loading);
        app.update();
        assert!(queued(&mut app).is_empty());
        *app.world_mut().resource_mut::<AppScreen>() = AppScreen::MainMenu;
        app.update();
        assert_eq!(queued(&mut app), ["map a"]);
        assert!(app.world().resource::<StartupCommands>().lines.is_empty());
    }
}
