//! `r_lockPvs [0|1]`: freeze the viewpoint the portal walk uses, so the
//! player can fly out of it and see what it left in and out.

use bevy::prelude::*;
use render_frontend::prepare::scene::view_parms::{PreparedSceneView, RLockPvs};

use crate::feature_dispatch::ConsoleEcho;
use crate::{CommandSpec, ConsoleCommand, ConsoleRegistry};

const NAME: &str = "r_lockPvs";
const USAGE: &str = "usage: r_lockPvs [0|1]";

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    if registry.resolve(NAME).is_none() {
        registry.register(
            CommandSpec::new(NAME)
                .usage("r_lockPvs [0|1] — freeze portal-walk viewpoint (`lockPvsViewParms`)"),
        );
    }
}

fn switch(word: &str) -> Option<bool> {
    match word {
        "1" | "on" | "true" => Some(true),
        "0" | "off" | "false" => Some(false),
        _ => None,
    }
}

/// Turning it on takes the view as it is now when there is one; with none
/// yet, the freeze is taken from the first view that arrives.
fn answer(args: &[String], lock: &mut RLockPvs, live: &PreparedSceneView) -> String {
    let word = match args {
        [] => {
            return format!(
                "{NAME} = {} frozen={}",
                u8::from(lock.enabled),
                u8::from(lock.frozen.is_some())
            );
        }
        [word] => word,
        _ => return USAGE.into(),
    };
    match switch(word) {
        Some(true) => {
            lock.enabled = true;
            lock.recapture = true;
            if live.ready {
                lock.apply_after_stamp(live);
            }
            format!("{NAME} = 1")
        }
        Some(false) => {
            *lock = RLockPvs::default();
            format!("{NAME} = 0")
        }
        None => USAGE.into(),
    }
}

pub(crate) fn route_lock_pvs(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut lock: ResMut<RLockPvs>,
    live: Res<PreparedSceneView>,
) {
    for command in commands.read().filter(|command| command.name == NAME) {
        echo.write(answer(&command.args, &mut lock, &live));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn app(ready: bool) -> App {
        let mut app = console_app();
        app.init_resource::<RLockPvs>()
            .insert_resource(PreparedSceneView {
                ready,
                eye: Vec3::new(1.0, 2.0, 3.0),
                ..PreparedSceneView::default()
            })
            .add_systems(Update, route_lock_pvs);
        app
    }

    fn lock(app: &App) -> &RLockPvs {
        app.world().resource::<RLockPvs>()
    }

    #[test]
    fn asked_it_says_whether_it_is_on_and_holding_a_view() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs; r_lockPvs 1; r_lockPvs");
        assert_eq!(
            said(&app),
            [
                "r_lockPvs = 0 frozen=0",
                "r_lockPvs = 1",
                "r_lockPvs = 1 frozen=1"
            ]
        );
    }

    #[test]
    fn on_freezes_the_view_there_is_and_off_lets_go() {
        for word in ["1", "on", "true"] {
            let mut app = app(true);
            feed(&mut app, &format!("r_lockPvs {word}"));
            assert!(lock(&app).enabled, "{word}");
            assert_eq!(
                lock(&app).frozen.as_ref().map(|view| view.eye),
                Some(Vec3::new(1.0, 2.0, 3.0))
            );
            assert!(!lock(&app).recapture, "taken, so nothing left to take");
        }
        for word in ["0", "off", "false"] {
            let mut app = app(true);
            feed(&mut app, &format!("r_lockPvs 1; r_lockPvs {word}"));
            assert!(!lock(&app).enabled, "{word}");
            assert!(lock(&app).frozen.is_none(), "{word}");
            assert!(!lock(&app).recapture, "{word}");
            assert_eq!(said(&app).last().unwrap(), "r_lockPvs = 0");
        }
    }

    #[test]
    fn on_before_there_is_a_view_waits_for_one() {
        let mut app = app(false);
        feed(&mut app, "r_lockPvs 1");
        assert!(lock(&app).enabled);
        assert!(lock(&app).frozen.is_none());
        assert!(lock(&app).recapture, "the first view that arrives is taken");
    }

    #[test]
    fn a_second_on_does_not_move_the_freeze_until_the_view_is_retaken() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs 1");
        app.world_mut().resource_mut::<PreparedSceneView>().eye = Vec3::splat(9.0);
        feed(&mut app, "r_lockPvs 1");
        assert_eq!(
            lock(&app).frozen.as_ref().map(|view| view.eye),
            Some(Vec3::splat(9.0)),
            "on again takes the view again"
        );
    }

    #[test]
    fn anything_else_is_usage_and_changes_nothing() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs 2; r_lockPvs 1 1; r_lockPvs maybe");
        assert_eq!(said(&app), [USAGE; 3]);
        assert!(!lock(&app).enabled);
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app(true);
        feed(&mut app, "r_lockPv 1; r_znear");
        assert_eq!(said(&app), Vec::<String>::new());
    }

    #[test]
    fn registered_once_with_a_usage_line() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        register(&mut registry);
        assert!(registry.resolve(NAME).is_some());
        assert_eq!(
            registry
                .help_lines()
                .iter()
                .filter(|line| line.starts_with("r_lockPvs [0|1]"))
                .count(),
            1
        );
    }
}
