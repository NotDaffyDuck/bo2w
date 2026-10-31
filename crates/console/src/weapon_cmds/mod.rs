//! Console commands for weapons. `give` equips a weapon from the catalog or
//! hands out a killstreak, `attach` lists or flips the attachments on the
//! weapon being held, and the prompt can complete the names both take.

mod catalog;

use std::{sync::Arc, sync::RwLock};

use assets::StagedWeapons;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use frame::GameSessionClosed;
use net::{DumpConfigurationChangeLog, GrantLogDumper, HostViewClient, ShownWorldFrame};
use sim::{PlayerIntent, menu_response_field};

pub(crate) use catalog::resolve_give_id;
pub use catalog::attach_completions;
pub use catalog::weapon_completions;
use catalog::{attachment_hints, toggled_id};

use crate::tool_cmds::PromptReplyWriter;
use crate::host_link::HostLink;
use crate::ParamSuggester;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::OrderCatalog;
use crate::FixedListSuggester;

const GIVE_USAGE: &str = "give <game:weapon> [attachment...] | give killstreak <name> — equip a \
                          weapon on the active slot (e.g. give t6:an94_mp reflex), or grant a \
                          killstreak (e.g. give killstreak airdrop)";
const ATTACH_USAGE: &str =
    "attach [name] — show the current set and choices, or toggle a named attachment";
const GIVE_BARE_USAGE: &str =
    "usage: give <weapon> [attachment...] — equip a catalog weapon on the active slot";
const KILLSTREAK_USAGE: &str =
    "usage: give killstreak <name> (e.g. airdrop, uav, predator_missile)";

/// Candidate names shown at the prompt for `give` and `attach`. A system
/// ([`rebuild_gun_hints`]) keeps both lists current, and the
/// completers registered with the commands read them.
#[derive(Clone, Default, Resource)]
pub struct GunParamHints {
    pub gun_names: Arc<RwLock<Vec<String>>>,
    pub attachment_names: Arc<RwLock<Vec<String>>>,
}

impl GunParamHints {
    fn replace(target: &Arc<RwLock<Vec<String>>>, fresh: Vec<String>) {
        let Ok(mut slot) = target.write() else {
            return;
        };
        *slot = fresh;
    }
}

/// Completes an argument from a shared list that another system maintains.
struct SharedNameHints(Arc<RwLock<Vec<String>>>);

impl ParamSuggester for SharedNameHints {
    fn suggest(&self, typed: &str) -> Vec<String> {
        let Ok(names) = self.0.read() else {
            return vec![];
        };
        FixedListSuggester::new(names.iter().cloned()).suggest(typed)
    }
}

pub(crate) fn install_gun_orders(
    book: &mut OrderCatalog,
    completions: &GunParamHints,
) {
    let table = [
        ("give", GIVE_USAGE, &completions.gun_names),
        ("attach", ATTACH_USAGE, &completions.attachment_names),
    ];
    for (name, usage, list) in table {
        if book.look_up(name).is_some() {
            continue;
        }
        let completer = SharedNameHints(Arc::clone(list));
        book.enlist(OrderBlueprint::new(name).help_line(usage).arg(completer));
    }
}

/// When a match is torn down, the weapon names of its catalog go too.
pub(crate) fn drop_gun_hints_on_close(
    mut torn: MessageReader<GameSessionClosed>,
    completions: Res<GunParamHints>,
) {
    let closed = torn.read().count();
    if closed != 0 {
        for list in [&completions.gun_names, &completions.attachment_names] {
            GunParamHints::replace(list, vec![]);
        }
    }
}

/// Rebuilds both lists when the catalog changes or the held weapon does.
pub(crate) fn rebuild_gun_hints(
    weapons: Option<Res<StagedWeapons>>,
    presented: Res<ShownWorldFrame>,
    local: Res<HostViewClient>,
    completions: Res<GunParamHints>,
    mut listed_for: Local<Option<u32>>,
) {
    let Some(catalog) = weapons else {
        return;
    };
    let in_hand = presented.held_weapon(local.0).unwrap_or(0);
    if *listed_for == Some(in_hand) && !catalog.is_changed() {
        return;
    }
    *listed_for = Some(in_hand);
    GunParamHints::replace(&completions.gun_names, weapon_completions(&catalog));
    let attachments = match in_hand {
        0 => Vec::new(),
        id => attach_completions(&catalog, id),
    };
    GunParamHints::replace(&completions.attachment_names, attachments);
}

#[derive(SystemParam)]
pub(crate) struct Arms<'w> {
    staged: Option<Res<'w, StagedWeapons>>,
    shown: Res<'w, ShownWorldFrame>,
    link: HostLink<'w>,
    echo: PromptReplyWriter<'w>,
}

impl Arms<'_> {
    /// Id of the weapon the local player holds, or an error when not alive.
    fn alive_weapon(&self, verb: &str) -> Result<u32, String> {
        match self.shown.held_weapon(self.link.client()) {
            Some(id) => Ok(id),
            None => Err(format!("{verb}: not Alive — spawn a class first")),
        }
    }

    fn give(&mut self, args: &[String]) -> Result<String, String> {
        let Some((first, extras)) = args.split_first() else {
            return Err(GIVE_BARE_USAGE.into());
        };
        if first == "killstreak" {
            return self.give_killstreak(args.get(1));
        }
        let catalog = &self
            .staged
            .as_ref()
            .ok_or("give: weapon catalog not loaded")?
            .0;
        self.alive_weapon("give")?;
        let weapon = resolve_give_id(catalog, first, extras).map_err(|why| format!("give: {why}"))?;
        let ticket = self
            .link
            .queue("give", |serial| PlayerIntent::HandOutGun {
                request_id: serial,
                gun: weapon,
            })?;
        let label = catalog.configuration_label(weapon);
        let answer = format!("give: queued {label} id={weapon} request_id={ticket}");
        Ok(answer)
    }

    fn give_killstreak(&mut self, streak: Option<&String>) -> Result<String, String> {
        let streak = streak.ok_or(KILLSTREAK_USAGE)?;
        self.alive_weapon("give killstreak")?;
        let Some(field) = menu_response_field(streak) else {
            return Err(format!("give killstreak: `{streak}` is too long"));
        };
        let ticket = self.link.queue("give killstreak", |serial| {
            PlayerIntent::GiveKillstreak {
                request_id: serial,
                name: field,
            }
        })?;
        let answer = format!("give killstreak: queued {streak} request_id={ticket}");
        Ok(answer)
    }

    fn attach(&mut self, args: &[String]) -> Result<String, String> {
        let catalog = &self
            .staged
            .as_ref()
            .ok_or("attach: weapon catalog not loaded")?
            .0;
        let held = self.alive_weapon("attach")?;
        if held == 0 {
            return Err("attach: no weapon in hands".into());
        }
        let Some(wanted) = args.first() else {
            let hints = attachment_hints(catalog, held).map_err(|why| format!("attach: {why}"))?;
            return Ok(format!("attach: {hints}"));
        };
        let target = toggled_id(catalog, held, wanted).map_err(|why| format!("attach: {why}"))?;
        if target == held {
            let label = catalog.configuration_label(held);
            let unchanged = format!(
                "attach: `{label}` is already the selected configuration — refused (an unchanged id \
                 is not a successful attach)"
            );
            return Err(unchanged);
        }
        let ticket = self.link.queue("attach", |serial| {
            PlayerIntent::ChangeWeaponConfiguration {
                request_id: serial,
                from: held,
                to: target,
            }
        })?;
        let label = catalog.configuration_label(target);
        let answer = format!("attach: queued {label} id={target} request_id={ticket}");
        Ok(answer)
    }
}

pub(crate) fn steer_gun_orders(mut commands: MessageReader<TypedOrder>, mut arms: Arms) {
    for order in commands.read() {
        let outcome = match order.name.as_str() {
            "give" => arms.give(&order.words),
            "attach" => arms.attach(&order.words),
            _ignored => continue,
        };
        let (Ok(line) | Err(line)) = outcome;
        arms.echo.write(line);
    }
}

/// The catalog's label for a weapon id; `#<id>` when the catalog has none.
fn label_for(weapons: Option<&StagedWeapons>, id: u32) -> String {
    let named = weapons
        .map(|catalog| catalog.0.configuration_label(id))
        .filter(|label| !label.is_empty());
    named.unwrap_or_else(|| format!("#{id}"))
}

fn host_reply(
    verb: &str,
    label_text: &str,
    id: u32,
    serial: u32,
    host_said: Result<(), &str>,
) -> String {
    let tail = format!("{label_text} id={id} request_id={serial}");
    match host_said {
        Ok(()) => format!("{verb}: ok {tail}"),
        Err(why) => format!("{verb}: rejected {tail} ({why})"),
    }
}

/// Reports the host's answer to the latest `give`, one time only.
pub(crate) fn report_handout_outcome(
    give: Option<Res<GrantLogDumper>>,
    weapons: Option<Res<StagedWeapons>>,
    mut echo: PromptReplyWriter,
    mut said_last: Local<Option<(u32, u8)>>,
) {
    let Some(log) = give else {
        return;
    };
    let (Some(serial), Some(gun), Some(verdict)) =
        (log.ask_id, log.weapon, log.accepted)
    else {
        return;
    };
    let this_one = Some((serial, verdict));
    if *said_last == this_one {
        return;
    }
    *said_last = this_one;
    let result = match verdict {
        1 => Ok(()),
        _ => Err(log.refusal_cause.unwrap_or("unknown")),
    };
    let label = label_for(weapons.as_deref(), gun);
    echo.write(host_reply("give", &label, gun, serial, result));
}

/// Reports the host's answer to the latest `attach`, one time only.
pub(crate) fn echo_configuration_change_results(
    changes: Option<Res<DumpConfigurationChangeLog>>,
    weapons: Option<Res<StagedWeapons>>,
    mut echo: PromptReplyWriter,
    mut echoed: Local<Option<u32>>,
) {
    let Some(log) = changes else {
        return;
    };
    let (Some(serial), Some(to), Some(accepted)) = (log.ask_id, log.to, log.accepted)
    else {
        return;
    };
    if *echoed == Some(serial) {
        return;
    }
    *echoed = Some(serial);
    let result = match accepted {
        true => Ok(()),
        false => Err(log.refusal_cause.unwrap_or("unknown")),
    };
    let label = label_for(weapons.as_deref(), to);
    echo.write(host_reply("attach", &label, to, serial, result));
}

#[cfg(test)]
mod tests {
    use frame::GameSessionStamp;
    use frame::ShutdownCause;
    use frame::LevelGeneration;

    use crate::weapon_cmds::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use crate::host_link::testing::{FULL, host_app, present_holding, present_player, queued};
    use bevy::app::Update;

    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<ShownWorldFrame>()
            .add_systems(Update, steer_gun_orders);
        app
    }

    fn with_catalog() -> App {
        let mut app = app();
        app.init_resource::<StagedWeapons>();
        app
    }

    fn stand(app: &mut App, alive: bool) {
        present_player(app, [0.0; 3], [0.0; 3], alive, "me");
    }

    fn airdrop() -> [u8; 48] {
        menu_response_field("airdrop").expect("fits the field")
    }

    #[test]
    fn give_with_nothing_after_it_says_how_it_is_used() {
        let mut app = app();
        feed(&mut app, "give");
        let expected = ["usage: give <weapon> [attachment...] — equip a catalog weapon on the active slot"];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn other_commands_are_not_theirs() {
        let mut app = with_catalog();
        feed(&mut app, "echo hello; giveaway; attached");
        assert!(said(&app).is_empty());
    }

    #[test]
    fn give_needs_the_catalog_before_anything_else() {
        let mut app = app();
        stand(&mut app, true);
        feed(&mut app, "give t6:an94");
        assert_eq!(said(&app), ["give: weapon catalog not loaded"]);
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn give_needs_a_player_who_is_alive() {
        let mut app = with_catalog();
        feed(&mut app, "give t6:an94");
        stand(&mut app, false);
        feed(&mut app, "give t6:an94");
        assert_eq!(said(&app), ["give: not Alive — spawn a class first"; 2]);
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn give_names_a_weapon_the_catalog_does_not_have() {
        let mut app = with_catalog();
        stand(&mut app, true);
        feed(&mut app, "give an94; give an94 reflex acog");
        assert_eq!(said(&app), ["give: unknown weapon `an94`"; 2]);
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn killstreak_with_no_name_says_how_it_is_used() {
        let mut app = app();
        feed(&mut app, "give killstreak");
        let expected = ["usage: give killstreak <name> (e.g. airdrop, uav, predator_missile)"];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn a_killstreak_is_queued_by_name_with_the_next_request_number() {
        let mut app = app();
        stand(&mut app, true);
        feed(&mut app, "give killstreak airdrop; give killstreak airdrop");
        let expected = [
            "give killstreak: queued airdrop request_id=1",
            "give killstreak: queued airdrop request_id=2"
        ];
        assert_eq!(said(&app), expected);
        const FIRST: u32 = 1;
        let actual = queued(&app);
        assert_eq!(actual, [
            PlayerIntent::GiveKillstreak {
                request_id: FIRST,
                name: airdrop()
            },
            PlayerIntent::GiveKillstreak {
                request_id: 2,
                name: airdrop()
            }
        ]);
    }

    #[test]
    fn a_killstreak_needs_a_player_who_is_alive() {
        let mut app = app();
        stand(&mut app, false);
        feed(&mut app, "give killstreak airdrop");
        let expected = ["give killstreak: not Alive — spawn a class first"];
        assert_eq!(said(&app), expected);
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn a_killstreak_name_longer_than_the_field_is_refused() {
        let mut app = app();
        stand(&mut app, true);
        let long = "x".repeat(49);
        feed(&mut app, &format!("give killstreak {long}"));
        let expected = [format!("give killstreak: `{long}` is too long")];
        assert_eq!(said(&app), expected);
        assert!(queued(&app).is_empty());
        feed(&mut app, &format!("give killstreak {}", "x".repeat(48)));
        assert_eq!(queued(&app).len(), 1, "the field's full width still fits");
    }

    #[test]
    fn a_killstreak_asked_of_a_full_queue_says_so() {
        let mut app = app();
        stand(&mut app, true);
        for _ in 0..FULL {
            feed(&mut app, "give killstreak airdrop");
        }
        feed(&mut app, "give killstreak airdrop");
        let expected = Some("give killstreak: action queue full (32 unresolved requests)");
        assert_eq!(said(&app).last().map(String::as_str), expected);
        assert_eq!(queued(&app).len(), FULL);
    }

    #[test]
    fn attach_needs_the_catalog() {
        let mut app = app();
        present_holding(&mut app, 7);
        feed(&mut app, "attach; attach reflex");
        assert_eq!(said(&app), ["attach: weapon catalog not loaded"; 2]);
    }

    #[test]
    fn attach_needs_a_player_who_is_alive() {
        let mut app = with_catalog();
        stand(&mut app, false);
        feed(&mut app, "attach reflex");
        assert_eq!(said(&app), ["attach: not Alive — spawn a class first"]);
    }

    #[test]
    fn attach_needs_something_in_the_hands() {
        let mut app = with_catalog();
        stand(&mut app, true);
        feed(&mut app, "attach");
        assert_eq!(said(&app), ["attach: no weapon in hands"]);
    }

    #[test]
    fn attach_on_a_weapon_that_belongs_to_no_family_says_so() {
        let mut app = with_catalog();
        present_holding(&mut app, 7);
        feed(&mut app, "attach; attach reflex");
        assert_eq!(said(&app), ["attach: `` belongs to no weapon family"; 2]);
        assert!(queued(&app).is_empty());
    }

    fn echo_app() -> App {
        let mut app = console_app();
        let systems = (report_handout_outcome, echo_configuration_change_results).chain();
        app.add_systems(bevy::app::Update, systems);
        app
    }

    fn given(gun: u32, serial: u32, accepted: u8, why: Option<&'static str>) -> GrantLogDumper {
        GrantLogDumper {
            ask_id: Some(serial),
            weapon: Some(gun),
            accepted: Some(accepted),
            refusal_cause: why,
        }
    }

    fn changed(
        to: u32,
        serial: u32,
        accepted: bool,
        why: Option<&'static str>,
    ) -> DumpConfigurationChangeLog {
        DumpConfigurationChangeLog {
            ask_id: Some(serial),
            from: Some(1),
            to: Some(to),
            accepted: Some(accepted),
            refusal_cause: why,
        }
    }

    #[test]
    fn the_hosts_answer_to_a_give_is_said_once_under_the_weapons_number() {
        let mut app = echo_app();
        app.insert_resource(given(9, 4, 1, None));
        app.update();
        app.update();
        assert_eq!(said(&app), ["give: ok #9 id=9 request_id=4"]);
    }

    #[test]
    fn an_empty_catalog_has_no_name_for_the_weapon_either() {
        let mut app = echo_app();
        app.init_resource::<StagedWeapons>()
            .insert_resource(given(9, 4, 0, Some("no_slot")));
        app.update();
        let expected = ["give: rejected #9 id=9 request_id=4 (no_slot)"];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn a_give_that_flips_from_one_answer_to_the_other_is_said_again() {
        let mut app = echo_app();
        app.insert_resource(given(9, 4, 0, None));
        app.update();
        app.insert_resource(given(9, 4, 1, None));
        app.update();
        app.insert_resource(given(9, 5, 1, None));
        app.update();
        let expected = [
            "give: rejected #9 id=9 request_id=4 (unknown)",
            "give: ok #9 id=9 request_id=4",
            "give: ok #9 id=9 request_id=5"
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn a_give_log_with_a_part_missing_is_not_said() {
        let mut app = echo_app();
        app.update();
        assert!(said(&app).is_empty(), "no log at all");
        for hole in 0..3 {
            let mut log = given(9, 4, 1, None);
            match hole {
                0 => log.ask_id = None,
                1 => log.weapon = None,
                _ => log.accepted = None,
            }
            app.insert_resource(log);
            app.update();
        }
        assert!(said(&app).is_empty());
    }

    #[test]
    fn the_hosts_answer_to_an_attach_is_said_once_per_request() {
        let mut app = echo_app();
        app.insert_resource(changed(12, 3, true, None));
        app.update();
        app.insert_resource(changed(12, 3, false, Some("late")));
        app.update();
        app.insert_resource(changed(13, 4, false, Some("late")));
        app.update();
        app.insert_resource(changed(14, 5, false, None));
        app.update();
        let expected = [
            "attach: ok #12 id=12 request_id=3",
            "attach: rejected #13 id=13 request_id=4 (late)",
            "attach: rejected #14 id=14 request_id=5 (unknown)"
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn an_attach_log_with_a_part_missing_is_not_said() {
        let mut app = echo_app();
        for hole in 0..3 {
            let mut log = changed(12, 3, true, None);
            match hole {
                0 => log.ask_id = None,
                1 => log.to = None,
                _ => log.accepted = None,
            }
            app.insert_resource(log);
            app.update();
        }
        assert!(said(&app).is_empty());
    }

    fn lists_app() -> App {
        let mut app = host_app();
        let chained = (drop_gun_hints_on_close, rebuild_gun_hints).chain();
        app.init_resource::<ShownWorldFrame>()
            .init_resource::<GunParamHints>()
            .add_message::<GameSessionClosed>()
            .add_systems(bevy::app::Update, chained);
        app
    }

    fn listed(list: &Arc<RwLock<Vec<String>>>) -> Vec<String> {
        list.read().expect("nobody panicked holding it").clone()
    }

    fn stale(app: &App) {
        let lists = app.world().resource::<GunParamHints>();
        GunParamHints::replace(&lists.gun_names, vec!["t6:old".to_owned()]);
        GunParamHints::replace(&lists.attachment_names, vec!["old".to_owned()]);
    }

    fn lists(app: &App) -> [Vec<String>; 2] {
        let lists = app.world().resource::<GunParamHints>();
        [listed(&lists.gun_names), listed(&lists.attachment_names)]
    }

    #[test]
    fn with_no_catalog_the_lists_are_left_alone() {
        let mut app = lists_app();
        stale(&app);
        app.update();
        let expected = [vec!["t6:old".to_owned()], vec!["old".to_owned()]];
        assert_eq!(lists(&app), expected);
    }

    #[test]
    fn a_catalog_that_arrives_replaces_the_lists_and_then_they_are_left_alone() {
        let mut app = lists_app();
        stale(&app);
        app.init_resource::<StagedWeapons>();
        app.update();
        assert!(lists(&app).iter().all(Vec::is_empty));
        stale(&app);
        app.update();
        let expected = [vec!["t6:old".to_owned()], vec!["old".to_owned()]];
        assert_eq!(lists(&app), expected);
    }

    #[test]
    fn a_new_weapon_in_hand_rebuilds_the_lists() {
        let mut app = lists_app();
        app.init_resource::<StagedWeapons>();
        app.update();
        stale(&app);
        present_holding(&mut app, 7);
        app.update();
        assert!(lists(&app).iter().all(Vec::is_empty));
        stale(&app);
        app.update();
        assert_eq!(lists(&app)[0], ["t6:old"], "the same weapon changes nothing");
    }

    #[test]
    fn a_match_coming_down_empties_the_lists() {
        let mut app = lists_app();
        stale(&app);
        app.update();
        assert_eq!(lists(&app)[0], ["t6:old"], "no message, nothing cleared");
        app.world_mut().write_message(GameSessionClosed {
            reason: ShutdownCause::Disconnect,
            level_generation: LevelGeneration::default(),
            game_session_key: GameSessionStamp::default(),
            game_generation: 0,
        });
        app.update();
        assert!(lists(&app).iter().all(Vec::is_empty));
    }

    fn registered() -> (OrderCatalog, GunParamHints) {
        let completions = GunParamHints::default();
        let mut book = OrderCatalog::default();
        install_gun_orders(&mut book, &completions);
        (book, completions)
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn both_commands_are_registered_with_their_usage() {
        let (book, _) = registered();
        let give = book.look_up("give").expect("give is registered");
        assert!(
            give.help_line
                .starts_with("give <game:weapon> [attachment...] | give killstreak <name>")
        );
        assert!(give.help_line.ends_with("(e.g. give killstreak airdrop)"));
        let expected = "attach [name] — show the current set and choices, or toggle a named attachment";
        assert_eq!(book
            .look_up("attach")
            .expect("attach is registered")
            .help_line, expected);
    }

    #[test]
    fn registering_twice_or_over_a_command_that_is_there_changes_nothing() {
        let (mut book, completions) = registered();
        install_gun_orders(&mut book, &completions);
        let expected = 1;
        assert_eq!(book
            .all_names()
            .iter()
            .filter(|name| *name == "give")
            .count(), expected);

        let mut mine = OrderCatalog::default();
        mine.enlist(OrderBlueprint::new("give").help_line("mine"));
        install_gun_orders(&mut mine, &completions);
        assert_eq!(mine.look_up("give").expect("still there").help_line, "mine");
        assert!(mine.look_up("attach").is_some());
    }

    #[test]
    fn the_prompt_completes_from_the_lists_as_they_stand() {
        let (book, completions) = registered();
        assert!(book.hint_list("give t6:a", 9).is_empty());
        GunParamHints::replace(
            &completions.gun_names,
            names(&["t6:an94", "t6:ak47", "mp:ak74u"]),
        );
        GunParamHints::replace(&completions.attachment_names, names(&["reflex", "acog"]));
        assert_eq!(book.hint_list("give t6:a", 9), ["t6:an94", "t6:ak47"]);
        assert_eq!(book.hint_list("give ", 5).len(), 3);
        assert_eq!(book.hint_list("attach RE", 9), ["reflex"]);
        GunParamHints::replace(&completions.gun_names, names(&["t6:fal"]));
        assert_eq!(book.hint_list("give t6:", 8), ["t6:fal"]);
    }
}
