//! The commands the console answers to, with the help and completions each
//! one offers.

use bevy::{ecs::system::Res, ecs::system::ResMut};
use input_t6::action_labels;

use crate::OrderBlueprint;
use crate::OrderCatalog;
use crate::BindingTable;
use crate::FixedListSuggester;
use crate::key_name_list;
use crate::weapon_cmds::GunParamHints;
use crate::weapon_cmds::install_gun_orders;

/// Adds `spec` unless a command of that name is already there.
fn register_once(book: &mut OrderCatalog, spec: OrderBlueprint) {
    if book.look_up(&spec.name).is_none() {
        book.enlist(spec);
    }
}

/// What a console command suggests for its arguments.
#[derive(Copy, Clone)]
enum Offers {
    Nothing,
    /// The `+input` names.
    Inputs,
    /// The `+input` names, or `all`.
    InputsOrAll,
    /// The key names.
    Keys,
    /// A key name, then an input name.
    KeyThenInput,
    /// The class presets.
    Presets,
}

/// The commands the console runs itself, in registration order: held
/// inputs, key binds, the overlay, and the two a script leans on, `class`
/// and `spawn`. Each help line starts with the command's own name.
const OWN_COMMANDS: &[(&str, Offers, &str)] = &[
    ("hold", Offers::Inputs, "hold <+input> — keep an input active until release"),
    ("press", Offers::Inputs, "press <+input> [seconds] — hold an input, then let it go"),
    (
        "key",
        Offers::Nothing,
        "key <key> [seconds] — hold a keyboard key or mouse button (through the binds), then release it",
    ),
    (
        "keydown",
        Offers::Nothing,
        "keydown <key> — press a keyboard key or mouse button (through the binds) and keep it down until keyup",
    ),
    ("keyup", Offers::Nothing, "keyup <key> — let go of a key that keydown holds"),
    ("mousemove", Offers::Nothing, "mousemove <dx> [dy] — inject one-shot mouse pixels"),
    ("mouserate", Offers::Nothing, "mouserate <dx> [dy] — add mouse pixels every frame until mouserate 0"),
    ("release", Offers::InputsOrAll, "release <+input|all> — stop a held console input"),
    ("bind", Offers::KeyThenInput, "bind [key] [command] — set or list key binds"),
    ("unbind", Offers::Keys, "unbind <key> — clear a key bind"),
    ("unbindall", Offers::Nothing, "unbindall — clear every key bind"),
    ("binddefaults", Offers::Nothing, "binddefaults — restore the default control script"),
    ("con_toggle", Offers::Nothing, "con_toggle — open/close the overlay (debug)"),
    ("con_feed", Offers::Nothing, "con_feed <text> — type <text> into the prompt as key presses (debug, skips the OS)"),
    ("con_paste", Offers::Nothing, "con_paste — insert OS clipboard into the prompt (debug)"),
    ("class", Offers::Presets, "class [name|index] — list presets, or pick one to spawn with"),
    ("mark", Offers::Nothing, "mark <label> — log a monotonic engine timestamp for external timing"),
    ("spawn", Offers::Presets, "spawn [name|index] — Equip the selected class and enter the match"),
];

fn own_commands(presets: &[String]) -> Vec<OrderBlueprint> {
    let input_names = || FixedListSuggester::new(action_labels());
    let key_names = || FixedListSuggester::new(key_name_list());
    OWN_COMMANDS
        .iter()
        .map(|&(name, offers, help)| {
            let blueprint = OrderBlueprint::new(name).help_line(help);
            match offers {
                Offers::Nothing => blueprint,
                Offers::Inputs => blueprint.arg(input_names()),
                Offers::InputsOrAll => blueprint.arg(FixedListSuggester::new(action_labels().chain(["all"]))),
                Offers::Keys => blueprint.arg(key_names()),
                Offers::KeyThenInput => blueprint.arg(key_names()).arg(input_names()),
                Offers::Presets => blueprint.arg(FixedListSuggester::new(presets.to_vec())),
            }
        })
        .collect::<Vec<OrderBlueprint>>()
}

/// Fills the registry: the console's own commands first, then every
/// feature's, so that a feature cannot shadow a console command.
pub(super) fn register_commands(
    mut book: ResMut<OrderCatalog>,
    mut binds: ResMut<BindingTable>,
    gun_hints: Res<GunParamHints>,
) {
    binds.restore_stock_controls();
    let class_names: Vec<String> = ui::showcase_classes().iter().map(|class| class.name.to_string()).collect();
    for blueprint in own_commands(&class_names) {
        register_once(&mut book, blueprint);
    }
    install_gun_orders(&mut book, &gun_hints);
    crate::showpos::enlist(&mut book);
    crate::player_cmds::enlist(&mut book);
    crate::dvars::register_all(&mut book);
    crate::fog::enlist(&mut book);
    crate::vision::enlist(&mut book);
    crate::mover_cmds::enlist(&mut book);
    crate::lock_sight::enlist(&mut book);
    crate::reports::enlist(&mut book);
    // No game folder (a bare test run) just means no map names to offer.
    let maps = match asset_transport::game_dir_from_environment() {
        Ok(game_dir) => asset_transport::multiplayer_map_names(&game_dir),
        Err(_) => Default::default(),
    };
    crate::tool_cmds::install_tool_orders(&mut book, &maps);
    crate::menus::enlist(&mut book);
    crate::class_menu::enlist(&mut book);
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::app::commands::*;

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
                spec.help_line.starts_with(&spec.name),
                "{} has usage {:?}",
                spec.name,
                spec.help_line
            );
        }
    }

    #[test]
    fn a_command_registered_first_is_not_replaced() {
        let mut book = OrderCatalog::default();
        register_once(&mut book, OrderBlueprint::new("mark").help_line("mark first"));
        register_once(&mut book, OrderBlueprint::new("mark").help_line("mark second"));
        assert_eq!(book.all_names(), ["mark"]);
        let expected = Some("mark first");
        assert_eq!(book.look_up("mark").map(|s| s.help_line.as_str()), expected);
    }

    #[test]
    fn class_and_spawn_complete_to_the_presets() {
        let mut book = OrderCatalog::default();
        for spec in own_commands(&presets()) {
            register_once(&mut book, spec);
        }
        assert_eq!(book.hint_list("spawn sm", 8), ["smg"]);
        assert_eq!(book.hint_list("class a", 7), ["assault"]);
    }

    #[test]
    fn held_inputs_complete_from_the_input_names() {
        let mut book = OrderCatalog::default();
        for spec in own_commands(&presets()) {
            register_once(&mut book, spec);
        }
        let offered = book.hint_list("hold +", 6);
        assert!(!offered.is_empty());
        assert!(offered.iter().all(|name| name.starts_with('+')));
    }
}
