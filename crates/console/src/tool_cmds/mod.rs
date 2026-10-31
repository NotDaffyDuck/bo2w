//! The commands that reach into the rest of the game: bots, screenshots,
//! sessions, demos, dumps. Each lives in its own module with a router system
//! that reads the `TypedOrder` messages it owns and ignores the others.

use bevy::ecs::resource::Resource;

mod bots;
mod capture;
mod echo;
mod files;
mod hitvol;
mod process;
mod replay;
pub(super) mod session;
mod state_dump;
mod ui;

pub(crate) use bots::route_bot_commands;
pub(crate) use capture::steer_screenshot_orders;
pub(crate) use echo::PromptReplyWriter;
#[cfg(test)]
pub(crate) use echo::testing;
pub(crate) use hitvol::route_hitvol_commands;
pub(crate) use process::exit_process;
pub(crate) use replay::steer_playback_orders;
pub(crate) use session::{carry_on_session_orders, steer_lobby_orders};
pub(crate) use state_dump::steer_snapshot_orders;
pub(crate) use ui::steer_menu_orders;

/// Whether the on-screen position readout is showing (`debug_pos`).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct CoordinateReadout(pub bool);

/// Commands the queue itself runs, so the registry knows them for `help` and
/// completion.
const GENERAL: &[(&str, &str)] = &[
    ("help", "help — list every registered command"),
    ("clear", "clear — empty the console scrollback"),
    (
        "splash",
        "splash <key> [optionalNumber] — splash slot 0 from mp/splashTable.csv (one_shot_kill, longshot, capture, …)",
    ),
    (
        "wait",
        "wait [seconds|<n>t|world|spawn|torn|ambient] — hold the console FIFO; <n>t waits n authority ticks; world waits for scene.spawned; spawn for AppScreen::InGame; torn for HasWorld false and scene.spawned false (after MatchTornDown); ambient for MapAmbientBooted (overlay done, CreateFX loops running)",
    ),
];

/// Adds the commands of this module to the registry, each unless something
/// has already registered the name. `zones` are what `map` completes.
pub fn install_tool_orders(catalog: &mut crate::OrderCatalog, zones: &[String]) {
    if catalog.look_up("map").is_none() {
        catalog.enlist(
            crate::OrderBlueprint::new("map")
                .help_line("map <zone> — tear down whatever is running, then load the zone given")
                .arg(crate::FixedListSuggester::new(zones.iter().cloned())),
        );
    }
    let tables = [
        GENERAL,
        ui::HELP_BANNER,
        bots::HELP_BANNER,
        capture::HELP_BANNER,
        hitvol::HELP_BANNER,
        session::HELP_BANNER,
        replay::HELP_BANNER,
        state_dump::HELP_BANNER,
    ];
    for &(order, usage) in tables.into_iter().flatten() {
        if catalog.look_up(order).is_none() {
            catalog.enlist(crate::OrderBlueprint::new(order).help_line(usage));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tool_cmds::*;

    fn registered(maps: &[&str]) -> crate::OrderCatalog {
        let mut book = crate::OrderCatalog::default();
        let maps: Vec<String> = maps.iter().map(|map| (*map).to_owned()).collect();
        install_tool_orders(&mut book, &maps);
        book
    }

    #[test]
    fn every_command_a_router_owns_is_registered() {
        let names = registered(&[]).all_names();
        for want in [
            "bot",
            "clear",
            "clip",
            "demo",
            "disconnect",
            "dump",
            "exit",
            "finish_run",
            "help",
            "hitvol",
            "hurt",
            "map",
            "map_restart",
            "play",
            "quit",
            "record",
            "screenshot",
            "splash",
            "stoprecord",
            "ui",
            "wait",
        ] {
            assert!(names.iter().any(|name| name == want), "{want} missing");
        }
    }

    #[test]
    fn map_completes_the_zones_it_was_given() {
        let book = registered(&["mp_nuketown", "mp_raid", "zm_town"]);
        let expected = ["mp_nuketown", "mp_raid"];
        assert_eq!(book.hint_list("map mp_", 7), expected);
    }

    #[test]
    fn a_name_that_was_there_first_is_left_alone() {
        let mut book = crate::OrderCatalog::default();
        book.enlist(crate::OrderBlueprint::new("map").help_line("map — mine"));
        install_tool_orders(&mut book, &[]);
        let expected = 1;
        assert_eq!(book
            .all_names()
            .iter()
            .filter(|name| *name == "map")
            .count(), expected);
        assert!(book.usage_listing().contains(&"map — mine".to_owned()));
    }

    #[test]
    fn every_help_line_starts_with_a_name() {
        for line in registered(&[]).usage_listing() {
            assert!(
                !line.starts_with([' ', '\u{2014}']),
                "a line that does not start with a name: {line}"
            );
        }
    }
}
