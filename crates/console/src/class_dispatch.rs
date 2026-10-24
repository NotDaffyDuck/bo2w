//! `class` and `spawn`: pick a class preset, and equip it to enter the match.
//!
//! `spawn` equips at once when the class menu allows a change. Otherwise a
//! command that waits holds the queue until the menu opens, and one that runs
//! in the background is turned away.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{ActionRequestIds, SignonPhase, SignonState};
use ui::{
    ClassChangeAllowed, ClassChangeBlockReason, ClassEquipRefusal, ClassSelectHighlight,
    ClassSelectPhase, ClassSelectStatus, PendingClassEquip, SessionClassStore, class_index_by_name,
    commit_class_equip,
};

use crate::{
    ConsoleCommand, ConsoleDispatch, ConsoleLine, ConsoleSettings, ConsoleState, app::Hold,
};

/// What `spawn` does about the class menu, given what the menu allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpawnGate {
    EquipNow,
    WaitForAdmission,
    RefuseNow,
}

fn spawn_gate(change_allowed: bool, background: bool) -> SpawnGate {
    match (change_allowed, background) {
        (true, _) => SpawnGate::EquipNow,
        (false, true) => SpawnGate::RefuseNow,
        (false, false) => SpawnGate::WaitForAdmission,
    }
}

/// The class menu's state, which `class` and `spawn` read and change.
#[derive(SystemParam)]
pub(crate) struct ClassMenu<'w> {
    store: ResMut<'w, SessionClassStore>,
    highlight: ResMut<'w, ClassSelectHighlight>,
    phase: ResMut<'w, ClassSelectPhase>,
    pending: ResMut<'w, PendingClassEquip>,
    status: ResMut<'w, ClassSelectStatus>,
    request_ids: ResMut<'w, ActionRequestIds>,
}

impl ClassMenu<'_> {
    /// Asks the host to equip preset `index`; the id of the request it made.
    fn equip(&mut self, index: usize) -> Result<u32, ClassEquipRefusal> {
        commit_class_equip(
            index,
            &mut self.store,
            &mut self.highlight,
            &mut self.phase,
            &mut self.pending,
            &mut self.status,
            &mut self.request_ids,
        )
    }
}

/// Where the commands print: the console's log, its last line and the diagnostics.
#[derive(SystemParam)]
pub(crate) struct Say<'w> {
    console: ResMut<'w, ConsoleState>,
    line: ResMut<'w, ConsoleLine>,
    settings: Res<'w, ConsoleSettings>,
}

impl Say<'_> {
    fn say(&mut self, message: impl Into<String>) {
        let message = message.into();
        diag::info!(Console, "{message}");
        self.line.0.clone_from(&message);
        self.console.echo(message, self.settings.log_capacity);
    }
}

pub(crate) fn route_class_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut menu: ClassMenu,
    mut say: Say,
    mut dispatch: ResMut<ConsoleDispatch>,
    change_allowed: Res<ClassChangeAllowed>,
    block_reason: Res<ClassChangeBlockReason>,
) {
    for command in commands.read() {
        let named = command.args.first().map(String::as_str);
        match command.name.as_str() {
            "class" => match named {
                None => {
                    for line in listing(&menu.store, menu.highlight.0) {
                        say.say(line);
                    }
                }
                Some(name) => select(name, &mut menu, &mut say),
            },
            "spawn" => {
                if let Some(name) = named
                    && class_index_by_name(&menu.store, name).is_none()
                {
                    say.say(format!("spawn: no preset called `{name}`"));
                    continue;
                }
                match spawn_gate(change_allowed.0, command.background) {
                    SpawnGate::EquipNow => {
                        equip(
                            named,
                            command.background,
                            &mut menu,
                            &mut dispatch,
                            &mut say,
                        );
                    }
                    SpawnGate::WaitForAdmission => {
                        dispatch.hold(Hold::SpawnAdmit);
                        dispatch.pending_spawn_class = named.map(str::to_owned);
                        say.say("spawn: waiting for the class menu to open");
                    }
                    SpawnGate::RefuseNow => {
                        let reason = block_reason
                            .0
                            .as_deref()
                            .unwrap_or("class change not allowed");
                        say.say(format!("spawn: {reason}"));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Finishes a `spawn` that was waiting for the class menu: equips as soon as
/// the menu allows a change, and gives up if the session fails first.
pub(crate) fn complete_pending_spawn(
    mut menu: ClassMenu,
    mut say: Say,
    mut dispatch: ResMut<ConsoleDispatch>,
    change_allowed: Res<ClassChangeAllowed>,
    signon: Option<Res<SignonState>>,
) {
    if dispatch.holding() != Some(Hold::SpawnAdmit) {
        return;
    }
    if let Some(signon) = signon
        && (signon.phase.is_failed() || signon.phase == SignonPhase::Cancelled)
    {
        let reason = signon
            .phase
            .status_line()
            .unwrap_or_else(|| "session failed".to_owned());
        dispatch.end_hold();
        dispatch.pending_spawn_class = None;
        say.say(format!("spawn: given up — {reason}"));
        return;
    }
    if !change_allowed.0 {
        return;
    }
    let class = dispatch.pending_spawn_class.take();
    dispatch.end_hold();
    equip(class.as_deref(), false, &mut menu, &mut dispatch, &mut say);
}

/// Makes the preset called `name` the one the menu has picked.
fn select(name: &str, menu: &mut ClassMenu, say: &mut Say) {
    let Some(index) = class_index_by_name(&menu.store, name) else {
        say.say(format!(
            "class: no preset called `{name}` — `class` lists them"
        ));
        return;
    };
    if menu.phase.is_pending() {
        say.say("class: an equip is already waiting on the host");
        return;
    }
    menu.highlight.0 = index;
    menu.store.selected = index;
    say.say(format!("class: picked {}", describe(&menu.store, index)));
}

/// Equips `class`, or the preset the menu has picked when there is none. A
/// foreground spawn then holds the queue until the player is in the game.
fn equip(
    class: Option<&str>,
    background: bool,
    menu: &mut ClassMenu,
    dispatch: &mut ConsoleDispatch,
    say: &mut Say,
) {
    let index = match class {
        None => menu.highlight.0,
        Some(name) => match class_index_by_name(&menu.store, name) {
            Some(index) => index,
            None => {
                say.say(format!("spawn: no preset called `{name}`"));
                return;
            }
        },
    };
    match menu.equip(index) {
        Ok(request_id) if background => {
            let slot = describe(&menu.store, index);
            say.say(format!("spawn &: equipping {slot} (request {request_id})"));
        }
        Ok(request_id) => {
            dispatch.hold(Hold::Spawn);
            let slot = describe(&menu.store, index);
            say.say(format!(
                "spawn: equipping {slot} (request {request_id}), waiting until in game"
            ));
        }
        Err(refusal) => say.say(format!("spawn: {refusal}")),
    }
}

/// One preset on a line: its number, name and weapons, and why it is locked.
fn describe(store: &SessionClassStore, index: usize) -> String {
    let Some(slot) = store.slots.get(index) else {
        return format!("{index} (no such preset)");
    };
    let locked = slot
        .lock_reason
        .as_deref()
        .map_or_else(String::new, |reason| format!(" LOCKED — {reason}"));
    format!(
        "{index} `{}` primary={} secondary={}{locked}",
        slot.name, slot.primary, slot.secondary
    )
}

/// What `class` prints: a header, then every preset, the picked one marked.
fn listing(store: &SessionClassStore, picked: usize) -> Vec<String> {
    let equipped = store
        .equipped
        .map_or_else(|| "none".to_owned(), |index| index.to_string());
    let mut lines = vec![format!(
        "class: {} presets, picked {picked}, equipped {equipped}",
        store.slots.len()
    )];
    lines.extend((0..store.slots.len()).map(|index| {
        let mark = if index == picked { '>' } else { ' ' };
        format!("{mark} {}", describe(store, index))
    }));
    lines
}

/// The names `class` and `spawn` complete.
pub fn class_completions(store: &SessionClassStore) -> Vec<String> {
    store.slots.iter().map(|slot| slot.name.clone()).collect()
}

#[cfg(test)]
mod tests {
    use frame::HostClassSlot;
    use ui::ClassSlotState;

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};

    fn preset(name: &str, lock: Option<&str>) -> ClassSlotState {
        let mut slot = ClassSlotState::from_host_slot(&HostClassSlot {
            name: name.to_owned(),
            primary: format!("{name}_gun"),
            primary_attachments: Vec::new(),
            secondary: format!("{name}_pistol"),
            secondary_attachments: Vec::new(),
            lethal: String::new(),
            tactical: String::new(),
            perks: Default::default(),
            deathstreak: String::new(),
        });
        slot.lock_reason = lock.map(str::to_owned);
        slot
    }

    fn app() -> App {
        let mut app = console_app();
        app.init_resource::<ConsoleDispatch>()
            .init_resource::<ClassSelectHighlight>()
            .init_resource::<ClassSelectPhase>()
            .init_resource::<PendingClassEquip>()
            .init_resource::<ClassSelectStatus>()
            .init_resource::<ActionRequestIds>()
            .init_resource::<ClassChangeAllowed>()
            .init_resource::<ClassChangeBlockReason>()
            .insert_resource(SessionClassStore {
                slots: vec![
                    preset("assault", None),
                    preset("smg", None),
                    preset("sniper", Some("rank 20")),
                ],
                equipped: None,
                selected: 0,
            })
            .add_systems(
                Update,
                (route_class_commands, complete_pending_spawn).chain(),
            );
        app
    }

    fn open_menu(app: &mut App) {
        app.world_mut().insert_resource(ClassChangeAllowed(true));
    }

    fn holding(app: &App) -> Option<Hold> {
        app.world().resource::<ConsoleDispatch>().holding()
    }

    fn picked(app: &App) -> usize {
        app.world().resource::<ClassSelectHighlight>().0
    }

    fn requested(app: &App) -> Option<usize> {
        let phase = app.world().resource::<ClassSelectPhase>();
        match phase {
            ClassSelectPhase::Pending { class_index, .. } => Some(*class_index),
            ClassSelectPhase::Interactive => None,
        }
    }

    #[test]
    fn the_menu_decides_what_spawn_does() {
        assert_eq!(spawn_gate(true, false), SpawnGate::EquipNow);
        assert_eq!(spawn_gate(true, true), SpawnGate::EquipNow);
        assert_eq!(spawn_gate(false, false), SpawnGate::WaitForAdmission);
        assert_eq!(spawn_gate(false, true), SpawnGate::RefuseNow);
    }

    #[test]
    fn class_lists_the_presets_and_marks_the_picked_one() {
        let mut app = app();
        feed(&mut app, "class");
        assert_eq!(
            said(&app),
            [
                "class: 3 presets, picked 0, equipped none",
                "> 0 `assault` primary=assault_gun secondary=assault_pistol",
                "  1 `smg` primary=smg_gun secondary=smg_pistol",
                "  2 `sniper` primary=sniper_gun secondary=sniper_pistol LOCKED — rank 20",
            ]
        );
    }

    #[test]
    fn class_names_a_preset_by_name_in_any_case_or_by_number() {
        let mut app = app();
        feed(&mut app, "class SMG");
        assert_eq!(picked(&app), 1);
        assert_eq!(app.world().resource::<SessionClassStore>().selected, 1);
        feed(&mut app, "class 0");
        assert_eq!(picked(&app), 0);
        assert_eq!(app.world().resource::<SessionClassStore>().selected, 0);
        assert_eq!(
            said(&app),
            [
                "class: picked 1 `smg` primary=smg_gun secondary=smg_pistol",
                "class: picked 0 `assault` primary=assault_gun secondary=assault_pistol",
            ]
        );
    }

    #[test]
    fn class_says_when_no_preset_has_the_name() {
        let mut app = app();
        feed(&mut app, "class knife");
        assert_eq!(
            said(&app),
            ["class: no preset called `knife` — `class` lists them"]
        );
        assert_eq!(picked(&app), 0);
    }

    #[test]
    fn class_waits_while_an_equip_is_on_its_way() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn smg &");
        feed(&mut app, "class assault");
        assert_eq!(picked(&app), 1);
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("class: an equip is already waiting on the host")
        );
    }

    #[test]
    fn spawn_with_a_menu_that_allows_it_equips_and_holds_the_queue() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn smg");
        assert_eq!(holding(&app), Some(Hold::Spawn));
        assert_eq!(requested(&app), Some(1));
        assert_eq!(picked(&app), 1);
        assert!(app.world().resource::<PendingClassEquip>().0.is_some());
        assert_eq!(
            said(&app),
            [
                "spawn: equipping 1 `smg` primary=smg_gun secondary=smg_pistol (request 1), waiting until in game"
            ]
        );
    }

    #[test]
    fn spawn_with_no_name_equips_the_picked_preset() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "class smg");
        feed(&mut app, "spawn");
        assert_eq!(requested(&app), Some(1));
    }

    #[test]
    fn a_spawn_in_the_background_equips_without_holding() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn assault &");
        assert_eq!(holding(&app), None);
        assert_eq!(requested(&app), Some(0));
        assert_eq!(
            said(&app),
            [
                "spawn &: equipping 0 `assault` primary=assault_gun secondary=assault_pistol (request 1)"
            ]
        );
    }

    #[test]
    fn spawn_turns_away_a_preset_that_is_locked() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn sniper");
        assert_eq!(holding(&app), None);
        assert_eq!(requested(&app), None);
        assert_eq!(said(&app), ["spawn: locked_content: preset 2 — rank 20"]);
    }

    #[test]
    fn spawn_turns_away_a_second_equip_while_one_is_pending() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn smg &");
        feed(&mut app, "spawn assault &");
        assert_eq!(requested(&app), Some(1));
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("spawn: equip already pending (request_id=1)")
        );
    }

    #[test]
    fn spawn_says_when_no_preset_has_the_name() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn knife");
        assert_eq!(said(&app), ["spawn: no preset called `knife`"]);
        assert_eq!(holding(&app), None);
    }

    #[test]
    fn spawn_before_the_menu_opens_waits_for_it() {
        let mut app = app();
        feed(&mut app, "spawn smg");
        assert_eq!(holding(&app), Some(Hold::SpawnAdmit));
        assert_eq!(
            app.world()
                .resource::<ConsoleDispatch>()
                .pending_spawn_class,
            Some("smg".to_owned())
        );
        assert_eq!(requested(&app), None);
        assert_eq!(said(&app), ["spawn: waiting for the class menu to open"]);
    }

    #[test]
    fn a_spawn_that_waited_equips_when_the_menu_opens() {
        let mut app = app();
        feed(&mut app, "spawn smg");
        app.update();
        assert_eq!(holding(&app), Some(Hold::SpawnAdmit));

        open_menu(&mut app);
        app.update();
        assert_eq!(holding(&app), Some(Hold::Spawn));
        assert_eq!(requested(&app), Some(1));
        let dispatch = app.world().resource::<ConsoleDispatch>();
        assert_eq!(dispatch.pending_spawn_class, None);
    }

    #[test]
    fn a_spawn_that_waited_gives_up_when_the_session_fails() {
        let mut app = app();
        feed(&mut app, "spawn smg");
        app.insert_resource(SignonState {
            phase: SignonPhase::Cancelled,
            ..Default::default()
        });
        app.update();
        assert_eq!(holding(&app), None);
        assert_eq!(
            app.world()
                .resource::<ConsoleDispatch>()
                .pending_spawn_class,
            None
        );
        assert_eq!(requested(&app), None);
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("spawn: given up — signon cancelled")
        );
    }

    #[test]
    fn a_background_spawn_before_the_menu_opens_says_why_not() {
        let mut app = app();
        feed(&mut app, "spawn &");
        assert_eq!(holding(&app), None);
        assert_eq!(said(&app), ["spawn: class change not allowed"]);

        app.world_mut().insert_resource(ClassChangeBlockReason(Some(
            "waiting for host: loading map".to_owned(),
        )));
        feed(&mut app, "spawn &");
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("spawn: waiting for host: loading map")
        );
    }

    #[test]
    fn the_second_system_does_nothing_unless_a_spawn_waits() {
        let mut app = app();
        open_menu(&mut app);
        app.update();
        assert_eq!(requested(&app), None);
        assert!(said(&app).is_empty());
    }

    #[test]
    fn other_commands_are_not_theirs() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "echo hi; classes; spawned");
        assert!(said(&app).is_empty());
        assert_eq!(requested(&app), None);
    }

    #[test]
    fn completions_are_the_preset_names() {
        let app = app();
        let store = app.world().resource::<SessionClassStore>();
        assert_eq!(class_completions(store), ["assault", "smg", "sniper"]);
    }
}
