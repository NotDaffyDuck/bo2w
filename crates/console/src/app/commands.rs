//! The commands the console answers to, with the help and completions each
//! one offers.

use bevy::prelude::*;
use input_t6::command_names;

use crate::{
    CommandSpec, ConsoleRegistry, KeyBinds, StaticCompleter, bindable_keys,
    weapon_dispatch::{WeaponArgCompletions, register_weapon_commands},
};

/// Adds `spec` unless a command of that name is already there.
fn register_once(registry: &mut ConsoleRegistry, spec: CommandSpec) {
    if registry.resolve(&spec.name).is_none() {
        registry.register(spec);
    }
}

/// The commands the console runs itself: held inputs, key binds, the overlay,
/// and the two a script leans on, `class` and `spawn`.
fn own_commands(presets: &[String]) -> Vec<CommandSpec> {
    let inputs = || StaticCompleter::new(command_names());
    let keys = || StaticCompleter::new(bindable_keys());
    vec![
        CommandSpec::new("hold")
            .usage("hold <+input> — keep an input active until release")
            .arg(inputs()),
        CommandSpec::new("press")
            .usage("press <+input> [seconds] — hold an input, then release it")
            .arg(inputs()),
        CommandSpec::new("key").usage(
            "key <key> [seconds] — hold a keyboard key or mouse button (through the binds), then release it",
        ),
        CommandSpec::new("mousemove").usage("mousemove <dx> [dy] — inject one-shot mouse pixels"),
        CommandSpec::new("mouserate")
            .usage("mouserate <dx> [dy] — add mouse pixels every frame until mouserate 0"),
        CommandSpec::new("release")
            .usage("release <+input|all> — stop a held console input")
            .arg(StaticCompleter::new(command_names().chain(["all"]))),
        CommandSpec::new("bind")
            .usage("bind [key] [command] — set or list key binds")
            .arg(keys())
            .arg(inputs()),
        CommandSpec::new("unbind")
            .usage("unbind <key> — clear a key bind")
            .arg(keys()),
        CommandSpec::new("unbindall").usage("unbindall — clear every key bind"),
        CommandSpec::new("binddefaults")
            .usage("binddefaults — restore the default control script"),
        CommandSpec::new("con_toggle").usage("con_toggle — open/close the overlay (debug)"),
        CommandSpec::new("con_feed")
            .usage("con_feed <text> — inject KeyboardInput.text (debug; not OS winit)"),
        CommandSpec::new("con_paste")
            .usage("con_paste — insert OS clipboard into the prompt (debug)"),
        CommandSpec::new("class")
            .usage("class [name|index] — list presets, or select one for spawn")
            .arg(StaticCompleter::new(presets.to_vec())),
        CommandSpec::new("mark")
            .usage("mark <label> — log a monotonic engine timestamp for external timing"),
        CommandSpec::new("spawn")
            .usage("spawn [name|index] — Equip the selected class and enter the match")
            .arg(StaticCompleter::new(presets.to_vec())),
    ]
}

/// Fills the registry: the console's own commands first, then every
/// feature's, so that a feature cannot shadow a console command.
pub(super) fn register_commands(
    mut registry: ResMut<ConsoleRegistry>,
    mut binds: ResMut<KeyBinds>,
    weapon_completions: Res<WeaponArgCompletions>,
) {
    binds.apply_defaults();
    let presets: Vec<String> = ui::showcase_classes()
        .iter()
        .map(|preset| preset.name.to_owned())
        .collect();
    for spec in own_commands(&presets) {
        register_once(&mut registry, spec);
    }
    register_weapon_commands(&mut registry, &weapon_completions);
    crate::showpos::register(&mut registry);
    crate::player_cmds::register(&mut registry);
    crate::dvars::register_all(&mut registry);
    crate::fog::register(&mut registry);
    crate::vision::register(&mut registry);
    crate::script_mover::register(&mut registry);
    crate::lock_pvs::register(&mut registry);
    crate::reports::register(&mut registry);
    let maps = match asset_transport::games_root_from_env() {
        Ok(root) => asset_transport::list_mp_maps(&root),
        Err(_) => Vec::new(),
    };
    crate::feature_dispatch::register_feature_commands(&mut registry, &maps);
    crate::frontend::register(&mut registry);
    crate::class_menu::register(&mut registry);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn presets() -> Vec<String> {
        vec!["assault".to_owned(), "smg".to_owned()]
    }

    #[test]
    fn every_own_command_has_a_unique_name_and_help() {
        let specs = own_commands(&presets());
        let names: HashSet<_> = specs.iter().map(|spec| spec.name.as_str()).collect();
        assert_eq!(names.len(), specs.len());
        for spec in &specs {
            assert!(
                spec.usage.starts_with(&spec.name),
                "{} has usage {:?}",
                spec.name,
                spec.usage
            );
        }
    }

    #[test]
    fn a_command_registered_first_is_not_replaced() {
        let mut registry = ConsoleRegistry::default();
        register_once(&mut registry, CommandSpec::new("mark").usage("mark first"));
        register_once(&mut registry, CommandSpec::new("mark").usage("mark second"));
        assert_eq!(registry.names(), ["mark"]);
        assert_eq!(
            registry.resolve("mark").map(|s| s.usage.as_str()),
            Some("mark first")
        );
    }

    #[test]
    fn class_and_spawn_complete_to_the_presets() {
        let mut registry = ConsoleRegistry::default();
        for spec in own_commands(&presets()) {
            register_once(&mut registry, spec);
        }
        assert_eq!(registry.suggestions("spawn sm", 8), ["smg"]);
        assert_eq!(registry.suggestions("class a", 7), ["assault"]);
    }

    #[test]
    fn held_inputs_complete_from_the_input_names() {
        let mut registry = ConsoleRegistry::default();
        for spec in own_commands(&presets()) {
            register_once(&mut registry, spec);
        }
        let offered = registry.suggestions("hold +", 6);
        assert!(!offered.is_empty());
        assert!(offered.iter().all(|name| name.starts_with('+')));
    }
}
