//! The commands that reach into the rest of the game: bots, screenshots,
//! sessions, demos, dumps. Each lives in its own module with a router system
//! that reads the `ConsoleCommand` messages it owns and ignores the others.

use bevy::prelude::Resource;

mod bots;
mod capture;
mod echo;
mod files;
mod hitvol;
mod process;
mod replay;
mod session;
mod state_dump;
mod ui;

pub(crate) use bots::route_bot_commands;
pub(crate) use capture::route_capture_commands;
pub(crate) use echo::ConsoleEcho;
#[cfg(test)]
pub(crate) use echo::testing;
pub(crate) use hitvol::route_hitvol_commands;
pub(crate) use process::exit_process;
pub(crate) use replay::route_replay_commands;
pub(crate) use session::{resume_lifecycle_commands, route_session_commands};
pub(crate) use state_dump::route_state_dump_commands;
pub(crate) use ui::route_ui_commands;

/// Whether the on-screen position readout is showing (`debug_pos`).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct DebugPosOverlay(pub bool);

/// Commands the queue itself runs, so the registry knows them for `help` and
/// completion.
const GENERAL: &[(&str, &str)] = &[
    ("help", "help — list registered commands"),
    ("clear", "clear — clear console scrollback"),
    (
        "splash",
        "splash <key> [optionalNumber] — splash slot 0 from mp/splashTable.csv (one_shot_kill, longshot, capture, …)",
    ),
    (
        "wait",
        "wait [seconds|<n>t|world|spawn|torn|ambient] — pause the console FIFO; <n>t = n authority ticks; world = scene.spawned; spawn = AppScreen::InGame; torn = HasWorld false and scene.spawned false (hold after MatchTornDown); ambient = MapAmbientBooted (overlay finished, CreateFX loops spawned)",
    ),
];

/// Adds the commands of this module to the registry, each unless something
/// has already registered the name. `maps` are the zones `map` completes.
pub fn register_feature_commands(registry: &mut crate::ConsoleRegistry, maps: &[String]) {
    if registry.resolve("map").is_none() {
        registry.register(
            crate::CommandSpec::new("map")
                .usage("map <zone> — tear down the current occupancy, then load a zone")
                .arg(crate::StaticCompleter::new(maps.iter().cloned())),
        );
    }
    let tables = [
        GENERAL,
        ui::USAGE,
        bots::USAGE,
        capture::USAGE,
        hitvol::USAGE,
        session::USAGE,
        replay::USAGE,
        state_dump::USAGE,
    ];
    for &(name, usage) in tables.into_iter().flatten() {
        if registry.resolve(name).is_none() {
            registry.register(crate::CommandSpec::new(name).usage(usage));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(maps: &[&str]) -> crate::ConsoleRegistry {
        let mut registry = crate::ConsoleRegistry::default();
        let maps: Vec<String> = maps.iter().map(|map| (*map).to_owned()).collect();
        register_feature_commands(&mut registry, &maps);
        registry
    }

    #[test]
    fn every_command_a_router_owns_is_registered() {
        let names = registered(&[]).names();
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
        let registry = registered(&["mp_nuketown", "mp_raid", "zm_town"]);
        assert_eq!(
            registry.suggestions("map mp_", 7),
            ["mp_nuketown", "mp_raid"]
        );
    }

    #[test]
    fn a_name_that_was_there_first_is_left_alone() {
        let mut registry = crate::ConsoleRegistry::default();
        registry.register(crate::CommandSpec::new("map").usage("map — mine"));
        register_feature_commands(&mut registry, &[]);
        assert_eq!(
            registry
                .names()
                .iter()
                .filter(|name| *name == "map")
                .count(),
            1
        );
        assert!(registry.help_lines().contains(&"map — mine".to_owned()));
    }

    #[test]
    fn every_help_line_starts_with_a_name() {
        for line in registered(&[]).help_lines() {
            assert!(
                !line.starts_with([' ', '\u{2014}']),
                "a line that does not start with a name: {line}"
            );
        }
    }
}
