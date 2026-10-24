//! `map`, `map_restart` and `disconnect`: each asks the session crate for a
//! swap. The command queue waits for the swap (the queue's `begin_sync` sets
//! that up), so a request that is refused lets the queue go at once, and a
//! swap that fails does the same when it ends.

use ::replay::ReplayPlayback;
use ::session::{SessionSwapCompletion, SessionSwapRequest, SessionSwapResult};
use bevy::{ecs::system::SystemParam, prelude::*};
use frame::{HasWorld, LaunchIdentity};
use net::MasterBridge;

use super::echo::ConsoleEcho;
use crate::{ConsoleCommand, ConsoleDispatch};

pub(super) const USAGE: &[(&str, &str)] = &[
    (
        "map_restart",
        "map_restart — a new match on the current map, on what the last one prepared",
    ),
    (
        "disconnect",
        "disconnect — leave the session: tear the world down, leave the room, back to the main menu",
    ),
];

/// What the session commands need to know about the game.
#[derive(SystemParam)]
pub(crate) struct SessionDesk<'w> {
    swap: ResMut<'w, SessionSwapRequest>,
    has_world: Option<Res<'w, HasWorld>>,
    playback: Option<Res<'w, ReplayPlayback>>,
    bridge: Option<Res<'w, MasterBridge>>,
    identity: Option<Res<'w, LaunchIdentity>>,
}

impl SessionDesk<'_> {
    fn has_world(&self) -> bool {
        self.has_world.as_deref().is_some_and(|has| has.0)
    }

    /// Starts the map again, and says so; `Err` is what to say when it cannot.
    fn restart_map(&mut self, args: &[String]) -> Result<String, String> {
        if !args.is_empty() {
            return Err("usage: map_restart".into());
        }
        let zone = self
            .identity
            .as_deref()
            .map(|identity| identity.zone.clone())
            .filter(|zone| self.has_world() && !zone.is_empty())
            .ok_or("map_restart: no map to restart")?;
        let id = self
            .swap
            .request_zone(zone.clone())
            .map_err(|why| format!("map_restart: {why}"))?;
        Ok(format!("map_restart: requested `{zone}` (swap #{id})"))
    }

    fn change_map(&mut self, args: &[String]) -> Result<String, String> {
        let [zone] = args else {
            return Err("usage: map <zone>".into());
        };
        let id = self
            .swap
            .request_zone(zone.clone())
            .map_err(|why| format!("map: {why}"))?;
        Ok(format!("map: requested `{zone}` (swap #{id})"))
    }

    fn leave(&mut self, args: &[String]) -> Result<String, String> {
        if !args.is_empty() {
            return Err("usage: disconnect".into());
        }
        let in_session = self.has_world()
            || self.playback.is_some()
            || self.bridge.is_some()
            || self.swap.dump_id().is_some();
        if !in_session {
            return Err("disconnect: no session to leave".into());
        }
        let id = self
            .swap
            .request_leave()
            .map_err(|why| format!("disconnect: {why}"))?;
        diag::lifecycle_boundary("disconnect_requested", &format!(" swap={id}"));
        Ok(format!(
            "disconnect: waiting for session teardown (swap #{id})"
        ))
    }
}

pub(crate) fn route_session_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut desk: SessionDesk,
    mut dispatch: ResMut<ConsoleDispatch>,
) {
    for command in commands.read() {
        let answer = match command.name.as_str() {
            "map_restart" => desk.restart_map(&command.args),
            "map" => desk.change_map(&command.args),
            "disconnect" => desk.leave(&command.args),
            _ => continue,
        };
        match answer {
            Ok(said) => echo.write(said),
            Err(said) => {
                dispatch.release();
                echo.write(said);
            }
        }
    }
}

/// What the queue does when a swap ends: a failed one lets it go and is
/// reported, any other ends the wait on the swap itself and leaves a wait
/// for the new world in place.
fn resume(dispatch: &mut ConsoleDispatch, done: SessionSwapCompletion) -> Option<String> {
    match done.result {
        SessionSwapResult::Failed { zone, error } => {
            dispatch.release();
            Some(format!("map: `{zone}` failed: {error}"))
        }
        SessionSwapResult::Installed { .. } | SessionSwapResult::Menu => {
            dispatch.paused = false;
            None
        }
    }
}

pub(crate) fn resume_lifecycle_commands(
    mut dispatch: ResMut<ConsoleDispatch>,
    mut swap: ResMut<SessionSwapRequest>,
    mut echo: ConsoleEcho,
) {
    if let Some(said) = swap
        .take_completed()
        .and_then(|done| resume(&mut dispatch, done))
    {
        echo.write(said);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConsoleLine, ConsoleSettings, ConsoleState, app::Hold};

    /// An app that has run `line` with the queue paused, the way a sync
    /// command leaves it.
    fn run(line: &str, setup: impl FnOnce(&mut App)) -> App {
        let mut app = App::new();
        app.add_message::<ConsoleCommand>()
            .init_resource::<ConsoleState>()
            .init_resource::<ConsoleSettings>()
            .init_resource::<ConsoleLine>()
            .init_resource::<SessionSwapRequest>()
            .insert_resource(ConsoleDispatch::in_flight())
            .add_systems(Update, route_session_commands);
        setup(&mut app);
        for command in ConsoleCommand::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn said(app: &App) -> &str {
        &app.world().resource::<ConsoleLine>().0
    }

    fn queue_paused(app: &App) -> bool {
        app.world().resource::<ConsoleDispatch>().paused
    }

    fn in_a_match(app: &mut App) {
        app.insert_resource(HasWorld(true))
            .insert_resource(LaunchIdentity {
                zone: "mp_raid".into(),
                ..LaunchIdentity::default()
            });
    }

    #[test]
    fn map_asks_for_the_zone_and_keeps_the_queue_waiting() {
        let app = run("map mp_nuketown", |_| {});
        assert_eq!(said(&app), "map: requested `mp_nuketown` (swap #1)");
        assert!(queue_paused(&app));
    }

    #[test]
    fn map_without_one_zone_is_usage_and_lets_the_queue_go() {
        for line in ["map", "map a b"] {
            let app = run(line, |_| {});
            assert_eq!(said(&app), "usage: map <zone>", "{line}");
            assert!(!queue_paused(&app), "{line}");
        }
    }

    #[test]
    fn a_second_map_while_one_is_loading_is_refused() {
        let app = run("map a; map b", |_| {});
        assert_eq!(said(&app), "map: session swap already in progress");
        assert!(!queue_paused(&app));
    }

    #[test]
    fn map_restart_needs_a_match_to_restart() {
        let app = run("map_restart", |_| {});
        assert_eq!(said(&app), "map_restart: no map to restart");
        assert!(!queue_paused(&app));

        let app = run("map_restart", |app| {
            app.insert_resource(LaunchIdentity {
                zone: "mp_raid".into(),
                ..LaunchIdentity::default()
            });
        });
        assert_eq!(said(&app), "map_restart: no map to restart", "no world yet");

        let app = run("map_restart", |app| {
            app.insert_resource(HasWorld(true))
                .init_resource::<LaunchIdentity>();
        });
        assert_eq!(said(&app), "map_restart: no map to restart", "no map name");
    }

    #[test]
    fn map_restart_asks_for_the_current_map() {
        let app = run("map_restart", in_a_match);
        assert_eq!(said(&app), "map_restart: requested `mp_raid` (swap #1)");
        assert!(queue_paused(&app));

        let app = run("map_restart now", in_a_match);
        assert_eq!(said(&app), "usage: map_restart");
        assert!(!queue_paused(&app));
    }

    #[test]
    fn disconnect_needs_something_to_leave() {
        let app = run("disconnect", |_| {});
        assert_eq!(said(&app), "disconnect: no session to leave");
        assert!(!queue_paused(&app));

        let app = run("disconnect", in_a_match);
        assert_eq!(
            said(&app),
            "disconnect: waiting for session teardown (swap #1)"
        );
        assert!(queue_paused(&app));

        let app = run("disconnect now", in_a_match);
        assert_eq!(said(&app), "usage: disconnect");
        assert!(!queue_paused(&app));
    }

    #[test]
    fn a_swap_under_way_counts_as_a_session_to_leave() {
        let app = run("map a; disconnect", |_| {});
        assert_eq!(
            said(&app),
            "disconnect: waiting for session teardown (swap #1)"
        );
    }

    #[test]
    fn a_failed_swap_lets_the_queue_go_and_says_why() {
        let mut dispatch = ConsoleDispatch::in_flight();
        dispatch.hold(Hold::World);
        let said = resume(
            &mut dispatch,
            SessionSwapCompletion {
                id: 3,
                result: SessionSwapResult::Failed {
                    zone: "mp_raid".into(),
                    error: "no such zone".into(),
                },
            },
        );
        assert_eq!(said.as_deref(), Some("map: `mp_raid` failed: no such zone"));
        assert!(!dispatch.paused);
        assert_eq!(dispatch.holding(), None);
    }

    #[test]
    fn a_finished_swap_only_ends_its_own_wait() {
        for result in [
            SessionSwapResult::Installed {
                zone: "mp_raid".into(),
            },
            SessionSwapResult::Menu,
        ] {
            let mut dispatch = ConsoleDispatch::in_flight();
            dispatch.hold(Hold::World);
            assert_eq!(
                resume(&mut dispatch, SessionSwapCompletion { id: 1, result }),
                None
            );
            assert!(!dispatch.paused);
            assert_eq!(dispatch.holding(), Some(Hold::World));
        }
    }
}
