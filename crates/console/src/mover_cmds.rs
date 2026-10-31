//! `rotatevelocity <deg_per_sec>`: the speed a GSC `rotatevelocity` call
//! would have given the spinning fan blades, sent to the host as a debug
//! action.

use bevy::{ecs::message::MessageReader, ecs::system::Res, ecs::system::ResMut};
use net::{ActionRequestCounter, HostSimulationWorld, PlayerActionIntake, HostViewClient};
use sim::PlayerIntent;

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

const NAME: &str = "rotatevelocity";

pub(crate) fn enlist(book: &mut OrderCatalog) {
    if book.look_up(NAME).is_none() {
        book.enlist(OrderBlueprint::new(NAME).help_line(
            "rotatevelocity <deg_per_sec> — spin speed for the spawned fan blades, as GSC would give it (listen)",
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
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    local: Res<HostViewClient>,
    authority: Option<Res<HostSimulationWorld>>,
    mut inbox: Option<ResMut<PlayerActionIntake>>,
    mut ids: ResMut<ActionRequestCounter>,
) {
    for command in commands.read().filter(|command| command.name == NAME) {
        let cheats_off = authority
            .as_deref()
            .is_some_and(|authority| !authority.0.cheats_enabled());
        let said = speed_of(&command.words).and_then(|speed| {
            if cheats_off {
                return Err(format!("{NAME}: cheats are off"));
            }
            let inbox = Option::as_deref_mut(&mut inbox)
                .ok_or_else(|| format!("{NAME}: no action inbox (not a listen host)"))?;
            let ticket = ids.allocate();
            inbox
                .push(
                    local.0,
                    PlayerIntent::SpinMover { request_id: ticket, speed },
                )
                .map_err(|why| format!("{NAME}: {why}"))?;
            Ok(format!(
                "{NAME}: queued speed={speed} request_id={ticket} (given, not rolled at random)"
            ))
        });
        echo.write(said.unwrap_or_else(|why| why));
    }
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use net::PLAYER_ACTION_QUEUE_LIMIT;

    use crate::mover_cmds::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use bevy::app::Update;

    fn app_with_inbox() -> App {
        let mut app = console_app();
        app.init_resource::<HostViewClient>()
            .init_resource::<ActionRequestCounter>()
            .init_resource::<PlayerActionIntake>()
            .add_systems(Update, route_script_mover_commands);
        app
    }

    fn queued(app: &App) -> Vec<PlayerIntent> {
        let local = app.world().resource::<HostViewClient>().0;
        app.world()
            .resource::<PlayerActionIntake>()
            .awaiting_from(local)
            .collect::<Vec<PlayerIntent>>()
    }

    #[test]
    fn a_speed_is_queued_for_the_local_player_under_the_next_request_id() {
        let mut app = app_with_inbox();
        feed(&mut app, "rotatevelocity 90; rotatevelocity -45.5");
        let expected = [
            "rotatevelocity: queued speed=90 request_id=1 (given, not rolled at random)",
            "rotatevelocity: queued speed=-45.5 request_id=2 (given, not rolled at random)",
        ];
        assert_eq!(said(&app), expected);
        let (first, second) = (1, 2);
        let expected = [
            PlayerIntent::SpinMover {
                request_id: first,
                speed: 90.0
            },
            PlayerIntent::SpinMover {
                request_id: second,
                speed: -45.5
            },
        ];
        assert_eq!(queued(&app), expected);
    }

    #[test]
    fn anything_but_one_finite_number_queues_nothing() {
        let mut app = app_with_inbox();
        feed(&mut app, "rotatevelocity; rotatevelocity 1 2");
        let expected = ["usage: rotatevelocity <deg_per_sec>"; 2];
        assert_eq!(said(&app), expected, "the wrong number of arguments");
        feed(
            &mut app,
            "rotatevelocity fast; rotatevelocity inf; rotatevelocity NaN",
        );
        let expected = [
            "rotatevelocity: not a finite number `fast`",
            "rotatevelocity: not a finite number `inf`",
            "rotatevelocity: not a finite number `NaN`",
        ];
        assert_eq!(&said(&app)[2..], expected);
        assert_eq!(queued(&app), []);
    }

    #[test]
    fn a_game_with_cheats_off_refuses() {
        let mut app = app_with_inbox();
        app.init_resource::<HostSimulationWorld>();
        feed(&mut app, "rotatevelocity 90");
        assert_eq!(said(&app), ["rotatevelocity: cheats are off"]);
        assert_eq!(queued(&app), []);
    }

    #[test]
    fn a_client_without_the_host_inbox_says_so() {
        let mut app = console_app();
        app.init_resource::<HostViewClient>()
            .init_resource::<ActionRequestCounter>()
            .add_systems(Update, route_script_mover_commands);
        feed(&mut app, "rotatevelocity 90");
        let actual = said(&app);
        assert_eq!(actual, ["rotatevelocity: no action inbox (not a listen host)"]);
    }

    #[test]
    fn a_full_queue_says_how_full() {
        let mut app = app_with_inbox();
        let line = vec!["rotatevelocity 1"; PLAYER_ACTION_QUEUE_LIMIT + 1].join("; ");
        feed(&mut app, &line);
        assert_eq!(queued(&app).len(), PLAYER_ACTION_QUEUE_LIMIT);
        let expected = Some("rotatevelocity: action queue full (32 unresolved requests)");
        assert_eq!(said(&app).last().map(String::as_str), expected);
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
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        assert!(book.look_up(NAME).is_some());
        let before = book.usage_listing();
        enlist(&mut book);
        assert_eq!(book.usage_listing(), before);
        assert!(
            before
                .iter()
                .any(|line| line.starts_with("rotatevelocity <deg_per_sec>"))
        );
    }
}
