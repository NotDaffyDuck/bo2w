//! The controls menu's label for each command: what it is bound to now.

use std::collections::BTreeMap;

use bevy::{ecs::change_detection::DetectChanges, ecs::system::Local, ecs::system::Res, ecs::system::ResMut};
use frame::InputDevices;
use frame::PromptStyle;
use frame::UiMenuDvars;

use crate::ActionBinding;
use crate::BindingTable;
use crate::button_label;

/// Per command, its keyboard and mouse names then its pad prompts, each side
/// sorted and joined with OR, the two sides with a bar: `f OR g | A OR X`.
/// Commands with nothing bound have no entry.
fn chord_texts(binds: &BindingTable, style: PromptStyle) -> BTreeMap<u32, String> {
    let mut sides = BTreeMap::<u32, (Vec<String>, Vec<String>)>::new();
    for (control, action) in binds.iter() {
        let (keys, pads) = sides.entry(action).or_default();
        match control {
            ActionBinding::Pad(pad) => pads.push(pad.prompt(style).to_owned()),
            other => keys.push(button_label(other)),
        }
    }
    sides
        .into_iter()
        .filter_map(|(command, (mut keys, mut pads))| {
            for list in [&mut keys, &mut pads] {
                list.sort();
                list.dedup();
            }
            let (keys, pads) = (keys.join(" OR "), pads.join(" OR "));
            let text = match (keys.is_empty(), pads.is_empty()) {
                (false, false) => format!("{keys} | {pads}"),
                (false, true) => keys,
                (true, false) => pads,
                (true, true) => return None,
            };
            Some((command, text))
        })
        .collect::<BTreeMap<_, _>>()
}

pub(crate) fn refresh_binding_rows(
    binds: Res<BindingTable>,
    devices: Res<InputDevices>,
    mut snapshot: ResMut<ui::KeymapSnapshot>,
    mut dvars: ResMut<UiMenuDvars>,
    mut shown_style: Local<Option<PromptStyle>>,
) {
    if !binds.is_changed() && *shown_style == Some(devices.style) {
        return;
    }
    *shown_style = Some(devices.style);
    snapshot.key_combos = chord_texts(&binds, devices.style);
    // Every command is published, so one that lost its last bind is relabelled.
    for (id, command) in input_t6::BIND_COMMAND_TABLE.iter().enumerate().skip(1) {
        dvars.set(&format!("ui_bind_{command}"), snapshot.key_combo(id as u32));
    }
    snapshot.revision = snapshot.revision.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use bevy::app::App;
    use bevy::prelude::*;

    use crate::prefs::view::*;
    use crate::GamepadKey;
    use bevy::app::Update;

    const ACTIVATE: &str = "+activate";

    fn activate() -> u32 {
        input_t6::action_number_for(ACTIVATE).expect("a known command")
    }

    fn view_app() -> App {
        let mut app = App::default();
        app.init_resource::<BindingTable>()
            .init_resource::<ui::KeymapSnapshot>()
            .init_resource::<UiMenuDvars>()
            .insert_resource(InputDevices {
                pad_prompts: false,
                style: PromptStyle::Generic,
                focused: true,
                aiming_with_pad: false,
            })
            .add_systems(Update, refresh_binding_rows);
        app
    }

    fn bind(app: &mut App, button: ActionBinding) {
        app.world_mut()
            .resource_mut::<BindingTable>()
            .assign(button, activate());
    }

    fn label(app: &App) -> String {
        app.world()
            .resource::<ui::KeymapSnapshot>()
            .key_combo(activate())
            .to_owned()
    }

    fn dvar(app: &App) -> Option<String> {
        app.world()
            .resource::<UiMenuDvars>()
            .get(&format!("ui_bind_{ACTIVATE}"))
            .map(str::to_owned)
    }

    fn revision(app: &App) -> u64 {
        app.world().resource::<ui::KeymapSnapshot>().revision
    }

    #[test]
    fn keys_are_sorted_and_joined_with_or() {
        let mut app = view_app();
        bind(&mut app, ActionBinding::Board(KeyCode::KeyG));
        bind(&mut app, ActionBinding::Board(KeyCode::KeyF));
        app.update();
        assert_eq!(label(&app), "f OR g");
        assert_eq!(dvar(&app).as_deref(), Some("f OR g"));
    }

    #[test]
    fn the_pad_side_follows_the_keys_after_a_bar() {
        let prompt = GamepadKey::Start.prompt(PromptStyle::Generic);
        let mut app = view_app();
        bind(&mut app, ActionBinding::Pad(GamepadKey::Start));
        app.update();
        assert_eq!(label(&app), prompt);
        bind(&mut app, ActionBinding::Board(KeyCode::KeyF));
        app.update();
        assert_eq!(label(&app), format!("f | {prompt}"));
    }

    #[test]
    fn a_command_with_nothing_bound_reads_unbound() {
        let mut app = view_app();
        app.update();
        assert_eq!(label(&app), "UNBOUND");
        assert_eq!(dvar(&app).as_deref(), Some("UNBOUND"));
    }

    #[test]
    fn losing_the_last_bind_relabels_the_command() {
        let mut app = view_app();
        bind(&mut app, ActionBinding::Board(KeyCode::KeyF));
        app.update();
        app.world_mut().resource_mut::<BindingTable>().forget_all();
        app.update();
        assert_eq!(label(&app), "UNBOUND");
        assert_eq!(dvar(&app).as_deref(), Some("UNBOUND"));
    }

    #[test]
    fn every_command_gets_a_dvar() {
        let mut app = view_app();
        app.update();
        let dvars = app.world().resource::<UiMenuDvars>();
        for command in input_t6::BIND_COMMAND_TABLE.iter().skip(1) {
            assert!(
                dvars.get(&format!("ui_bind_{command}")).is_some(),
                "no dvar for {command}"
            );
        }
    }

    #[test]
    fn nothing_is_republished_while_nothing_changes() {
        let mut app = view_app();
        app.update();
        let first = revision(&app);
        app.update();
        app.update();
        assert_eq!(revision(&app), first);
    }

    #[test]
    fn a_new_prompt_style_relabels_the_pad_buttons() {
        let mut app = view_app();
        bind(&mut app, ActionBinding::Pad(GamepadKey::Start));
        app.update();
        let before = revision(&app);
        app.world_mut().resource_mut::<InputDevices>().style = PromptStyle::PlayStation;
        app.update();
        assert_ne!(revision(&app), before);
        let actual = label(&app);
        assert_eq!(actual, GamepadKey::Start.prompt(PromptStyle::PlayStation));
    }
}
