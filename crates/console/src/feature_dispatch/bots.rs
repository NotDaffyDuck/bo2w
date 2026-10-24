//! `bot` and `hurt`: test bots, and a damage punch for the view.

use bevy::{ecs::system::SystemParam, prelude::*};
use bots::{
    BotAddQueue, BotFireQueue, BotHold, BotRoster, BotTpQueue, BotTpRequest, BotTpTarget,
    BotTpWhere,
};
use net::{ActionRequestIds, ClientActionInbox};
use render_frontend::adapters::anim::view_kick::PendingViewHurt;
use sim::{ClientAction, ClientId};

use super::echo::ConsoleEcho;
use crate::ConsoleCommand;

pub(super) const USAGE: &[(&str, &str)] = &[
    (
        "bot",
        "bot add [N] | dummy [N] | hold [on|off] | give <id> <weapon> [att...] | fire [all|<id>] | tp all|<id> above <h> | tp all|<id> <x> <y> <z>",
    ),
    (
        "hurt",
        "hurt — give the view one damage punch from no direction",
    ),
];

const BOT_USAGE: &str = "usage: bot add [N] | dummy [N] | hold [on|off] | give <id> <weapon> [att...] | fire [all|<id>] | tp all|<id> above <h> | tp all|<id> <x> <y> <z> [yaw] [pitch]";

const GIVE_USAGE: &str = "usage: bot give <id> <weapon> [attachment...]";

/// The most bots one `bot add` or `bot dummy` makes.
const MOST_BOTS: u32 = 16;

#[derive(Debug, PartialEq)]
pub(crate) enum BotVerb {
    Add(u32),
    Dummy(u32),
    Hold(bool),
    Give {
        id: ClientId,
        weapon: String,
        attachments: Vec<String>,
    },
    Fire(BotTpTarget),
    Tp(BotTpRequest),
}

fn finite(word: &str) -> Result<f32, String> {
    word.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("bot: not a finite number `{word}`"))
}

fn client_id(word: &str) -> Option<ClientId> {
    word.parse().ok().map(ClientId)
}

/// Who a `fire` or `tp` is about: `all`, or one client's number.
fn target_of(word: &str) -> Option<BotTpTarget> {
    match word {
        "all" => Some(BotTpTarget::All),
        _ => client_id(word).map(BotTpTarget::Id),
    }
}

fn parse_teleport(words: &[&str]) -> Result<BotVerb, String> {
    let Some((who, place)) = words.split_first() else {
        return Err(BOT_USAGE.into());
    };
    let target = target_of(who).ok_or(BOT_USAGE)?;
    let where_ = match place {
        ["above"] => return Err("usage: bot tp all|<id> above <h>".into()),
        ["above", height, ..] => BotTpWhere::Above {
            height: finite(height)?,
        },
        [x, y, z, angles @ ..] if angles.len() <= 2 => BotTpWhere::Absolute {
            origin: [finite(x)?, finite(y)?, finite(z)?],
            yaw: angles.first().map(|word| finite(word)).transpose()?,
            pitch: angles.get(1).map(|word| finite(word)).transpose()?,
        },
        _ => return Err(BOT_USAGE.into()),
    };
    Ok(BotVerb::Tp(BotTpRequest { target, where_ }))
}

pub(crate) fn parse_bot(args: &[String]) -> Result<BotVerb, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["add", rest @ ..] => {
            let count = rest.first().and_then(|word| word.parse().ok()).unwrap_or(1);
            Ok(BotVerb::Add(count.clamp(1, MOST_BOTS)))
        }
        ["dummy", rest @ ..] => {
            let usage = || "usage: bot dummy [N]".to_owned();
            let count = match rest {
                [] => 1,
                [word] => word.parse::<u32>().map_err(|_| usage())?,
                _ => return Err(usage()),
            };
            if (1..=MOST_BOTS).contains(&count) {
                Ok(BotVerb::Dummy(count))
            } else {
                Err(format!("bot dummy: count must be 1..{MOST_BOTS}"))
            }
        }
        ["hold"] | ["hold", "on" | "1"] => Ok(BotVerb::Hold(true)),
        ["hold", "off" | "0"] => Ok(BotVerb::Hold(false)),
        ["hold", other, ..] => Err(format!("usage: bot hold [on|off] (got `{other}`)")),
        ["give", id, weapon, attachments @ ..] => match client_id(id) {
            Some(id) => Ok(BotVerb::Give {
                id,
                weapon: (*weapon).to_owned(),
                attachments: attachments.iter().map(|word| (*word).to_owned()).collect(),
            }),
            None => Err(GIVE_USAGE.into()),
        },
        ["give", ..] => Err(GIVE_USAGE.into()),
        ["fire"] => Ok(BotVerb::Fire(BotTpTarget::All)),
        ["fire", who, ..] => target_of(who)
            .map(BotVerb::Fire)
            .ok_or_else(|| "usage: bot fire [all|<id>]".into()),
        ["tp", rest @ ..] => parse_teleport(rest),
        _ => Err(BOT_USAGE.into()),
    }
}

/// What `bot give` needs from the world.
#[derive(SystemParam)]
pub(crate) struct GiveDesk<'w> {
    weapons: Option<Res<'w, assets::PreparedWeapons>>,
    inbox: Option<ResMut<'w, ClientActionInbox>>,
    request_ids: ResMut<'w, ActionRequestIds>,
    roster: Option<Res<'w, BotRoster>>,
}

impl GiveDesk<'_> {
    /// Asks the authority to give bot `id` the weapon the words name, and
    /// says what was asked.
    fn give(
        &mut self,
        id: ClientId,
        weapon: &str,
        attachments: &[String],
    ) -> Result<String, String> {
        let weapons = self.weapons.as_ref().ok_or("weapon catalog not loaded")?;
        let inbox = self.inbox.as_mut().ok_or("no action inbox")?;
        if !self.roster.as_ref().is_some_and(|roster| roster.is_bot(id)) {
            return Err(format!("{id:?} is not a bot"));
        }
        let weapon_id = crate::weapon_dispatch::resolve_give_id(&weapons.0, weapon, attachments)?;
        let request_id = self.request_ids.allocate();
        inbox
            .push(
                id,
                ClientAction::GiveWeapon {
                    request_id,
                    weapon: weapon_id,
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(format!(
            "queued {} id={weapon_id} on {} request_id={request_id}",
            weapons.0.configuration_label(weapon_id),
            id.0
        ))
    }
}

/// The queues the bot crate drains each tick.
#[derive(SystemParam)]
pub(crate) struct BotQueues<'w> {
    add: ResMut<'w, BotAddQueue>,
    hold: ResMut<'w, BotHold>,
    teleport: ResMut<'w, BotTpQueue>,
    fire: ResMut<'w, BotFireQueue>,
}

impl BotQueues<'_> {
    /// Queues the verb, and says what was queued. `None` for a verb that is
    /// not a queue (`give`).
    fn queue(&mut self, verb: BotVerb) -> Option<String> {
        Some(match verb {
            BotVerb::Add(count) => {
                self.add.push(count);
                format!("bot: queued add {count}")
            }
            BotVerb::Dummy(count) => {
                self.add.push_dummy(count);
                format!("bot: queued dummy {count}")
            }
            BotVerb::Hold(on) => {
                self.hold.0 = on;
                format!("bot: hold {}", if on { "on" } else { "off" })
            }
            BotVerb::Tp(request) => {
                self.teleport.push(request);
                "bot: queued tp".into()
            }
            BotVerb::Fire(target) => {
                self.fire.push(target);
                "bot: queued fire".into()
            }
            BotVerb::Give { .. } => return None,
        })
    }
}

pub(crate) fn route_bot_commands(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut queues: BotQueues,
    mut desk: GiveDesk,
    mut hurt: ResMut<PendingViewHurt>,
) {
    for command in commands.read() {
        match command.name.as_str() {
            "bot" => match parse_bot(&command.args) {
                Err(message) => echo.write(message),
                Ok(BotVerb::Give {
                    id,
                    weapon,
                    attachments,
                }) => match desk.give(id, &weapon, &attachments) {
                    Ok(done) => echo.write(format!("bot give: {done}")),
                    Err(why) => echo.write(format!("bot give: {why}")),
                },
                Ok(verb) => {
                    if let Some(done) = queues.queue(verb) {
                        echo.write(done);
                    }
                }
            },
            "hurt" => {
                hurt.0 = hurt.0.saturating_add(1);
                echo.write("hurt: queued undirected view punch (255/255 count=1)");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<BotVerb, String> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse_bot(&args)
    }

    fn error(line: &str) -> String {
        parse(line).unwrap_err()
    }

    #[test]
    fn add_takes_a_count_that_is_clamped_and_defaults_to_one() {
        assert_eq!(parse("add"), Ok(BotVerb::Add(1)));
        assert_eq!(parse("add 5"), Ok(BotVerb::Add(5)));
        assert_eq!(parse("add 0"), Ok(BotVerb::Add(1)));
        assert_eq!(parse("add 99"), Ok(BotVerb::Add(16)));
        assert_eq!(parse("add many"), Ok(BotVerb::Add(1)));
    }

    #[test]
    fn dummy_wants_a_whole_count_from_one_to_sixteen() {
        assert_eq!(parse("dummy"), Ok(BotVerb::Dummy(1)));
        assert_eq!(parse("dummy 16"), Ok(BotVerb::Dummy(16)));
        assert_eq!(error("dummy 0"), "bot dummy: count must be 1..16");
        assert_eq!(error("dummy 17"), "bot dummy: count must be 1..16");
        assert_eq!(error("dummy x"), "usage: bot dummy [N]");
        assert_eq!(error("dummy 1 2"), "usage: bot dummy [N]");
    }

    #[test]
    fn hold_is_on_unless_told_off() {
        for line in ["hold", "hold on", "hold 1"] {
            assert_eq!(parse(line), Ok(BotVerb::Hold(true)), "{line}");
        }
        for line in ["hold off", "hold 0"] {
            assert_eq!(parse(line), Ok(BotVerb::Hold(false)), "{line}");
        }
        assert_eq!(
            error("hold maybe"),
            "usage: bot hold [on|off] (got `maybe`)"
        );
    }

    #[test]
    fn give_names_a_client_a_weapon_and_any_attachments() {
        assert_eq!(
            parse("give 3 an94 reflex silencer"),
            Ok(BotVerb::Give {
                id: ClientId(3),
                weapon: "an94".into(),
                attachments: vec!["reflex".into(), "silencer".into()],
            })
        );
        assert_eq!(error("give"), GIVE_USAGE);
        assert_eq!(error("give 3"), GIVE_USAGE);
        assert_eq!(error("give x an94"), GIVE_USAGE);
    }

    #[test]
    fn fire_is_for_everyone_or_one_client() {
        assert_eq!(parse("fire"), Ok(BotVerb::Fire(BotTpTarget::All)));
        assert_eq!(parse("fire all"), Ok(BotVerb::Fire(BotTpTarget::All)));
        assert_eq!(
            parse("fire 2"),
            Ok(BotVerb::Fire(BotTpTarget::Id(ClientId(2))))
        );
        assert_eq!(error("fire everyone"), "usage: bot fire [all|<id>]");
    }

    #[test]
    fn teleporting_above_takes_a_height() {
        assert_eq!(
            parse("tp all above 64"),
            Ok(BotVerb::Tp(BotTpRequest {
                target: BotTpTarget::All,
                where_: BotTpWhere::Above { height: 64.0 },
            }))
        );
        assert_eq!(error("tp all above"), "usage: bot tp all|<id> above <h>");
        assert_eq!(
            error("tp all above high"),
            "bot: not a finite number `high`"
        );
    }

    #[test]
    fn teleporting_to_a_place_takes_three_to_five_numbers() {
        assert_eq!(
            parse("tp 4 1 2 3"),
            Ok(BotVerb::Tp(BotTpRequest {
                target: BotTpTarget::Id(ClientId(4)),
                where_: BotTpWhere::Absolute {
                    origin: [1.0, 2.0, 3.0],
                    yaw: None,
                    pitch: None,
                },
            }))
        );
        let Ok(BotVerb::Tp(request)) = parse("tp all 1 2 3 90 -10") else {
            panic!("five numbers should parse");
        };
        assert_eq!(
            request.where_,
            BotTpWhere::Absolute {
                origin: [1.0, 2.0, 3.0],
                yaw: Some(90.0),
                pitch: Some(-10.0),
            }
        );
        assert_eq!(error("tp all 1 2"), BOT_USAGE);
        assert_eq!(error("tp all 1 2 3 4 5 6"), BOT_USAGE);
    }

    #[test]
    fn a_number_that_is_not_finite_is_refused() {
        assert_eq!(error("tp all 1 2 inf"), "bot: not a finite number `inf`");
        assert_eq!(error("tp all 1 NaN 3"), "bot: not a finite number `NaN`");
        assert_eq!(error("tp all above inf"), "bot: not a finite number `inf`");
    }

    #[test]
    fn anything_else_prints_the_whole_usage() {
        assert_eq!(error(""), BOT_USAGE);
        assert_eq!(error("dance"), BOT_USAGE);
        assert_eq!(error("tp"), BOT_USAGE);
        assert_eq!(error("tp nobody 1 2 3"), BOT_USAGE);
    }
}
