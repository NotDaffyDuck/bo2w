//! The controls menu's label for each command: what it is bound to now.

use std::collections::BTreeMap;

use bevy::prelude::*;
use frame::{InputDevices, PromptStyle, UiMenuDvars};

use crate::{BindButton, KeyBinds, display_button};

/// Per command, its keyboard and mouse names then its pad prompts, each side
/// sorted and joined with OR, the two sides with a bar: `f OR g | A OR X`.
/// Commands with nothing bound have no entry.
fn chord_texts(binds: &KeyBinds, style: PromptStyle) -> BTreeMap<u32, String> {
    let mut sides = BTreeMap::<u32, (Vec<String>, Vec<String>)>::new();
    for (button, id) in binds.iter() {
        let (keys, pads) = sides.entry(id).or_default();
        match button {
            BindButton::Pad(pad) => pads.push(pad.prompt(style).to_owned()),
            other => keys.push(display_button(other)),
        }
    }
    sides
        .into_iter()
        .filter_map(|(id, (mut keys, mut pads))| {
            for names in [&mut keys, &mut pads] {
                names.sort();
                names.dedup();
            }
            let (keys, pads) = (keys.join(" OR "), pads.join(" OR "));
            let text = match (keys.is_empty(), pads.is_empty()) {
                (false, false) => format!("{keys} | {pads}"),
                (false, true) => keys,
                (true, false) => pads,
                (true, true) => return None,
            };
            Some((id, text))
        })
        .collect()
}

pub(crate) fn sync_binding_view(
    binds: Res<KeyBinds>,
    devices: Res<InputDevices>,
    mut view: ResMut<ui::BindingView>,
    mut dvars: ResMut<UiMenuDvars>,
    mut shown_style: Local<Option<PromptStyle>>,
) {
    if !binds.is_changed() && *shown_style == Some(devices.style) {
        return;
    }
    *shown_style = Some(devices.style);
    view.chords = chord_texts(&binds, devices.style);
    // Every command is published, so one that lost its last bind is relabelled.
    for (id, command) in input_t6::INPUT_COMMAND_NAMES.iter().enumerate().skip(1) {
        dvars.set(&format!("ui_bind_{command}"), view.chord(id as u32));
    }
    view.revision = view.revision.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use bevy::input::keyboard::KeyCode;

    use super::*;
    use crate::PadButton;

    const ACTIVATE: &str = "+activate";

    fn activate() -> u32 {
        input_t6::command_id_lookup(ACTIVATE).expect("a known command")
    }

    fn view_app() -> App {
        let mut app = App::new();
        app.init_resource::<KeyBinds>()
            .init_resource::<ui::BindingView>()
            .init_resource::<UiMenuDvars>()
            .insert_resource(InputDevices {
                pad_prompts: false,
                style: PromptStyle::Generic,
                focused: true,
                aiming_with_pad: false,
            })
            .add_systems(Update, sync_binding_view);
        app
    }

    fn bind(app: &mut App, button: BindButton) {
        app.world_mut()
            .resource_mut::<KeyBinds>()
            .set(button, activate());
    }

    fn label(app: &App) -> String {
        app.world()
            .resource::<ui::BindingView>()
            .chord(activate())
            .to_owned()
    }

    fn dvar(app: &App) -> Option<String> {
        app.world()
            .resource::<UiMenuDvars>()
            .get(&format!("ui_bind_{ACTIVATE}"))
            .map(str::to_owned)
    }

    fn revision(app: &App) -> u64 {
        app.world().resource::<ui::BindingView>().revision
    }

    #[test]
    fn keys_are_sorted_and_joined_with_or() {
        let mut app = view_app();
        bind(&mut app, BindButton::Key(KeyCode::KeyG));
        bind(&mut app, BindButton::Key(KeyCode::KeyF));
        app.update();
        assert_eq!(label(&app), "f OR g");
        assert_eq!(dvar(&app).as_deref(), Some("f OR g"));
    }

    #[test]
    fn the_pad_side_follows_the_keys_after_a_bar() {
        let prompt = PadButton::Start.prompt(PromptStyle::Generic);
        let mut app = view_app();
        bind(&mut app, BindButton::Pad(PadButton::Start));
        app.update();
        assert_eq!(label(&app), prompt);
        bind(&mut app, BindButton::Key(KeyCode::KeyF));
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
        bind(&mut app, BindButton::Key(KeyCode::KeyF));
        app.update();
        app.world_mut().resource_mut::<KeyBinds>().clear_all();
        app.update();
        assert_eq!(label(&app), "UNBOUND");
        assert_eq!(dvar(&app).as_deref(), Some("UNBOUND"));
    }

    #[test]
    fn every_command_gets_a_dvar() {
        let mut app = view_app();
        app.update();
        let dvars = app.world().resource::<UiMenuDvars>();
        for command in input_t6::INPUT_COMMAND_NAMES.iter().skip(1) {
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
        bind(&mut app, BindButton::Pad(PadButton::Start));
        app.update();
        let before = revision(&app);
        app.world_mut().resource_mut::<InputDevices>().style = PromptStyle::PlayStation;
        app.update();
        assert_ne!(revision(&app), before);
        assert_eq!(
            label(&app),
            PadButton::Start.prompt(PromptStyle::PlayStation)
        );
    }
}
