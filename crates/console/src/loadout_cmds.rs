//! The `class` and `spawn` commands: pick one of the class presets, and equip
//! one to go into the match.
//!
//! While the class menu takes changes, `spawn` equips straight away. While it
//! does not, a `spawn` typed in the foreground holds the queue until it does,
//! and one sent to the background is turned down.

use bevy::ecs::message::MessageReader;
use bevy::ecs::system::SystemParam;
use bevy::ecs::system::Res;
use bevy::ecs::system::ResMut;
use net::ActionRequestCounter;
use net::JoinProgressRecord;
use net::JoinSequenceStep;
use ui::loadout_slot_named;
use ui::finalize_loadout_arm;
use ui::LoadoutArmRejection;
use ui::LoadoutPickerFocus;
use ui::LoadoutPickerReport;
use ui::LoadoutPickerStage;
use ui::LoadoutSwapDenial;
use ui::LoadoutSwapPermit;
use ui::MatchLoadoutVault;
use ui::QueuedLoadoutArm;

use crate::app::Hold;
use crate::PromptOutputRow;
use crate::PromptPanelOptions;
use crate::PromptPanelStatus;
use crate::TypedOrder;
use crate::TypedOrderRouter;

/// What `spawn` does, given the menu and where the command was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpawnMove {
    /// The menu takes changes: equip now.
    EquipNow,
    /// Sent in the foreground while the menu is shut: hold for it to open.
    WaitForMenu,
    /// Sent to the background while the menu is shut: say no.
    TurnDown,
}

fn spawn_move(menu_open: bool, in_background: bool) -> SpawnMove {
    match (menu_open, in_background) {
        (true, _) => SpawnMove::EquipNow,
        (false, false) => SpawnMove::WaitForMenu,
        (false, true) => SpawnMove::TurnDown,
    }
}

/// The class menu resources that `class` and `spawn` read and change.
#[derive(SystemParam)]
pub(crate) struct ClassMenu<'w> {
    vault: ResMut<'w, MatchLoadoutVault>,
    focus: ResMut<'w, LoadoutPickerFocus>,
    stage: ResMut<'w, LoadoutPickerStage>,
    queued: ResMut<'w, QueuedLoadoutArm>,
    report: ResMut<'w, LoadoutPickerReport>,
    counter: ResMut<'w, ActionRequestCounter>,
}

impl ClassMenu<'_> {
    /// Asks the host to equip preset `slot`; gives back the request number.
    fn arm(&mut self, slot: usize) -> Result<u32, LoadoutArmRejection> {
        let ClassMenu { vault, focus, stage, queued, report, counter } = self;
        finalize_loadout_arm(slot, vault, focus, stage, queued, report, counter)
    }

    /// The preset slot that `label` picks, if there is one.
    fn slot_of(&self, label: &str) -> Option<usize> {
        loadout_slot_named(&self.vault, label)
    }
}

/// Whether the class menu takes a change now, and if not, why.
#[derive(SystemParam)]
pub(crate) struct MenuGate<'w> {
    permit: Res<'w, LoadoutSwapPermit>,
    denial: Res<'w, LoadoutSwapDenial>,
}

/// Where these commands report: the console's scrollback, the one-line
/// readout and the diagnostics log.
#[derive(SystemParam)]
pub(crate) struct Reply<'w> {
    scrollback: ResMut<'w, PromptPanelStatus>,
    readout: ResMut<'w, PromptOutputRow>,
    panel_options: Res<'w, PromptPanelOptions>,
}

impl Reply<'_> {
    fn post(&mut self, words: impl Into<String>) {
        let words: String = words.into();
        diag::info!(Console, "{}", words);
        self.readout.0.clone_from(&words);
        let keep = self.panel_options.backlog_limit;
        self.scrollback.post_line(words, keep);
    }
}

/// Runs the `class` and `spawn` lines that come in.
pub(crate) fn steer_kit_orders(
    mut orders: MessageReader<TypedOrder>,
    mut menu: ClassMenu,
    mut reply: Reply,
    mut router: ResMut<TypedOrderRouter>,
    gate: MenuGate,
) {
    for order in orders.read() {
        let named = order.words.first().map(String::as_str);
        match order.name.as_str() {
            "class" => class_order(named, &mut menu, &mut reply),
            "spawn" => spawn_order(named, order.detached, &gate, &mut menu, &mut router, &mut reply),
            _ => {}
        }
    }
}

/// `class` by itself lists the presets; `class <name>` picks one.
fn class_order(named: Option<&str>, menu: &mut ClassMenu, reply: &mut Reply) {
    let Some(name) = named else {
        for row in preset_rows(&menu.vault, menu.focus.0) {
            reply.post(row);
        }
        return;
    };
    pick(name, menu, reply);
}

/// `spawn [name]`: equips now, holds the queue for the menu, or says why it
/// cannot, as the menu and the background flag decide.
fn spawn_order(
    named: Option<&str>,
    in_background: bool,
    gate: &MenuGate,
    menu: &mut ClassMenu,
    router: &mut TypedOrderRouter,
    reply: &mut Reply,
) {
    if let Some(name) = named
        && menu.slot_of(name).is_none()
    {
        reply.post(format!("spawn: no preset called `{name}`"));
        return;
    }
    match spawn_move(gate.permit.0, in_background) {
        SpawnMove::EquipNow => arm_preset(named, in_background, menu, router, reply),
        SpawnMove::WaitForMenu => {
            // The preset to equip once the menu opens rides along with the hold.
            router.stall(Hold::SpawnAdmit);
            router.queued_kit_choice = named.map(str::to_owned);
            reply.post("spawn: waiting for the class menu to open");
        }
        SpawnMove::TurnDown => {
            let reason = gate.denial.0.as_deref().unwrap_or("class change not allowed");
            reply.post(format!("spawn: {reason}"));
        }
    }
}

/// Finishes a `spawn` that is holding for the class menu: equips once the
/// menu takes changes, or drops it if joining the server fails first.
pub(crate) fn finish_queued_respawn(
    mut menu: ClassMenu,
    mut reply: Reply,
    mut router: ResMut<TypedOrderRouter>,
    gate: MenuGate,
    join: Option<Res<JoinProgressRecord>>,
) {
    if router.holding() != Some(Hold::SpawnAdmit) {
        return;
    }
    let step = join.as_deref().map(|record| &record.phase);
    let failed_step = step.filter(|step| step.has_failed() || **step == JoinSequenceStep::Cancelled);
    if let Some(step) = failed_step {
        let reason = step.summary_banner().unwrap_or_else(|| "session failed".to_owned());
        router.end_hold();
        router.queued_kit_choice = None;
        reply.post(format!("spawn: given up — {reason}"));
    } else if gate.permit.0 {
        let preset = router.queued_kit_choice.take();
        router.end_hold();
        arm_preset(preset.as_deref(), false, &mut menu, &mut router, &mut reply);
    }
}

/// Makes preset `name` the menu's pick, unless an equip is still on its way.
fn pick(name: &str, menu: &mut ClassMenu, reply: &mut Reply) {
    let Some(slot) = menu.slot_of(name) else {
        reply.post(format!("class: no preset called `{name}` — `class` lists them"));
        return;
    };
    if menu.stage.awaiting_host() {
        reply.post("class: an equip is already waiting on the host");
        return;
    }
    menu.focus.0 = slot;
    menu.vault.selected = slot;
    reply.post(format!("class: picked {}", preset_line(&menu.vault, slot)));
}

/// Equips preset `named`, or the menu's pick when no name is given. Sent from
/// the foreground, it then holds the queue until the player is in the game.
fn arm_preset(
    named: Option<&str>,
    in_background: bool,
    menu: &mut ClassMenu,
    router: &mut TypedOrderRouter,
    reply: &mut Reply,
) {
    let slot = match named.map(|name| (name, menu.slot_of(name))) {
        None => menu.focus.0,
        Some((_, Some(found))) => found,
        Some((name, None)) => {
            reply.post(format!("spawn: no preset called `{name}`"));
            return;
        }
    };
    let ticket = match menu.arm(slot) {
        Ok(ticket) => ticket,
        Err(turned_down) => {
            reply.post(format!("spawn: {turned_down}"));
            return;
        }
    };
    let label = preset_line(&menu.vault, slot);
    if in_background {
        reply.post(format!("spawn &: equipping {label} (request {ticket})"));
    } else {
        router.stall(Hold::Spawn);
        reply.post(format!("spawn: equipping {label} (request {ticket}), waiting until in game"));
    }
}

/// One preset on one line: its slot, name and weapons, and why it is locked
/// when it is.
fn preset_line(vault: &MatchLoadoutVault, slot: usize) -> String {
    let Some(preset) = vault.slots.get(slot) else {
        return format!("{slot} (no such preset)");
    };
    let weapons = format!("primary={} secondary={}", preset.primary, preset.secondary);
    let line = format!("{slot} `{}` {weapons}", preset.name);
    match &preset.frozen_because {
        Some(lock) => line + " LOCKED — " + lock,
        None => line,
    }
}

/// What a bare `class` prints: a heading row, then a row for each preset
/// with `>` in front of the pick.
fn preset_rows(vault: &MatchLoadoutVault, pick: usize) -> Vec<String> {
    let total = vault.slots.len();
    let equipped = match vault.equipped {
        Some(slot) => slot.to_string(),
        None => "none".to_owned(),
    };
    let heading = format!("class: {total} presets, picked {pick}, equipped {equipped}");
    let presets = (0..total).map(|slot| {
        let marker = if slot == pick { '>' } else { ' ' };
        format!("{marker} {}", preset_line(vault, slot))
    });
    std::iter::once(heading).chain(presets).collect::<Vec<String>>()
}

/// The preset names, offered while `class` and `spawn` are typed.
pub fn class_completions(store: &MatchLoadoutVault) -> Vec<String> {
    let mut names = Vec::with_capacity(store.slots.len());
    names.extend(store.slots.iter().map(|preset| preset.name.clone()));
    names
}

#[cfg(test)]
mod tests {
    use bevy::app::{App, Update};
    use bevy::ecs::schedule::IntoScheduleConfigs;
    use frame::ServerKitSlot;
    use ui::LoadoutSlotStatus;

    use crate::loadout_cmds::*;
    use crate::tool_cmds::testing::{console_app, feed, said};

    fn preset(name: &str, lock: Option<&str>) -> LoadoutSlotStatus {
        let mut slot = LoadoutSlotStatus::from_host_slot(&ServerKitSlot {
            name: String::from(name),
            primary: format!("{name}_gun"),
            main_gun_parts: Vec::new(),
            secondary: format!("{name}_pistol"),
            side_gun_parts: Vec::new(),
            lethal: String::new(),
            tactical: String::new(),
            perks: Default::default(),
        });
        slot.frozen_because = lock.map(str::to_owned);
        slot
    }

    fn app() -> App {
        let mut app = console_app();
        let presets = [("assault", None), ("smg", None), ("sniper", Some("rank 20"))];
        let mut store = MatchLoadoutVault::default();
        store.slots = presets.into_iter().map(|(name, lock)| preset(name, lock)).collect();
        // The class menu starts closed, nothing picked or pending.
        let world = app.world_mut();
        world.insert_resource(store);
        world.insert_resource(LoadoutPickerFocus::default());
        world.insert_resource(QueuedLoadoutArm::default());
        world.insert_resource(LoadoutPickerReport::default());
        world.insert_resource(LoadoutSwapPermit::default());
        world.insert_resource(LoadoutSwapDenial::default());
        world.init_resource::<LoadoutPickerStage>();
        world.init_resource::<ActionRequestCounter>();
        world.init_resource::<TypedOrderRouter>();
        app.add_systems(Update, (steer_kit_orders, finish_queued_respawn).chain());
        app
    }

    fn open_menu(app: &mut App) {
        app.world_mut().insert_resource(LoadoutSwapPermit(true));
    }

    fn holding(app: &App) -> Option<Hold> {
        app.world().resource::<TypedOrderRouter>().holding()
    }

    fn picked(app: &App) -> usize {
        app.world().resource::<LoadoutPickerFocus>().0
    }

    fn requested(app: &App) -> Option<usize> {
        if let LoadoutPickerStage::Pending { category_slot, .. } = app.world().resource::<LoadoutPickerStage>() {
            Some(*category_slot)
        } else {
            None
        }
    }

    #[test]
    fn the_menu_decides_what_spawn_does() {
        assert_eq!(spawn_move(true, false), SpawnMove::EquipNow);
        assert_eq!(spawn_move(true, true), SpawnMove::EquipNow);
        assert_eq!(spawn_move(false, false), SpawnMove::WaitForMenu);
        assert_eq!(spawn_move(false, true), SpawnMove::TurnDown);
    }

    #[test]
    fn class_lists_the_presets_and_marks_the_picked_one() {
        let mut app = app();
        feed(&mut app, "class");
        let expected = [
            "class: 3 presets, picked 0, equipped none",
            "> 0 `assault` primary=assault_gun secondary=assault_pistol",
            "  1 `smg` primary=smg_gun secondary=smg_pistol",
            "  2 `sniper` primary=sniper_gun secondary=sniper_pistol LOCKED — rank 20",
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn class_names_a_preset_by_name_in_any_case_or_by_number() {
        let mut app = app();
        feed(&mut app, "class SMG");
        assert_eq!(picked(&app), 1);
        assert_eq!(app.world().resource::<MatchLoadoutVault>().selected, 1);
        feed(&mut app, "class 0");
        assert_eq!(picked(&app), 0);
        assert_eq!(app.world().resource::<MatchLoadoutVault>().selected, 0);
        let expected = [
            "class: picked 1 `smg` primary=smg_gun secondary=smg_pistol",
            "class: picked 0 `assault` primary=assault_gun secondary=assault_pistol",
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn class_says_when_no_preset_has_the_name() {
        let mut app = app();
        feed(&mut app, "class knife");
        let expected = ["class: no preset called `knife` — `class` lists them"];
        assert_eq!(said(&app), expected);
        assert_eq!(picked(&app), 0);
    }

    #[test]
    fn class_waits_while_an_equip_is_on_its_way() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn smg &");
        feed(&mut app, "class assault");
        assert_eq!(picked(&app), 1);
        let expected = Some("class: an equip is already waiting on the host");
        assert_eq!(said(&app).last().map(String::as_str), expected);
    }

    #[test]
    fn spawn_with_a_menu_that_allows_it_equips_and_holds_the_queue() {
        let mut app = app();
        open_menu(&mut app);
        feed(&mut app, "spawn smg");
        assert_eq!(holding(&app), Some(Hold::Spawn));
        assert_eq!(requested(&app), Some(1));
        assert_eq!(picked(&app), 1);
        assert!(app.world().resource::<QueuedLoadoutArm>().0.is_some());
        let actual = said(&app);
        assert_eq!(actual, [
            "spawn: equipping 1 `smg` primary=smg_gun secondary=smg_pistol (request 1), waiting until in game"
        ]);
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
        let actual = said(&app);
        assert_eq!(actual, [
            "spawn &: equipping 0 `assault` primary=assault_gun secondary=assault_pistol (request 1)"
        ]);
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
        let expected = Some("spawn: equip already pending (request_id=1)");
        assert_eq!(said(&app).last().map(String::as_str), expected);
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
        let expected = Some("smg".to_owned());
        assert_eq!(app.world()
            .resource::<TypedOrderRouter>()
            .queued_kit_choice, expected);
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
        let dispatch = app.world().resource::<TypedOrderRouter>();
        assert_eq!(dispatch.queued_kit_choice, None);
    }

    #[test]
    fn a_spawn_that_waited_gives_up_when_the_session_fails() {
        let mut app = app();
        feed(&mut app, "spawn smg");
        app.insert_resource(JoinProgressRecord {
            phase: JoinSequenceStep::Cancelled,
            ..Default::default()
        });
        app.update();
        assert_eq!(holding(&app), None);
        let expected = None;
        assert_eq!(app.world()
            .resource::<TypedOrderRouter>()
            .queued_kit_choice, expected);
        assert_eq!(requested(&app), None);
        let expected = Some("spawn: given up — signon cancelled");
        assert_eq!(said(&app).last().map(String::as_str), expected);
    }

    #[test]
    fn a_background_spawn_before_the_menu_opens_says_why_not() {
        let mut app = app();
        feed(&mut app, "spawn &");
        assert_eq!(holding(&app), None);
        assert_eq!(said(&app), ["spawn: class change not allowed"]);

        app.world_mut().insert_resource(LoadoutSwapDenial(Some(
            "waiting for host: loading map".to_owned(),
        )));
        feed(&mut app, "spawn &");
        let expected = Some("spawn: waiting for host: loading map");
        assert_eq!(said(&app).last().map(String::as_str), expected);
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
        let store = app.world().resource::<MatchLoadoutVault>();
        assert_eq!(class_completions(store), ["assault", "smg", "sniper"]);
    }
}
