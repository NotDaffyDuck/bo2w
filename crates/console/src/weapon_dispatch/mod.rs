//! The weapon commands: `give` equips a catalog weapon or grants a killstreak,
//! `attach` shows or toggles the attachments on the weapon in hand, and the
//! prompt completes the names they take.

mod catalog;

use std::sync::{Arc, RwLock};

use assets::PreparedWeapons;
use bevy::{ecs::system::SystemParam, prelude::*};
use frame::MatchTornDown;
use net::{DumpConfigurationChangeLog, DumpGiveLog, LocalPresentClient, PresentedSnapshot};
use sim::{ClientAction, menu_response_field};

pub(crate) use catalog::resolve_give_id;
pub use catalog::{attach_completions, weapon_completions};
use catalog::{attachment_hints, toggled_id};

use crate::feature_dispatch::ConsoleEcho;
use crate::host_link::HostLink;
use crate::{ArgCompleter, CommandSpec, ConsoleCommand, ConsoleRegistry, StaticCompleter};

const GIVE_USAGE: &str = "give <game:weapon> [attachment...] | give killstreak <name> — equip a \
                          weapon on the active slot (e.g. give t6:an94_mp reflex), or grant a \
                          killstreak (e.g. give killstreak airdrop)";
const ATTACH_USAGE: &str =
    "attach [name] — show the current set and choices, or toggle a named attachment";

type NameList = Arc<RwLock<Vec<String>>>;

/// The names the prompt offers for `give` and for `attach`. The lists are kept
/// current by [`refresh_weapon_arg_completions`] and read by the completers the
/// commands were registered with.
#[derive(Resource, Clone, Default)]
pub struct WeaponArgCompletions {
    pub give: NameList,
    pub attach: NameList,
}

impl WeaponArgCompletions {
    fn replace(list: &NameList, names: Vec<String>) {
        if let Ok(mut held) = list.write() {
            *held = names;
        }
    }
}

/// Completes an argument from a list somebody else keeps up to date.
struct LiveListCompleter(NameList);

impl ArgCompleter for LiveListCompleter {
    fn complete(&self, prefix: &str) -> Vec<String> {
        match self.0.read() {
            Ok(names) => StaticCompleter::new(names.iter().cloned()).complete(prefix),
            Err(_) => Vec::new(),
        }
    }
}

pub(crate) fn register_weapon_commands(
    registry: &mut ConsoleRegistry,
    completions: &WeaponArgCompletions,
) {
    let commands = [
        ("give", GIVE_USAGE, &completions.give),
        ("attach", ATTACH_USAGE, &completions.attach),
    ];
    for (name, usage, names) in commands {
        if registry.resolve(name).is_none() {
            registry.register(
                CommandSpec::new(name)
                    .usage(usage)
                    .arg(LiveListCompleter(Arc::clone(names))),
            );
        }
    }
}

/// A match going away takes the weapons of its catalog with it.
pub(crate) fn clear_weapon_args_on_torn_down(
    mut torn: MessageReader<MatchTornDown>,
    completions: Res<WeaponArgCompletions>,
) {
    if torn.read().count() == 0 {
        return;
    }
    WeaponArgCompletions::replace(&completions.give, Vec::new());
    WeaponArgCompletions::replace(&completions.attach, Vec::new());
}

/// Rebuilds the lists when the catalog changes or the weapon in hand does.
pub(crate) fn refresh_weapon_arg_completions(
    weapons: Option<Res<PreparedWeapons>>,
    presented: Res<PresentedSnapshot>,
    local: Res<LocalPresentClient>,
    completions: Res<WeaponArgCompletions>,
    mut listed_for: Local<Option<u32>>,
) {
    let Some(weapons) = weapons else {
        return;
    };
    let held = presented.held_weapon_id(local.0).unwrap_or(0);
    if !weapons.is_changed() && *listed_for == Some(held) {
        return;
    }
    *listed_for = Some(held);
    WeaponArgCompletions::replace(&completions.give, weapon_completions(&weapons));
    let attach = if held == 0 {
        Vec::new()
    } else {
        attach_completions(&weapons, held)
    };
    WeaponArgCompletions::replace(&completions.attach, attach);
}

#[derive(SystemParam)]
pub(crate) struct Arms<'w> {
    weapons: Option<Res<'w, PreparedWeapons>>,
    presented: Res<'w, PresentedSnapshot>,
    link: HostLink<'w>,
    echo: ConsoleEcho<'w>,
}

impl Arms<'_> {
    /// The weapon in the local player's hands, if they are alive.
    fn alive_weapon(&self, verb: &str) -> Result<u32, String> {
        self.presented
            .held_weapon_id(self.link.client())
            .ok_or_else(|| format!("{verb}: not Alive — spawn a class first"))
    }

    fn give(&mut self, args: &[String]) -> Result<String, String> {
        let Some((raw, attachments)) = args.split_first() else {
            return Err(
                "usage: give <weapon> [attachment...] — equip a catalog weapon on the active slot"
                    .into(),
            );
        };
        if raw == "killstreak" {
            return self.give_killstreak(args.get(1));
        }
        let registry = &self
            .weapons
            .as_ref()
            .ok_or("give: weapon catalog not loaded")?
            .0;
        self.alive_weapon("give")?;
        let weapon =
            resolve_give_id(registry, raw, attachments).map_err(|why| format!("give: {why}"))?;
        let request_id = self
            .link
            .queue("give", |request_id| ClientAction::GiveWeapon {
                request_id,
                weapon,
            })?;
        Ok(format!(
            "give: queued {} id={weapon} request_id={request_id}",
            registry.configuration_label(weapon)
        ))
    }

    fn give_killstreak(&mut self, name: Option<&String>) -> Result<String, String> {
        let name =
            name.ok_or("usage: give killstreak <name> (e.g. airdrop, uav, predator_missile)")?;
        self.alive_weapon("give killstreak")?;
        let field = menu_response_field(name)
            .ok_or_else(|| format!("give killstreak: `{name}` is too long"))?;
        let request_id = self.link.queue("give killstreak", |request_id| {
            ClientAction::GiveKillstreak {
                request_id,
                name: field,
            }
        })?;
        Ok(format!(
            "give killstreak: queued {name} request_id={request_id}"
        ))
    }

    fn attach(&mut self, args: &[String]) -> Result<String, String> {
        let registry = &self
            .weapons
            .as_ref()
            .ok_or("attach: weapon catalog not loaded")?
            .0;
        let current = self.alive_weapon("attach")?;
        if current == 0 {
            return Err("attach: no weapon in hands".into());
        }
        let Some(name) = args.first() else {
            return match attachment_hints(registry, current) {
                Ok(hints) => Ok(format!("attach: {hints}")),
                Err(why) => Err(format!("attach: {why}")),
            };
        };
        let next = toggled_id(registry, current, name).map_err(|why| format!("attach: {why}"))?;
        if next == current {
            return Err(format!(
                "attach: `{}` is already the selected configuration — refused (an unchanged id \
                 is not a successful attach)",
                registry.configuration_label(current)
            ));
        }
        let request_id = self.link.queue("attach", |request_id| {
            ClientAction::ChangeWeaponConfiguration {
                request_id,
                from: current,
                to: next,
            }
        })?;
        Ok(format!(
            "attach: queued {} id={next} request_id={request_id}",
            registry.configuration_label(next)
        ))
    }
}

pub(crate) fn route_weapon_commands(mut commands: MessageReader<ConsoleCommand>, mut arms: Arms) {
    for command in commands.read() {
        let reply = match command.name.as_str() {
            "give" => arms.give(&command.args),
            "attach" => arms.attach(&command.args),
            _ => continue,
        };
        let (Ok(text) | Err(text)) = reply;
        arms.echo.write(text);
    }
}

/// What the catalog calls a weapon, or its number when it has no name for it.
fn configuration_key(weapons: Option<&PreparedWeapons>, id: u32) -> String {
    weapons
        .map(|weapons| weapons.0.configuration_label(id))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| format!("#{id}"))
}

fn verdict(verb: &str, key: &str, id: u32, request_id: u32, outcome: Result<(), &str>) -> String {
    match outcome {
        Ok(()) => format!("{verb}: ok {key} id={id} request_id={request_id}"),
        Err(why) => format!("{verb}: rejected {key} id={id} request_id={request_id} ({why})"),
    }
}

/// Says how the host answered the last `give`, once.
pub(crate) fn echo_give_results(
    give: Option<Res<DumpGiveLog>>,
    weapons: Option<Res<PreparedWeapons>>,
    mut echo: ConsoleEcho,
    mut echoed: Local<Option<(u32, u8)>>,
) {
    let Some(give) = give else {
        return;
    };
    let (Some(request_id), Some(weapon), Some(accepted)) =
        (give.request_id, give.weapon, give.accepted)
    else {
        return;
    };
    if *echoed == Some((request_id, accepted)) {
        return;
    }
    *echoed = Some((request_id, accepted));
    let outcome = if accepted == 1 {
        Ok(())
    } else {
        Err(give.reject_reason.unwrap_or("unknown"))
    };
    let key = configuration_key(weapons.as_deref(), weapon);
    echo.write(verdict("give", &key, weapon, request_id, outcome));
}

/// Says how the host answered the last `attach`, once.
pub(crate) fn echo_configuration_change_results(
    changes: Option<Res<DumpConfigurationChangeLog>>,
    weapons: Option<Res<PreparedWeapons>>,
    mut echo: ConsoleEcho,
    mut echoed: Local<Option<u32>>,
) {
    let Some(changes) = changes else {
        return;
    };
    let (Some(request_id), Some(to), Some(accepted)) =
        (changes.request_id, changes.to, changes.accepted)
    else {
        return;
    };
    if *echoed == Some(request_id) {
        return;
    }
    *echoed = Some(request_id);
    let outcome = if accepted {
        Ok(())
    } else {
        Err(changes.reject_reason.unwrap_or("unknown"))
    };
    let key = configuration_key(weapons.as_deref(), to);
    echo.write(verdict("attach", &key, to, request_id, outcome));
}

#[cfg(test)]
mod tests {
    use frame::{MatchKey, TeardownReason, WorldGeneration};

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};
    use crate::host_link::testing::{FULL, host_app, present_holding, present_player, queued};

    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<PresentedSnapshot>()
            .add_systems(Update, route_weapon_commands);
        app
    }

    fn with_catalog() -> App {
        let mut app = app();
        app.init_resource::<PreparedWeapons>();
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
        assert_eq!(
            said(&app),
            ["usage: give <weapon> [attachment...] — equip a catalog weapon on the active slot"]
        );
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
        assert_eq!(
            said(&app),
            ["usage: give killstreak <name> (e.g. airdrop, uav, predator_missile)"]
        );
    }

    #[test]
    fn a_killstreak_is_queued_by_name_with_the_next_request_number() {
        let mut app = app();
        stand(&mut app, true);
        feed(&mut app, "give killstreak airdrop; give killstreak airdrop");
        assert_eq!(
            said(&app),
            [
                "give killstreak: queued airdrop request_id=1",
                "give killstreak: queued airdrop request_id=2"
            ]
        );
        assert_eq!(
            queued(&app),
            [
                ClientAction::GiveKillstreak {
                    request_id: 1,
                    name: airdrop()
                },
                ClientAction::GiveKillstreak {
                    request_id: 2,
                    name: airdrop()
                }
            ]
        );
    }

    #[test]
    fn a_killstreak_needs_a_player_who_is_alive() {
        let mut app = app();
        stand(&mut app, false);
        feed(&mut app, "give killstreak airdrop");
        assert_eq!(
            said(&app),
            ["give killstreak: not Alive — spawn a class first"]
        );
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn a_killstreak_name_longer_than_the_field_is_refused() {
        let mut app = app();
        stand(&mut app, true);
        let long = "x".repeat(49);
        feed(&mut app, &format!("give killstreak {long}"));
        assert_eq!(
            said(&app),
            [format!("give killstreak: `{long}` is too long")]
        );
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
        assert_eq!(
            said(&app).last().map(String::as_str),
            Some("give killstreak: action queue full (32 unresolved requests)")
        );
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
        app.add_systems(
            Update,
            (echo_give_results, echo_configuration_change_results).chain(),
        );
        app
    }

    fn given(weapon: u32, request_id: u32, accepted: u8, why: Option<&'static str>) -> DumpGiveLog {
        DumpGiveLog {
            request_id: Some(request_id),
            weapon: Some(weapon),
            accepted: Some(accepted),
            reject_reason: why,
        }
    }

    fn changed(
        to: u32,
        request_id: u32,
        accepted: bool,
        why: Option<&'static str>,
    ) -> DumpConfigurationChangeLog {
        DumpConfigurationChangeLog {
            request_id: Some(request_id),
            from: Some(1),
            to: Some(to),
            accepted: Some(accepted),
            reject_reason: why,
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
        app.init_resource::<PreparedWeapons>()
            .insert_resource(given(9, 4, 0, Some("no_slot")));
        app.update();
        assert_eq!(
            said(&app),
            ["give: rejected #9 id=9 request_id=4 (no_slot)"]
        );
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
        assert_eq!(
            said(&app),
            [
                "give: rejected #9 id=9 request_id=4 (unknown)",
                "give: ok #9 id=9 request_id=4",
                "give: ok #9 id=9 request_id=5"
            ]
        );
    }

    #[test]
    fn a_give_log_with_a_part_missing_is_not_said() {
        let mut app = echo_app();
        app.update();
        assert!(said(&app).is_empty(), "no log at all");
        for hole in 0..3 {
            let mut log = given(9, 4, 1, None);
            match hole {
                0 => log.request_id = None,
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
        assert_eq!(
            said(&app),
            [
                "attach: ok #12 id=12 request_id=3",
                "attach: rejected #13 id=13 request_id=4 (late)",
                "attach: rejected #14 id=14 request_id=5 (unknown)"
            ]
        );
    }

    #[test]
    fn an_attach_log_with_a_part_missing_is_not_said() {
        let mut app = echo_app();
        for hole in 0..3 {
            let mut log = changed(12, 3, true, None);
            match hole {
                0 => log.request_id = None,
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
        app.init_resource::<PresentedSnapshot>()
            .init_resource::<WeaponArgCompletions>()
            .add_message::<MatchTornDown>()
            .add_systems(
                Update,
                (
                    clear_weapon_args_on_torn_down,
                    refresh_weapon_arg_completions,
                )
                    .chain(),
            );
        app
    }

    fn listed(list: &NameList) -> Vec<String> {
        list.read().expect("nobody panicked holding it").clone()
    }

    fn stale(app: &App) {
        let lists = app.world().resource::<WeaponArgCompletions>();
        WeaponArgCompletions::replace(&lists.give, vec!["t6:old".to_owned()]);
        WeaponArgCompletions::replace(&lists.attach, vec!["old".to_owned()]);
    }

    fn lists(app: &App) -> (Vec<String>, Vec<String>) {
        let lists = app.world().resource::<WeaponArgCompletions>();
        (listed(&lists.give), listed(&lists.attach))
    }

    #[test]
    fn with_no_catalog_the_lists_are_left_alone() {
        let mut app = lists_app();
        stale(&app);
        app.update();
        assert_eq!(
            lists(&app),
            (vec!["t6:old".to_owned()], vec!["old".to_owned()])
        );
    }

    #[test]
    fn a_catalog_that_arrives_replaces_the_lists_and_then_they_are_left_alone() {
        let mut app = lists_app();
        stale(&app);
        app.init_resource::<PreparedWeapons>();
        app.update();
        assert_eq!(lists(&app), (Vec::new(), Vec::new()));
        stale(&app);
        app.update();
        assert_eq!(
            lists(&app),
            (vec!["t6:old".to_owned()], vec!["old".to_owned()])
        );
    }

    #[test]
    fn a_new_weapon_in_hand_rebuilds_the_lists() {
        let mut app = lists_app();
        app.init_resource::<PreparedWeapons>();
        app.update();
        stale(&app);
        present_holding(&mut app, 7);
        app.update();
        assert_eq!(lists(&app), (Vec::new(), Vec::new()));
        stale(&app);
        app.update();
        assert_eq!(lists(&app).0, ["t6:old"], "the same weapon changes nothing");
    }

    #[test]
    fn a_match_coming_down_empties_the_lists() {
        let mut app = lists_app();
        stale(&app);
        app.update();
        assert_eq!(lists(&app).0, ["t6:old"], "no message, nothing cleared");
        app.world_mut().write_message(MatchTornDown {
            reason: TeardownReason::Disconnect,
            world_generation: WorldGeneration::default(),
            match_key: MatchKey::default(),
            match_epoch: 0,
        });
        app.update();
        assert_eq!(lists(&app), (Vec::new(), Vec::new()));
    }

    fn registered() -> (ConsoleRegistry, WeaponArgCompletions) {
        let completions = WeaponArgCompletions::default();
        let mut registry = ConsoleRegistry::default();
        register_weapon_commands(&mut registry, &completions);
        (registry, completions)
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn both_commands_are_registered_with_their_usage() {
        let (registry, _) = registered();
        let give = registry.resolve("give").expect("give is registered");
        assert!(
            give.usage
                .starts_with("give <game:weapon> [attachment...] | give killstreak <name>")
        );
        assert!(give.usage.ends_with("(e.g. give killstreak airdrop)"));
        assert_eq!(
            registry
                .resolve("attach")
                .expect("attach is registered")
                .usage,
            "attach [name] — show the current set and choices, or toggle a named attachment"
        );
    }

    #[test]
    fn registering_twice_or_over_a_command_that_is_there_changes_nothing() {
        let (mut registry, completions) = registered();
        register_weapon_commands(&mut registry, &completions);
        assert_eq!(
            registry
                .names()
                .iter()
                .filter(|name| *name == "give")
                .count(),
            1
        );

        let mut mine = ConsoleRegistry::default();
        mine.register(CommandSpec::new("give").usage("mine"));
        register_weapon_commands(&mut mine, &completions);
        assert_eq!(mine.resolve("give").expect("still there").usage, "mine");
        assert!(mine.resolve("attach").is_some());
    }

    #[test]
    fn the_prompt_completes_from_the_lists_as_they_stand() {
        let (registry, completions) = registered();
        assert!(registry.suggestions("give t6:a", 9).is_empty());
        WeaponArgCompletions::replace(
            &completions.give,
            names(&["t6:an94", "t6:ak47", "mp:ak74u"]),
        );
        WeaponArgCompletions::replace(&completions.attach, names(&["reflex", "acog"]));
        assert_eq!(registry.suggestions("give t6:a", 9), ["t6:an94", "t6:ak47"]);
        assert_eq!(registry.suggestions("give ", 5).len(), 3);
        assert_eq!(registry.suggestions("attach RE", 9), ["reflex"]);
        WeaponArgCompletions::replace(&completions.give, names(&["t6:fal"]));
        assert_eq!(registry.suggestions("give t6:", 8), ["t6:fal"]);
    }
}
