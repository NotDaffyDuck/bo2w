//! `r_lockPvs [0|1]`: freeze the viewpoint the portal walk uses, so the
//! player can fly out of it and see what it left in and out.

use bevy::{ecs::message::MessageReader, ecs::system::Res, ecs::system::ResMut};
use render_frontend::prepare::scene::eye_parms::{StagedStageEye, FrozenVisibilitySet};

use crate::tool_cmds::PromptReplyWriter;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;

const NAME: &str = "r_lockPvs";
const HELP_BANNER: &str = "usage: r_lockPvs [0|1]";

pub(crate) fn enlist(book: &mut OrderCatalog) {
    if book.look_up(NAME).is_none() {
        let help = "r_lockPvs [0|1] — freeze the viewpoint the portal walk uses (`lockPvsViewParms`)";
        book.enlist(OrderBlueprint::new(NAME).help_line(help));
    }
}

fn switch(word: &str) -> Option<bool> {
    const ON: [&str; 3] = ["1", "on", "true"];
    const OFF: [&str; 3] = ["0", "off", "false"];
    let on = ON.contains(&word);
    let off = OFF.contains(&word);
    (on || off).then_some(on)
}

/// Turning it on takes the view as it is now when there is one; with none
/// yet, the freeze is taken from the first view that arrives.
fn answer(args: &[String], lock: &mut FrozenVisibilitySet, live: &StagedStageEye) -> String {
    let word = match args {
        [] => {
            let enabled = u8::from(lock.enabled);
            let frozen = u8::from(lock.frozen.is_some());
            return format!("{NAME} = {enabled} frozen={frozen}");
        }
        [word] => word,
        _ => return HELP_BANNER.into(),
    };
    let Some(on) = switch(word) else {
        return HELP_BANNER.into();
    };
    if !on {
        *lock = FrozenVisibilitySet::default();
        return format!("{NAME} = 0");
    }
    lock.enabled |= true;
    lock.capture_again = true;
    if live.primed {
        lock.refresh_from(live);
    }
    format!("{NAME} = 1")
}

pub(crate) fn route_lock_pvs(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut lock: ResMut<FrozenVisibilitySet>,
    live: Res<StagedStageEye>,
) {
    for command in commands.read().filter(|command| command.name == NAME) {
        echo.write(answer(&command.words, &mut lock, &live));
    }
}

#[cfg(test)]
mod tests {
    use bevy::{app::App, prelude::Vec3};
    use crate::lock_sight::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use bevy::app::Update;

    fn app(ready: bool) -> App {
        let mut app = console_app();
        app.init_resource::<FrozenVisibilitySet>()
            .insert_resource(StagedStageEye {
                primed: ready,
                eye: Vec3::new(1.0, 2.0, 3.0),
                ..StagedStageEye::default()
            })
            .add_systems(Update, route_lock_pvs);
        app
    }

    fn lock(app: &App) -> &FrozenVisibilitySet {
        app.world().resource::<FrozenVisibilitySet>()
    }

    #[test]
    fn asked_it_says_whether_it_is_on_and_holding_a_view() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs; r_lockPvs 1; r_lockPvs");
        let expected = [
            "r_lockPvs = 0 frozen=0",
            "r_lockPvs = 1",
            "r_lockPvs = 1 frozen=1"
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn on_freezes_the_view_there_is_and_off_lets_go() {
        for word in ["1", "on", "true"] {
            let mut app = app(true);
            feed(&mut app, &format!("r_lockPvs {word}"));
            assert!(lock(&app).enabled, "{word}");
            let expected = Some(Vec3::new(1.0, 2.0, 3.0));
            assert_eq!(lock(&app).frozen.as_ref().map(|view| view.eye), expected);
            assert!(!lock(&app).capture_again, "taken, so nothing left to take");
        }
        for word in ["0", "off", "false"] {
            let mut app = app(true);
            feed(&mut app, &format!("r_lockPvs 1; r_lockPvs {word}"));
            assert!(!lock(&app).enabled, "{word}");
            assert!(lock(&app).frozen.is_none(), "{word}");
            assert!(!lock(&app).capture_again, "{word}");
            assert_eq!(said(&app).last().unwrap(), "r_lockPvs = 0");
        }
    }

    #[test]
    fn on_before_there_is_a_view_waits_for_one() {
        let mut app = app(false);
        feed(&mut app, "r_lockPvs 1");
        assert!(lock(&app).enabled);
        assert!(lock(&app).frozen.is_none());
        assert!(lock(&app).capture_again, "the first view that arrives is taken");
    }

    #[test]
    fn a_second_on_does_not_move_the_freeze_until_the_view_is_retaken() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs 1");
        app.world_mut().resource_mut::<StagedStageEye>().eye = Vec3::splat(9.0);
        feed(&mut app, "r_lockPvs 1");
        let expected = Some(Vec3::splat(9.0));
        assert_eq!(lock(&app).frozen.as_ref().map(|view| view.eye), expected, "on again takes the view again");
    }

    #[test]
    fn anything_else_is_usage_and_changes_nothing() {
        let mut app = app(true);
        feed(&mut app, "r_lockPvs 2; r_lockPvs 1 1; r_lockPvs maybe");
        assert_eq!(said(&app), [HELP_BANNER; 3]);
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
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        assert!(book.look_up(NAME).is_some());
        let expected = 1;
        assert_eq!(book
            .usage_listing()
            .iter()
            .filter(|line| line.starts_with("r_lockPvs [0|1]"))
            .count(), expected);
    }
}
