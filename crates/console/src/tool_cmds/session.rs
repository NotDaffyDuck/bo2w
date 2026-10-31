//! `map`, `map_restart` and `disconnect`: each asks the session crate for a
//! swap. The command queue waits for the swap (the queue's `begin_sync` sets
//! that up), so a request that is refused lets the queue go at once, and a
//! swap that fails does the same when it ends.

use ::replay::RecordingPlayer;
use ::session::{HandoverFinish, SessionHandoverOrder, HandoverVerdict};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::LevelPresent;
use frame::StartupPersona;
use net::DirectoryLink;

use super::echo::PromptReplyWriter;
use crate::TypedOrder;
use crate::TypedOrderRouter;

pub(super) const HELP_BANNER: &[(&str, &str)] = &[
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
    swap: ResMut<'w, SessionHandoverOrder>,
    level_is_up: Option<Res<'w, LevelPresent>>,
    playback: Option<Res<'w, RecordingPlayer>>,
    link: Option<Res<'w, DirectoryLink>>,
    persona: Option<Res<'w, StartupPersona>>,
}

impl SessionDesk<'_> {
    fn level_is_up(&self) -> bool {
        self.level_is_up.as_deref().is_some_and(|has| has.0)
    }

    /// Starts the map again, and says so; `Err` is what to say when it cannot.
    fn restart_map(&mut self, args: &[String]) -> Result<String, String> {
        if !args.is_empty() {
            return Err("usage: map_restart".into());
        }
        let zone = self
            .persona
            .as_deref()
            .map(|persona| persona.zone.clone())
            .filter(|zone| self.level_is_up() && !zone.is_empty())
            .ok_or("map_restart: no map to restart")?;
        let swap_no = self
            .swap
            .ask_for_region(zone.clone())
            .map_err(|why| format!("map_restart: {why}"))?;
        Ok(format!("map_restart: requested `{zone}` (swap #{swap_no})"))
    }

    fn change_map(&mut self, args: &[String]) -> Result<String, String> {
        let [zone] = args else {
            return Err("usage: map <zone>".into());
        };
        let swap_no = self
            .swap
            .ask_for_region(zone.clone())
            .map_err(|why| format!("map: {why}"))?;
        Ok(format!("map: requested `{zone}` (swap #{swap_no})"))
    }

    fn leave(&mut self, args: &[String]) -> Result<String, String> {
        if !args.is_empty() {
            return Err("usage: disconnect".into());
        }
        let in_session = self.level_is_up()
            || self.playback.is_some()
            || self.link.is_some()
            || self.swap.trace_dump_number().is_some();
        if !in_session {
            return Err("disconnect: no session to leave".into());
        }
        let swap_no = self
            .swap
            .request_leave()
            .map_err(|why| format!("disconnect: {why}"))?;
        diag::lifecycle_boundary("disconnect_requested", &format!(" swap={swap_no}"));
        Ok(format!(
            "disconnect: waiting for the session to tear down (swap #{swap_no})"
        ))
    }
}

pub(crate) fn steer_lobby_orders(
    mut orders: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut desk: SessionDesk,
    mut dispatch: ResMut<TypedOrderRouter>,
) {
    for order in orders.read() {
        let reply = match order.name.as_str() {
            "map_restart" => desk.restart_map(&order.words),
            "map" => desk.change_map(&order.words),
            "disconnect" => desk.leave(&order.words),
            _ => continue,
        };
        match reply {
            Ok(said) => echo.write(said),
            Err(said) => {
                dispatch.unblock();
                echo.write(said);
            }
        }
    }
}

/// What the queue does when a swap ends: a failed one lets it go and is
/// reported, any other ends the wait on the swap itself and leaves a wait
/// for the new world in place.
fn resume(dispatch: &mut TypedOrderRouter, done: HandoverFinish) -> Option<String> {
    match done.result {
        HandoverVerdict::Failed { zone, error } => {
            dispatch.unblock();
            Some(format!("map: `{zone}` failed: {error}"))
        }
        HandoverVerdict::Installed { .. } | HandoverVerdict::Menu => {
            dispatch.command_running = false;
            None
        }
    }
}

pub(crate) fn carry_on_session_orders(
    mut dispatch: ResMut<TypedOrderRouter>,
    mut swap: ResMut<SessionHandoverOrder>,
    mut echo: PromptReplyWriter,
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
    use crate::tool_cmds::session::*;
    use crate::PromptOutputRow;
    use crate::PromptPanelOptions;
    use crate::PromptPanelStatus;
    use crate::app::Hold;
    use bevy::app::Update;

    /// An app that has run `line` with the queue paused, the way a sync
    /// command leaves it.
    fn run(line: &str, setup: impl FnOnce(&mut App)) -> App {
        let mut app = App::default();
        app.add_message::<TypedOrder>()
            .init_resource::<PromptPanelStatus>()
            .init_resource::<PromptPanelOptions>()
            .init_resource::<PromptOutputRow>()
            .init_resource::<SessionHandoverOrder>()
            .insert_resource(TypedOrderRouter::in_flight())
            .add_systems(Update, steer_lobby_orders);
        setup(&mut app);
        for command in TypedOrder::parse_script(line) {
            app.world_mut().write_message(command);
        }
        app.update();
        app
    }

    fn said(app: &App) -> &str {
        &app.world().resource::<PromptOutputRow>().0
    }

    fn queue_paused(app: &App) -> bool {
        app.world().resource::<TypedOrderRouter>().command_running
    }

    fn in_a_match(app: &mut App) {
        app.insert_resource(LevelPresent(true))
            .insert_resource(StartupPersona {
                zone: "mp_raid".into(),
                ..StartupPersona::default()
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
            app.insert_resource(StartupPersona {
                zone: "mp_raid".into(),
                ..StartupPersona::default()
            });
        });
        assert_eq!(said(&app), "map_restart: no map to restart", "no world yet");

        let app = run("map_restart", |app| {
            app.insert_resource(LevelPresent(true))
                .init_resource::<StartupPersona>();
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
        let actual = said(&app);
        assert_eq!(actual, "disconnect: waiting for the session to tear down (swap #1)");
        assert!(queue_paused(&app));

        let app = run("disconnect now", in_a_match);
        assert_eq!(said(&app), "usage: disconnect");
        assert!(!queue_paused(&app));
    }

    #[test]
    fn a_swap_under_way_counts_as_a_session_to_leave() {
        let app = run("map a; disconnect", |_| {});
        let actual = said(&app);
        assert_eq!(actual, "disconnect: waiting for the session to tear down (swap #1)");
    }

    #[test]
    fn a_failed_swap_lets_the_queue_go_and_says_why() {
        let mut dispatch = TypedOrderRouter::in_flight();
        dispatch.stall(Hold::World);
        let said = resume(
            &mut dispatch,
            HandoverFinish {
                id: 3,
                result: HandoverVerdict::Failed {
                    zone: "mp_raid".into(),
                    error: "no such zone".into(),
                },
            },
        );
        assert_eq!(said.as_deref(), Some("map: `mp_raid` failed: no such zone"));
        assert!(!dispatch.command_running);
        assert_eq!(dispatch.holding(), None);
    }

    #[test]
    fn a_finished_swap_only_ends_its_own_wait() {
        for result in [
            HandoverVerdict::Installed {
                zone: "mp_raid".into(),
            },
            HandoverVerdict::Menu,
        ] {
            let mut dispatch = TypedOrderRouter::in_flight();
            dispatch.stall(Hold::World);
            let expected = None;
            assert_eq!(resume(&mut dispatch, HandoverFinish { id: 1, result }), expected);
            assert!(!dispatch.command_running);
            assert_eq!(dispatch.holding(), Some(Hold::World));
        }
    }
}
