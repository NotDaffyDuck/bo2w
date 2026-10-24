//! `rotatevelocity <deg_per_sec>`: the speed a GSC `rotatevelocity` call
//! would have given the spinning fan blades, sent to the host as a debug
//! action.

use bevy::prelude::*;
use net::{ActionRequestIds, AuthorityWorld, ClientActionInbox, LocalPresentClient};
use sim::ClientAction;

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

const NAME: &str = "rotatevelocity";

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    if registry.resolve(NAME).is_none() {
        registry.register(CommandSpec::new(NAME).usage(
            "rotatevelocity <deg_per_sec> — supplied GSC rotatevelocity speed on spawned fan blades (listen)",
        ));
    }
}

/// The speed in the arguments, or what to say about them.
fn speed_of(args: &[String]) -> Result<f32, String> {
    let [text] = args else {
        return Err(format!("usage: {NAME} <deg_per_sec>"));
    };
    text.parse::<f32>()
        .ok()
        .filter(|speed| speed.is_finite())
        .ok_or_else(|| format!("{NAME}: not a finite number `{text}`"))
}

pub(crate) fn route_script_mover_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    local: Res<LocalPresentClient>,
    authority: Option<Res<AuthorityWorld>>,
    mut inbox: Option<ResMut<ClientActionInbox>>,
    mut ids: ResMut<ActionRequestIds>,
) {
    for command in commands.read().filter(|command| command.name == NAME) {
        let cheats_off = authority
            .as_deref()
            .is_some_and(|authority| !authority.0.cheats_enabled());
        let said = speed_of(&command.args).and_then(|speed| {
            if cheats_off {
                return Err(format!("{NAME}: cheats are off"));
            }
            let inbox = inbox
                .as_deref_mut()
                .ok_or_else(|| format!("{NAME}: no action inbox (not a listen host)"))?;
            let request_id = ids.allocate();
            inbox
                .push(
                    local.0,
                    ClientAction::BeginScriptMoverRotateVelocity { request_id, speed },
                )
                .map_err(|why| format!("{NAME}: {why}"))?;
            Ok(format!(
                "{NAME}: queued speed={speed} request_id={request_id} (supplied, not randomfloatrange)"
            ))
        });
        echo.write(said.unwrap_or_else(|why| why));
    }
}

#[cfg(test)]
mod tests {
    use net::MAX_PENDING_ACTIONS_PER_CLIENT;

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn app_with_inbox() -> App {
        let mut app = console_app();
        app.init_resource::<LocalPresentClient>()
            .init_resource::<ActionRequestIds>()
            .init_resource::<ClientActionInbox>()
            .add_systems(Update, route_script_mover_commands);
        app
    }

    fn queued(app: &App) -> Vec<ClientAction> {
        let local = app.world().resource::<LocalPresentClient>().0;
        app.world()
            .resource::<ClientActionInbox>()
            .pending_for(local)
            .collect()
    }

    #[test]
    fn a_speed_is_queued_for_the_local_player_under_the_next_request_id() {
        let mut app = app_with_inbox();
        feed(&mut app, "rotatevelocity 90; rotatevelocity -45.5");
        assert_eq!(
            said(&app),
            [
                "rotatevelocity: queued speed=90 request_id=1 (supplied, not randomfloatrange)",
                "rotatevelocity: queued speed=-45.5 request_id=2 (supplied, not randomfloatrange)",
            ]
        );
        assert_eq!(
            queued(&app),
            [
                ClientAction::BeginScriptMoverRotateVelocity {
                    request_id: 1,
                    speed: 90.0
                },
                ClientAction::BeginScriptMoverRotateVelocity {
                    request_id: 2,
                    speed: -45.5
                },
            ]
        );
    }

    #[test]
    fn anything_but_one_finite_number_queues_nothing() {
        let mut app = app_with_inbox();
        feed(&mut app, "rotatevelocity; rotatevelocity 1 2");
        assert_eq!(
            said(&app),
            ["usage: rotatevelocity <deg_per_sec>"; 2],
            "the wrong number of arguments"
        );
        feed(
            &mut app,
            "rotatevelocity fast; rotatevelocity inf; rotatevelocity NaN",
        );
        assert_eq!(
            &said(&app)[2..],
            [
                "rotatevelocity: not a finite number `fast`",
                "rotatevelocity: not a finite number `inf`",
                "rotatevelocity: not a finite number `NaN`",
            ]
        );
        assert_eq!(queued(&app), []);
    }

    #[test]
    fn a_game_with_cheats_off_refuses() {
        let mut app = app_with_inbox();
        app.init_resource::<AuthorityWorld>();
        feed(&mut app, "rotatevelocity 90");
        assert_eq!(said(&app), ["rotatevelocity: cheats are off"]);
        assert_eq!(queued(&app), []);
    }

    #[test]
    fn a_client_without_the_host_inbox_says_so() {
        let mut app = console_app();
        app.init_resource::<LocalPresentClient>()
            .init_resource::<ActionRequestIds>()
            .add_systems(Update, route_script_mover_commands);
        feed(&mut app, "rotatevelocity 90");
        assert_eq!(
            said(&app),
            ["rotatevelocity: no action inbox (not a listen host)"]
        );
    }

    #[test]
    fn a_full_queue_says_how_full() {
        let mut app = app_with_inbox();
        let line = vec!["rotatevelocity 1"; MAX_PENDING_ACTIONS_PER_CLIENT + 1].join("; ");
        feed(&mut app, &line);
        assert_eq!(queued(&app).len(), MAX_PENDING_ACTIONS_PER_CLIENT);
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("rotatevelocity: action queue full (32 unresolved requests)")
        );
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app_with_inbox();
        feed(&mut app, "rotate 90; fog");
        assert_eq!(said(&app), Vec::<String>::new());
        assert_eq!(queued(&app), []);
    }

    #[test]
    fn a_name_somebody_else_took_is_left_to_them() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        assert!(registry.resolve(NAME).is_some());
        let before = registry.help_lines();
        register(&mut registry);
        assert_eq!(registry.help_lines(), before);
        assert!(
            before
                .iter()
                .any(|line| line.starts_with("rotatevelocity <deg_per_sec>"))
        );
    }
}
