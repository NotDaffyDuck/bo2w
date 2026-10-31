//! `bot` and `hurt`: test bots, and a damage punch for the view.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bots::{
    BotJoinQueue, BotFreezeFlag, BotTeamList, BotWarpQueue, BotWarpOrder, BotWarpSubject,
    BotWarpDestination,
};
use net::{ActionRequestCounter, PlayerActionIntake};
use render_frontend::adapters::motion::eye_kick::QueuedHurtFlinch;
use sim::PlayerIntent;
use sim::ClientId;

use super::echo::PromptReplyWriter;
use crate::TypedOrder;

pub(super) const HELP_BANNER: &[(&str, &str)] = &[
    (
        "bot",
        "bot add [N] | hold [on|off] | give <id> <weapon> [att...] | tp all|<id> above <h> | tp all|<id> <x> <y> <z>",
    ),
    (
        "hurt",
        "hurt — give the view one damage punch from no direction",
    ),
];

const BOT_ORDER_HELP: &str = "usage: bot add [N] | hold [on|off] | give <id> <weapon> [att...] | tp all|<id> above <h> | tp all|<id> <x> <y> <z> [yaw] [pitch]";

const GIVE_USAGE: &str = "usage: bot give <id> <weapon> [attachment...]";

/// The most bots one `bot add` makes.
const MOST_BOTS: u32 = 16;

#[derive(Debug, PartialEq)]
pub(crate) enum BotOrderKind {
    Add(u32),
    Freeze(bool),
    Give {
        bot: sim::ClientId,
        weapon_name: String,
        attachments: Vec<String>,
    },
    Tp(BotWarpOrder),
}

fn finite(word: &str) -> Result<f32, String> {
    let refusal = || format!("bot: not a finite number `{word}`");
    word.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(refusal)
}

fn player_slot_no(word: &str) -> Option<ClientId> {
    word.parse().ok().map(ClientId)
}

/// Who a `tp` is about: `all`, or one client's number.
fn target_of(word: &str) -> Option<BotWarpSubject> {
    match word {
        "all" => Some(BotWarpSubject::All),
        _ => player_slot_no(word).map(BotWarpSubject::Id),
    }
}

fn parse_teleport(words: &[&str]) -> Result<BotOrderKind, String> {
    let Some((who, place)) = words.split_first() else {
        return Err(BOT_ORDER_HELP.into());
    };
    let target = target_of(who).ok_or(BOT_ORDER_HELP)?;
    let where_ = match place {
        ["above"] => return Err("usage: bot tp all|<id> above <h>".into()),
        ["above", height, ..] => BotWarpDestination::Above {
            height: finite(height)?,
        },
        [x, y, z, angles @ ..] if angles.len() <= 2 => BotWarpDestination::Absolute {
            at_point: [finite(x)?, finite(y)?, finite(z)?],
            turn_yaw: angles.first().map(|word| finite(word)).transpose()?,
            pitch: angles.get(1).map(|word| finite(word)).transpose()?,
        },
        _ => return Err(BOT_ORDER_HELP.into()),
    };
    Ok(BotOrderKind::Tp(BotWarpOrder { target, where_ }))
}

pub(crate) fn parse_bot(args: &[String]) -> Result<BotOrderKind, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["add", tail @ ..] => {
            let count = tail.first().and_then(|word| word.parse().ok()).unwrap_or(1);
            Ok(BotOrderKind::Add(count.clamp(1, MOST_BOTS)))
        }
        ["hold"] | ["hold", "on" | "1"] => Ok(BotOrderKind::Freeze(true)),
        ["hold", "off" | "0"] => Ok(BotOrderKind::Freeze(false)),
        ["hold", other, ..] => Err(format!("usage: bot hold [on|off] (got `{other}`)")),
        ["give", id, weapon, attachments @ ..] => match player_slot_no(id) {
            Some(bot) => Ok(BotOrderKind::Give {
                bot,
                weapon_name: (*weapon).to_owned(),
                attachments: attachments.iter().map(|word| (*word).to_owned()).collect(),
            }),
            None => Err(GIVE_USAGE.into()),
        },
        ["give", ..] => Err(GIVE_USAGE.into()),
        ["tp", rest @ ..] => parse_teleport(rest),
        _ => Err(BOT_ORDER_HELP.into()),
    }
}

/// What `bot give` needs from the world.
#[derive(SystemParam)]
pub(crate) struct GiveDesk<'w> {
    staged: Option<Res<'w, assets::StagedWeapons>>,
    inbox: Option<ResMut<'w, PlayerActionIntake>>,
    ask_serials: ResMut<'w, ActionRequestCounter>,
    roster: Option<Res<'w, BotTeamList>>,
}

impl GiveDesk<'_> {
    /// Asks the authority to give bot `id` the weapon the words name, and
    /// says what was asked.
    fn give(
        &mut self,
        bot: sim::ClientId,
        gun: &str,
        attachments: &[String],
    ) -> Result<String, String> {
        let weapons = self.staged.as_ref().ok_or("weapon catalog not loaded")?;
        let inbox = self.inbox.as_mut().ok_or("no action inbox")?;
        if !self.roster.as_ref().is_some_and(|roster| roster.seats_bot(bot)) {
            return Err(format!("{bot:?} is not a bot"));
        }
        let gun_catalog_no = crate::weapon_cmds::resolve_give_id(&weapons.0, gun, attachments)?;
        let serial = self.ask_serials.allocate();
        inbox
            .push(
                bot,
                PlayerIntent::HandOutGun {
                    request_id: serial,
                    gun: gun_catalog_no,
                },
            )
            .map_err(|refused| refused.to_string())?;
        let answer = format!(
            "queued {} id={gun_catalog_no} on {} request_id={serial}",
            weapons.0.configuration_label(gun_catalog_no),
            bot.0
        );
        Ok(answer)
    }
}

/// The queues the bot crate drains each tick.
#[derive(SystemParam)]
pub(crate) struct BotQueues<'w> {
    add: ResMut<'w, BotJoinQueue>,
    hold: ResMut<'w, BotFreezeFlag>,
    teleport: ResMut<'w, BotWarpQueue>,
}

impl BotQueues<'_> {
    /// Queues the verb, and says what was queued. `None` for a verb that is
    /// not a queue (`give`).
    fn queue(&mut self, verb: BotOrderKind) -> Option<String> {
        Some(match verb {
            BotOrderKind::Add(count) => {
                self.add.ask_for(count);
                format!("bot: queued add {count}")
            }
            BotOrderKind::Freeze(on) => {
                self.hold.0 = on;
                let state = if on { "on" } else { "off" };
                format!("bot: hold {state}")
            }
            BotOrderKind::Tp(request) => {
                self.teleport.enqueue(request);
                "bot: queued tp".into()
            }
            BotOrderKind::Give { .. } => return None,
        })
    }
}

pub(crate) fn route_bot_commands(
    mut commands: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut queues: BotQueues,
    mut desk: GiveDesk,
    mut hurt: ResMut<QueuedHurtFlinch>,
) {
    for order in commands.read() {
        let verb = order.name.as_str();
        match verb {
            "bot" => match parse_bot(&order.words) {
                Err(message) => echo.write(message),
                Ok(BotOrderKind::Give {
                    bot,
                    weapon_name,
                    attachments: extras,
                }) => match desk.give(bot, &weapon_name, &extras) {
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
                let flinches = &mut hurt.0;
                *flinches = flinches.saturating_add(1);
                echo.write("hurt: queued undirected view punch (255/255 count=1)");
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tool_cmds::bots::*;

    fn parse(line: &str) -> Result<BotOrderKind, String> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        parse_bot(&args)
    }

    fn error(line: &str) -> String {
        parse(line).unwrap_err()
    }

    #[test]
    fn add_takes_a_count_that_is_clamped_and_defaults_to_one() {
        assert_eq!(parse("add"), Ok(BotOrderKind::Add(1)));
        assert_eq!(parse("add 5"), Ok(BotOrderKind::Add(5)));
        assert_eq!(parse("add 0"), Ok(BotOrderKind::Add(1)));
        assert_eq!(parse("add 99"), Ok(BotOrderKind::Add(16)));
        assert_eq!(parse("add many"), Ok(BotOrderKind::Add(1)));
    }

    #[test]
    fn hold_is_on_unless_told_off() {
        for line in ["hold", "hold on", "hold 1"] {
            assert_eq!(parse(line), Ok(BotOrderKind::Freeze(true)), "{line}");
        }
        for line in ["hold off", "hold 0"] {
            assert_eq!(parse(line), Ok(BotOrderKind::Freeze(false)), "{line}");
        }
        let expected = "usage: bot hold [on|off] (got `maybe`)";
        assert_eq!(error("hold maybe"), expected);
    }

    #[test]
    fn give_names_a_client_a_weapon_and_any_attachments() {
        let expected = Ok(BotOrderKind::Give {
            bot: sim::ClientId(3),
            weapon_name: "an94".into(),
            attachments: vec!["reflex".into(), "silencer".into()],
        });
        assert_eq!(parse("give 3 an94 reflex silencer"), expected);
        assert_eq!(error("give"), GIVE_USAGE);
        assert_eq!(error("give 3"), GIVE_USAGE);
        assert_eq!(error("give x an94"), GIVE_USAGE);
    }

    #[test]
    fn teleporting_above_takes_a_height() {
        let expected = Ok(BotOrderKind::Tp(BotWarpOrder {
            target: BotWarpSubject::All,
            where_: BotWarpDestination::Above { height: 64.0 },
        }));
        assert_eq!(parse("tp all above 64"), expected);
        assert_eq!(error("tp all above"), "usage: bot tp all|<id> above <h>");
        let expected = "bot: not a finite number `high`";
        assert_eq!(error("tp all above high"), expected);
    }

    #[test]
    fn teleporting_to_a_place_takes_three_to_five_numbers() {
        let expected = Ok(BotOrderKind::Tp(BotWarpOrder {
                target: BotWarpSubject::Id(ClientId(4)),
            where_: BotWarpDestination::Absolute {
                at_point: [1.0, 2.0, 3.0],
                turn_yaw: None,
                pitch: None,
            },
        }));
        assert_eq!(parse("tp 4 1 2 3"), expected);
        let Ok(BotOrderKind::Tp(request)) = parse("tp all 1 2 3 90 -10") else {
            panic!("five numbers should parse");
        };
        let expected = BotWarpDestination::Absolute {
            at_point: [1.0, 2.0, 3.0],
            turn_yaw: Some(90.0),
            pitch: Some(-10.0),
        };
        assert_eq!(request.where_, expected);
        assert_eq!(error("tp all 1 2"), BOT_ORDER_HELP);
        assert_eq!(error("tp all 1 2 3 4 5 6"), BOT_ORDER_HELP);
    }

    #[test]
    fn a_number_that_is_not_finite_is_refused() {
        assert_eq!(error("tp all 1 2 inf"), "bot: not a finite number `inf`");
        assert_eq!(error("tp all 1 NaN 3"), "bot: not a finite number `NaN`");
        assert_eq!(error("tp all above inf"), "bot: not a finite number `inf`");
    }

    #[test]
    fn anything_else_prints_the_whole_usage() {
        assert_eq!(error(""), BOT_ORDER_HELP);
        assert_eq!(error("dance"), BOT_ORDER_HELP);
        assert_eq!(error("tp"), BOT_ORDER_HELP);
        assert_eq!(error("tp nobody 1 2 3"), BOT_ORDER_HELP);
    }
}
