//! Console commands that act on the local player by asking the host: teleport,
//! turn, damage, kill, respawn, rename, and starting the match. Each one
//! queues a request and reports what it queued; the host carries it out.
//!
//! Checks always run in the same order: the arguments, then that the player is
//! Alive (where it matters), then that the host allows cheats, then that the
//! request can be queued. The first failure is the reply, and in that case
//! nothing is queued and no wait begins.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use net::{ViewAimAccumulator, ShownWorldFrame, aim_angles_from_degrees};
use sim::{IntentSerial, PlayerIntent, GameRoundStage, SpawnPick};
use sim::LifeSequence;

use crate::tool_cmds::PromptReplyWriter;
use crate::host_link::HostLink;
use crate::OrderBlueprint;
use crate::TypedOrder;
use crate::TypedOrderRouter;
use crate::OrderCatalog;
use crate::Hold;
use crate::AwaitMotionPose;

const MOVE_USAGE: &str = "usage: move <x> <y> <z> [yaw] [pitch]";
const LOOK_USAGE: &str = "usage: look <yaw> <pitch>";
const NEARPIECE_USAGE: &str = "usage: nearpiece [dist]";
const DAMAGE_USAGE: &str = "usage: damage [amount]";
const SPAWN_USAGE: &str = "usage: force_spawn [random <seed> | at <x> <y> <z> [yaw]]";
const NUDGE_USAGE: &str = "usage: nudge <dx> <dy> <dz>";
const NAME_USAGE: &str = "usage: name [string]";

/// Default standing distance from a resting piece for `nearpiece`.
const PIECE_DISTANCE: f32 = 40.0;
/// Default amount of `damage`.
const DAMAGE: i32 = 40;
/// Feet-to-eye offset, used to aim at a piece from where `nearpiece` stands.
const EYE_HEIGHT: f32 = 60.0;

/// One console command: its name, aliases and help line.
#[derive(Clone, Copy)]
struct CheatEntry {
    name: &'static str,
    aliases: &'static [&'static str],
    help: &'static str,
}

const CHEAT_COMMANDS: [CheatEntry; 9] = [
    CheatEntry {
        name: "move",
        aliases: &["tp"],
        help: "move <x> <y> <z> [yaw] [pitch] — place the player (needs cheats)",
    },
    CheatEntry {
        name: "nearpiece",
        aliases: &[],
        help: "nearpiece [dist] — stand dist (40) from the last loose piece to come to rest, looking at it (needs cheats)",
    },
    CheatEntry {
        name: "kill",
        aliases: &[],
        help: "kill — kill the player (needs cheats)",
    },
    CheatEntry {
        name: "force_spawn",
        aliases: &[],
        help: "force_spawn [random <seed> | at <x> <y> <z> [yaw]] — spawn now (needs cheats)",
    },
    CheatEntry {
        name: "damage",
        aliases: &[],
        help: "damage [amount] — hurt the player, 40 if not told (needs cheats)",
    },
    CheatEntry {
        name: "look",
        aliases: &[],
        help: "look <yaw> <pitch> — turn the player where they stand (needs cheats)",
    },
    CheatEntry {
        name: "name",
        aliases: &[],
        help: "name [string] — show or set the player name",
    },
    CheatEntry {
        name: "nudge",
        aliases: &[],
        help: "nudge <dx> <dy> <dz> — shift the host's origin for the player (needs cheats)",
    },
    CheatEntry {
        name: "force_match_start",
        aliases: &[],
        help: "force_match_start — end warmup and play (needs cheats)",
    },
];

pub(crate) fn enlist(book: &mut OrderCatalog) {
    let missing = CHEAT_COMMANDS.iter().filter(|entry| book.look_up(entry.name).is_none());
    let blueprints: Vec<OrderBlueprint> = missing
        .map(|entry| {
            let mut blueprint = OrderBlueprint::new(entry.name);
            for other in entry.aliases {
                blueprint = blueprint.also_known(*other);
            }
            blueprint.help_line(entry.help)
        })
        .collect::<Vec<OrderBlueprint>>();
    for blueprint in blueprints {
        book.enlist(blueprint);
    }
}

/// Everything the commands touch: the screen's snapshot, the host link, the
/// player's aim and the console's waits.
#[derive(SystemParam)]
pub(crate) struct Desk<'w> {
    shown: Res<'w, ShownWorldFrame>,
    link: HostLink<'w>,
    look: ResMut<'w, ViewAimAccumulator>,
    dispatch: ResMut<'w, TypedOrderRouter>,
}

/// Position and view angles of the local player.
struct Pose {
    spot: [f32; 3],
    aim: [f32; 3],
}

impl Desk<'_> {
    fn pose(&self, verb: &str) -> Result<Pose, String> {
        let Some(player) = self.shown.living_participant(self.link.client()) else {
            return Err(format!("{verb}: not Alive — spawn a class first"));
        };
        Ok(Pose {
            spot: player.origin,
            aim: player.look_angles,
        })
    }

    /// Asks the host to put the player at `origin` facing `angles`, points
    /// the local aim the same way, and (for a line not ended with `&`) keeps
    /// the console waiting until the screen shows the player there.
    fn place(
        &mut self,
        verb: &str,
        detached: bool,
        spot: [f32; 3],
        aim: [f32; 3],
    ) -> Result<IntentSerial, String> {
        self.link.allow_cheats(verb)?;
        let move_intent = |serial| PlayerIntent::Move { request_id: serial, origin: spot, angles: aim };
        let id = self.link.queue(verb, move_intent)?;
        self.look.angles = aim_angles_from_degrees(aim);
        if !detached {
            let client = self.link.client();
            self.dispatch.stall(Hold::Move(AwaitMotionPose { who: client, spot, facing: aim }));
        }
        Ok(id)
    }
}

/// How a placing command waits, as its reply says it: on its own, or until
/// the screen shows the player there.
fn wait_words(detached: bool) -> &'static str {
    match detached {
        true => "async",
        false => "sync until presented pose",
    }
}

/// Parses exactly `N` finite numbers, or replies with `usage` when the count
/// is wrong.
fn exact_numbers<const N: usize>(
    verb: &str,
    words: &[String],
    usage: &str,
) -> Result<[f32; N], String> {
    if words.len() != N {
        return Err(usage.to_owned());
    }
    let mut parsed = [0.0f32; N];
    for (slot, text) in parsed.iter_mut().zip(words) {
        *slot = number(verb, text)?;
    }
    Ok(parsed)
}

fn number(verb: &str, text: &str) -> Result<f32, String> {
    match text.parse::<f32>() {
        Ok(value) if value.is_finite() => Ok(value),
        _ => Err(format!("{verb}: not a finite number `{text}`")),
    }
}

fn go(desk: &mut Desk, words: &[String], detached: bool) -> Result<String, String> {
    let (place_args, turn_args) = match words.len() {
        3..=5 => words.split_at(3),
        _ => return Err(MOVE_USAGE.into()),
    };
    let [x, y, z] = exact_numbers::<3>("move", place_args, MOVE_USAGE)?;
    let turn = turn_args
        .iter()
        .map(|text| number("move", text))
        .collect::<Result<Vec<f32>, String>>()?;
    let current = desk.pose("move")?;
    let [held_pitch, held_yaw, roll] = current.aim;
    let yaw = turn.first().copied().unwrap_or(held_yaw);
    let pitch = turn.get(1).copied().unwrap_or(held_pitch);
    let id = desk.place("move", detached, [x, y, z], [pitch, yaw, roll])?;
    let how = wait_words(detached);
    let answer = format!(
        "move: queued ({x:.1} {y:.1} {z:.1}) yaw={yaw:.0} pitch={pitch:.0} request_id={id} ({how})"
    );
    Ok(answer)
}

fn look(desk: &mut Desk, words: &[String], detached: bool) -> Result<String, String> {
    let [yaw, pitch] = exact_numbers::<2>("look", words, LOOK_USAGE)?;
    let current = desk.pose("look")?;
    let facing = [pitch, yaw, current.aim[2]];
    let id = desk.place("look", detached, current.spot, facing)?;
    let how = wait_words(detached);
    Ok(format!("look: queued yaw={yaw:.0} pitch={pitch:.0} request_id={id} ({how})"))
}

/// Destination and view angles that put a player now at `player` on the same
/// side of `piece`, `dist` away from it at its height, facing it.
fn stand_near(player: [f32; 3], piece: [f32; 3], dist: f32) -> ([f32; 3], [f32; 3]) {
    let offset_x = player[0] - piece[0];
    let offset_y = player[1] - piece[1];
    let span = offset_x.hypot(offset_y);
    let (dir_x, dir_y) = if span > 0.01 {
        (offset_x / span, offset_y / span)
    } else {
        (1.0, 0.0)
    };
    let origin = [piece[0] + dir_x * dist, piece[1] + dir_y * dist, piece[2]];
    let gaze = [
        piece[0] - origin[0],
        piece[1] - origin[1],
        piece[2] + 1.0 - (origin[2] + EYE_HEIGHT),
    ];
    let yaw = gaze[1].atan2(gaze[0]).to_degrees();
    let pitch = -gaze[2].atan2(gaze[0].hypot(gaze[1])).to_degrees();
    (origin, [pitch, yaw, 0.0])
}

/// Where the animation system last saw a loose piece settle, if it has.
fn last_rest() -> Option<[f32; 3]> {
    let guard = render_frontend::adapters::motion::debris_phys::LAST_REST.lock();
    guard.ok().and_then(|rest| *rest)
}

fn near_piece(
    desk: &mut Desk,
    words: &[String],
    detached: bool,
    resting_piece: Option<[f32; 3]>,
) -> Result<String, String> {
    let dist = match words {
        [] => PIECE_DISTANCE,
        [text] => number("nearpiece", text)?,
        _ => return Err(NEARPIECE_USAGE.into()),
    };
    let current = desk.pose("nearpiece")?;
    let piece = resting_piece.ok_or("nearpiece: no loose piece has come to rest")?;
    let (spot, facing) = stand_near(current.spot, piece, dist);
    diag::info!(Console, "nearpiece: piece at {piece:?}, standing at {spot:?}");
    let id = desk.place("nearpiece", detached, spot, facing)?;
    let how = wait_words(detached);
    let [x, y, z] = spot;
    Ok(format!("nearpiece: queued ({x:.1} {y:.1} {z:.1}) request_id={id} ({how})"))
}

fn kill(desk: &mut Desk) -> Result<String, String> {
    desk.pose("kill")?;
    desk.link.allow_cheats("kill")?;
    let id = desk
        .link
        .queue("kill", |request_id| PlayerIntent::ForceDeath { request_id })?;
    Ok(format!("kill: queued ForceDeath request_id={id}"))
}

fn damage(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    let points = match args {
        [] => DAMAGE,
        [text] => match text.parse::<i32>() {
            Ok(parsed) => parsed,
            Err(_) => return Err(format!("{DAMAGE_USAGE} (got `{text}`)")),
        },
        _ => return Err(DAMAGE_USAGE.into()),
    };
    if points <= 0 {
        return Err("damage: amount must be > 0".into());
    }
    desk.pose("damage")?;
    desk.link.allow_cheats("damage")?;
    let id = desk
        .link
        .queue("damage", |serial| PlayerIntent::TestHarm {
            request_id: serial,
            amount: points,
        })?;
    Ok(format!("damage: queued {points} request_id={id}"))
}

/// `force_spawn` with no words is a seeded pick with seed 0; `random <seed>`
/// picks another seed; `at <x> <y> <z> [yaw]` names the spot.
fn spawn_pick(args: &[String]) -> Result<SpawnPick, String> {
    let Some((mode, rest)) = args.split_first() else {
        return Ok(SpawnPick::Seeded(0));
    };
    let bad = |shown: String| format!("{SPAWN_USAGE} (got `{shown}`)");
    match (mode.as_str(), rest) {
        ("random", [seed]) => seed.parse().map(SpawnPick::Seeded).map_err(|_| bad(seed.clone())),
        ("at", coords) if (3..=4).contains(&coords.len()) => {
            let finite = |text: &String| text.parse::<f32>().ok().filter(|v| v.is_finite());
            let Some(values) = coords.iter().map(finite).collect::<Option<Vec<f32>>>() else {
                return Err(bad(coords.join(" ")));
            };
            let yaw = values.get(3).copied().unwrap_or(0.0);
            Ok(SpawnPick::At { origin: [values[0], values[1], values[2]], yaw })
        }
        _ => Err(SPAWN_USAGE.into()),
    }
}

fn force_spawn(desk: &mut Desk, words: &[String], detached: bool) -> Result<String, String> {
    let pick = spawn_pick(words)?;
    desk.link.allow_cheats("force_spawn")?;
    let spawn_intent = |serial| PlayerIntent::ForceSpawn { request_id: serial, pick };
    let id = desk.link.queue("force_spawn", spawn_intent)?;
    if !detached {
        // Wait for a life newer than the one the player has right now.
        let client = desk.link.client();
        let current_life = desk
            .link
            .world()
            .and_then(|world| world.client_meta(client))
            .map_or_else(LifeSequence::default, |record| record.life_sequence);
        desk.dispatch.stall(Hold::Alive(client, current_life));
    }
    Ok(format!("force_spawn: queued {pick:?} request_id={id}"))
}

fn nudge(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    let [dx, dy, dz] = exact_numbers::<3>("nudge", args, NUDGE_USAGE)?;
    desk.pose("nudge")?;
    desk.link.allow_cheats("nudge")?;
    let client = desk.link.client();
    desk.link.host_mut("nudge")?.gate_nudge_origin(client, [dx, dy, dz]);
    Ok(format!("nudge: authority origin += ({dx:.1} {dy:.1} {dz:.1})"))
}

/// `name` alone shows the name the screen has for the player; `name <text>`
/// asks the host to change it (cut to the 16-byte wire field).
fn name(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    if args.len() > 1 {
        return Err(NAME_USAGE.into());
    }
    let Some(wanted) = args.first() else {
        let client = desk.link.client();
        let frame = desk.shown.snapshot();
        let record = frame.and_then(|frame| frame.meta.for_client(client));
        let shown = record.map_or("", |record| entity_t6::client_info::c_str(&record.name));
        return Ok(format!("name is \"{shown}\""));
    };
    let name = entity_t6::client_info::strncpyz::<16>(wanted);
    let id = desk
        .link
        .queue("name", |serial| PlayerIntent::SetName { request_id: serial, name })?;
    Ok(format!("name: queued `{wanted}` request_id={id}"))
}

fn force_match_start(desk: &mut Desk, detached: bool) -> Result<String, String> {
    const VERB: &str = "force_match_start";
    desk.link.allow_cheats(VERB)?;
    let phase = desk.link.host(VERB)?.phase();
    if phase == GameRoundStage::Playing {
        return Err(format!("{VERB}: already playing"));
    }
    if phase != GameRoundStage::Warmup {
        return Err(format!("{VERB}: match is not in warmup (phase={phase:?})"));
    }
    let id = desk
        .link
        .queue(VERB, |serial| PlayerIntent::StageMatch {
            request_id: serial,
            phase: GameRoundStage::Playing,
        })?;
    let mode = if detached {
        "async"
    } else {
        desk.dispatch.stall(Hold::Playing);
        "sync until Playing"
    };
    let answer = format!("{VERB}: queued SetMatchPhase Playing request_id={id} ({mode})");
    Ok(answer)
}

/// Runs each of this module's commands from the console and replies with
/// what was queued or why nothing was.
pub(crate) fn route_player_orders(
    mut incoming: MessageReader<TypedOrder>,
    mut echo: PromptReplyWriter,
    mut desk: Desk,
) {
    for order in incoming.read() {
        let (args, detached) = (order.words.as_slice(), order.detached);
        let reply = match order.name.as_str() {
            "move" | "tp" => go(&mut desk, args, detached),
            "look" => look(&mut desk, args, detached),
            "nearpiece" => near_piece(&mut desk, args, detached, last_rest()),
            "force_spawn" => force_spawn(&mut desk, args, detached),
            "force_match_start" => force_match_start(&mut desk, detached),
            "kill" => kill(&mut desk),
            "damage" => damage(&mut desk, args),
            "nudge" => nudge(&mut desk, args),
            "name" => name(&mut desk, args),
            _ignored => continue,
        };
        echo.write(reply.unwrap_or_else(|refusal| refusal));
    }
}

/// Whether the screen now shows the player where a `move` put them: feet
/// within a unit across the ground (height is the host's to settle), and
/// aim within half a degree, however the angles wrap.
pub(crate) fn presented_matches_move(presented: &ShownWorldFrame, pose: AwaitMotionPose) -> bool {
    let close_angle = |a: f32, b: f32| {
        let gap = (a - b).rem_euclid(360.0);
        gap.min(360.0 - gap) < 0.5
    };
    let found = presented.living_participant(pose.who);
    found.is_some_and(|player| {
        let close_ground = |axis: usize| (player.origin[axis] - pose.spot[axis]).abs() < 1.0;
        close_ground(0)
            && close_ground(1)
            && close_angle(player.look_angles[0], pose.facing[0])
            && close_angle(player.look_angles[1], pose.facing[1])
    })
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use net::{ActionRequestCounter, HostViewClient};

    use crate::player_cmds::*;
    use crate::tool_cmds::testing::{console_app, feed, said};
    use crate::host_link::testing::{host_app, host_world, present_player, queued};

    const HERE: [f32; 3] = [100.0, 200.0, 0.0];
    const AIM: [f32; 3] = [10.0, 90.0, 0.0];
    const NOT_ALIVE: &str = "not Alive — spawn a class first";
    /// The number the first request of a fresh app gets.
    const FIRST: IntentSerial = 1;

    /// An alive player named Tester, a queue, and no host world — so the host
    /// is somebody else's and decides about cheats itself.
    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<ViewAimAccumulator>()
            .init_resource::<TypedOrderRouter>()
            .add_systems(bevy::app::Update, route_player_orders);
        present_player(&mut app, HERE, AIM, true, "Tester");
        app
    }

    fn hosted(cheats: bool) -> App {
        let mut app = app();
        host_world(&mut app, cheats);
        app
    }

    fn client(app: &App) -> sim::ClientId {
        app.world().resource::<HostViewClient>().0
    }

    fn hold(app: &App) -> Option<Hold> {
        app.world().resource::<TypedOrderRouter>().holding()
    }

    fn aim(app: &App) -> [i32; 3] {
        app.world().resource::<ViewAimAccumulator>().angles
    }

    fn last(app: &App) -> String {
        said(app).pop().expect("the console said something")
    }

    /// `line` is answered with `reply`, queues nothing and starts no wait.
    fn refused(app: &mut App, line: &str, reply: &str) {
        let before = queued(app).len();
        let waiting = hold(app);
        feed(app, line);
        assert_eq!(last(app), reply, "{line}");
        assert_eq!(queued(app).len(), before, "{line} queued something");
        assert_eq!(hold(app), waiting, "{line} started a wait");
    }

    #[test]
    fn move_asks_the_host_to_place_the_player_and_waits_to_see_it() {
        let mut app = app();
        feed(&mut app, "move 1 2.5 -3 45 -10");
        let angles = [-10.0, 45.0, 0.0];
        let expected = [PlayerIntent::Move {
            request_id: FIRST,
            origin: [1.0, 2.5, -3.0],
            angles
        }];
        assert_eq!(queued(&app), expected);
        let actual = said(&app);
        assert_eq!(actual, [
            "move: queued (1.0 2.5 -3.0) yaw=45 pitch=-10 request_id=1 (sync until presented pose)"
        ]);
        assert_eq!(aim(&app), aim_angles_from_degrees(angles));
        let actual = hold(&app);
        assert_eq!(actual, Some(Hold::Move(AwaitMotionPose {
            who: client(&app),
            spot: [1.0, 2.5, -3.0],
            facing: angles
        })));
    }

    #[test]
    fn move_without_angles_keeps_the_way_the_player_looks() {
        let mut app = app();
        feed(&mut app, "move 5 6 7");
        let expected = [PlayerIntent::Move {
            request_id: FIRST,
            origin: [5.0, 6.0, 7.0],
            angles: AIM
        }];
        assert_eq!(queued(&app), expected);
        feed(&mut app, "move 5 6 7 180");
        assert!(last(&app).contains("yaw=180 pitch=10"), "{}", last(&app));
    }

    #[test]
    fn tp_is_move_and_a_trailing_ampersand_does_not_wait() {
        let mut app = app();
        feed(&mut app, "tp 1 2 3 &");
        assert_eq!(queued(&app).len(), 1);
        assert_eq!(hold(&app), None);
        assert!(
            last(&app).ends_with("request_id=1 (async)"),
            "{}",
            last(&app)
        );
        assert_eq!(aim(&app), aim_angles_from_degrees(AIM));
    }

    #[test]
    fn move_that_cannot_go_ahead_says_why_and_asks_for_nothing() {
        let mut app = app();
        for (line, reply) in [
            ("move 1 2", MOVE_USAGE),
            ("move 1 2 3 4 5 6", MOVE_USAGE),
            ("move", MOVE_USAGE),
            ("move 1 2 x", "move: not a finite number `x`"),
            ("move 1 2 3 inf", "move: not a finite number `inf`"),
            ("move 1 2 3 4 NaN", "move: not a finite number `NaN`"),
        ] {
            refused(&mut app, line, reply);
        }
        assert_eq!(aim(&app), [0; 3], "an aim nobody asked for");

        present_player(&mut app, HERE, AIM, false, "");
        refused(&mut app, "move 1 2 3", &format!("move: {NOT_ALIVE}"));
    }

    #[test]
    fn move_is_refused_by_a_host_with_cheats_off_and_without_a_queue() {
        let mut off = hosted(false);
        refused(&mut off, "move 1 2 3", "move: cheats are off");
        present_player(&mut off, HERE, AIM, false, "");
        refused(&mut off, "move 1 2 3", &format!("move: {NOT_ALIVE}"));

        let mut bare = console_app();
        bare.init_resource::<HostViewClient>()
            .init_resource::<ActionRequestCounter>()
            .init_resource::<ViewAimAccumulator>()
            .init_resource::<TypedOrderRouter>()
            .add_systems(bevy::app::Update, route_player_orders);
        present_player(&mut bare, HERE, AIM, true, "");
        feed(&mut bare, "move 1 2 3");
        assert_eq!(last(&bare), "move: no action inbox (not a listen host)");
        assert_eq!(hold(&bare), None);
        assert_eq!(aim(&bare), [0; 3]);
    }

    #[test]
    fn a_full_queue_is_said_and_leaves_the_aim_and_the_wait_alone() {
        let mut app = app();
        for _ in 0..crate::host_link::testing::FULL {
            feed(&mut app, "kill");
        }
        refused(
            &mut app,
            "move 1 2 3",
            "move: action queue full (32 unresolved requests)",
        );
        assert_eq!(aim(&app), [0; 3]);
    }

    #[test]
    fn look_turns_the_player_where_they_stand() {
        let mut app = app();
        feed(&mut app, "look 270 -5");
        let angles = [-5.0, 270.0, 0.0];
        let expected = [PlayerIntent::Move {
            request_id: FIRST,
            origin: HERE,
            angles
        }];
        assert_eq!(queued(&app), expected);
        let expected = ["look: queued yaw=270 pitch=-5 request_id=1 (sync until presented pose)"];
        assert_eq!(said(&app), expected);
        assert_eq!(aim(&app), aim_angles_from_degrees(angles));

        feed(&mut app, "look 0 0 &");
        assert!(last(&app).ends_with("(async)"));
    }

    #[test]
    fn look_needs_two_numbers_an_alive_player_and_cheats() {
        let mut app = hosted(true);
        for (line, reply) in [
            ("look 1", LOOK_USAGE),
            ("look 1 2 3", LOOK_USAGE),
            ("look up 2", "look: not a finite number `up`"),
        ] {
            refused(&mut app, line, reply);
        }
        present_player(&mut app, HERE, AIM, false, "");
        refused(&mut app, "look 1 2", &format!("look: {NOT_ALIVE}"));
        let mut off = hosted(false);
        refused(&mut off, "look 1 2", "look: cheats are off");
    }

    #[test]
    fn standing_near_a_piece_is_its_distance_away_on_the_players_side_looking_at_it() {
        let (feet, angles) = stand_near([100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 40.0);
        assert_eq!(feet, [40.0, 0.0, 0.0]);
        assert_eq!(angles[1].abs(), 180.0, "facing back toward it");
        let [pitch, ..] = angles;
        assert!((pitch - 55.9).abs() < 0.1, "pitch {pitch}");
        assert_eq!(angles[2], 0.0);

        // The piece's height, not the player's.
        let (feet, _) = stand_near([0.0, 30.0, 500.0], [0.0, 0.0, 8.0], 10.0);
        assert_eq!(feet, [0.0, 10.0, 8.0]);

        // Right on top of it: any side will do, and it is the same side each time.
        let (feet, angles) = stand_near([5.0, 5.0, 0.0], [5.0, 5.0, 0.0], 40.0);
        assert_eq!(feet, [45.0, 5.0, 0.0]);
        assert!(angles.iter().all(|angle| angle.is_finite()));
    }

    fn near(app: &mut App, args: &[&str], piece: Option<[f32; 3]>) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
        let world = app.world_mut();
        world
            .run_system_once(move |mut desk: Desk| near_piece(&mut desk, &args, true, piece))
            .expect("the desk's resources are there")
    }

    #[test]
    fn nearpiece_places_the_player_by_the_piece_at_the_distance_given_or_forty() {
        let mut app = app();
        let said = near(&mut app, &["20"], Some([60.0, 200.0, 8.0])).unwrap();
        let expected = "nearpiece: queued (80.0 200.0 8.0) request_id=1 (async)";
        assert_eq!(said, expected);
        let asked = queued(&app);
        let [PlayerIntent::Move { origin, .. }] = asked[..] else {
            panic!("one move, got {asked:?}");
        };
        assert_eq!(origin, [80.0, 200.0, 8.0]);

        let said = near(&mut app, &[], Some([60.0, 200.0, 8.0])).unwrap();
        assert!(
            said.starts_with("nearpiece: queued (100.0 200.0 8.0)"),
            "{said}"
        );
    }

    #[test]
    fn nearpiece_without_a_piece_a_good_distance_or_an_alive_player_asks_for_nothing() {
        let mut app = app();
        let expected = Err("nearpiece: no loose piece has come to rest".into());
        assert_eq!(near(&mut app, &[], None), expected);
        let expected = Err("nearpiece: not a finite number `far`".into());
        assert_eq!(near(&mut app, &["far"], Some([0.0; 3])), expected);
        let expected = Err(NEARPIECE_USAGE.into());
        assert_eq!(near(&mut app, &["1", "2"], Some([0.0; 3])), expected);
        present_player(&mut app, HERE, AIM, false, "");
        let expected = Err(format!("nearpiece: {NOT_ALIVE}"));
        assert_eq!(near(&mut app, &[], None), expected);
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn kill_asks_the_host_to_end_the_players_life() {
        let mut app = app();
        feed(&mut app, "kill");
        assert_eq!(queued(&app), [PlayerIntent::ForceDeath { request_id: 1 }]);
        assert_eq!(said(&app), ["kill: queued ForceDeath request_id=1"]);

        let mut off = hosted(false);
        refused(&mut off, "kill", "kill: cheats are off");
        present_player(&mut off, HERE, AIM, false, "");
        refused(&mut off, "kill", &format!("kill: {NOT_ALIVE}"));
    }

    #[test]
    fn damage_hurts_by_forty_unless_told_otherwise() {
        let mut app = app();
        feed(&mut app, "damage; damage 75");
        let expected = [
            PlayerIntent::TestHarm {
                request_id: FIRST,
                amount: 40
            },
            PlayerIntent::TestHarm {
                request_id: 2,
                amount: 75
            }
        ];
        assert_eq!(queued(&app), expected);
        let expected = [
            "damage: queued 40 request_id=1",
            "damage: queued 75 request_id=2"
        ];
        assert_eq!(said(&app), expected);
    }

    #[test]
    fn damage_that_is_not_a_positive_whole_number_for_an_alive_player_asks_for_nothing() {
        let mut app = hosted(true);
        for (line, reply) in [
            ("damage lots", "usage: damage [amount] (got `lots`)"),
            ("damage 2.5", "usage: damage [amount] (got `2.5`)"),
            ("damage 1 2", DAMAGE_USAGE),
            ("damage 0", "damage: amount must be > 0"),
            ("damage -5", "damage: amount must be > 0"),
        ] {
            refused(&mut app, line, reply);
        }
        present_player(&mut app, HERE, AIM, false, "");
        refused(&mut app, "damage", &format!("damage: {NOT_ALIVE}"));
        let mut off = hosted(false);
        refused(&mut off, "damage", "damage: cheats are off");
    }

    #[test]
    fn force_spawn_picks_a_seeded_spot_or_a_named_one_and_waits_for_the_new_life() {
        let mut app = app();
        let wait = Some(Hold::Alive(client(&app), LifeSequence::default()));
        for (line, pick) in [
            ("force_spawn", SpawnPick::Seeded(0)),
            ("force_spawn random 7", SpawnPick::Seeded(7)),
            (
                "force_spawn at 1 2 3",
                SpawnPick::At {
                    origin: [1.0, 2.0, 3.0],
                    yaw: 0.0,
                },
            ),
            (
                "force_spawn at 1 2 3 90",
                SpawnPick::At {
                    origin: [1.0, 2.0, 3.0],
                    yaw: 90.0,
                },
            ),
        ] {
            let before = queued(&app).len();
            feed(&mut app, line);
            let serial = before as u32 + 1;
            let expected = Some(PlayerIntent::ForceSpawn { request_id: serial, pick });
            assert_eq!(queued(&app).pop(), expected, "{line}");
            let expected = format!("force_spawn: queued {pick:?} request_id={serial}");
            assert_eq!(last(&app), expected, "{line}");
            assert_eq!(hold(&app), wait, "{line}");
        }
    }

    #[test]
    fn force_spawn_works_for_a_player_who_is_not_alive_and_can_carry_on() {
        let mut app = app();
        present_player(&mut app, HERE, AIM, false, "");
        feed(&mut app, "force_spawn &");
        assert_eq!(queued(&app).len(), 1);
        assert_eq!(hold(&app), None);
    }

    #[test]
    fn force_spawn_that_is_misspelled_or_refused_asks_for_nothing() {
        let mut app = app();
        for (line, reply) in [
            ("force_spawn random", SPAWN_USAGE.to_owned()),
            ("force_spawn there", SPAWN_USAGE.to_owned()),
            ("force_spawn at 1 2", SPAWN_USAGE.to_owned()),
            ("force_spawn at 1 2 3 4 5", SPAWN_USAGE.to_owned()),
            (
                "force_spawn random soon",
                format!("{SPAWN_USAGE} (got `soon`)"),
            ),
            (
                "force_spawn at 1 2 up",
                format!("{SPAWN_USAGE} (got `1 2 up`)"),
            ),
        ] {
            refused(&mut app, line, &reply);
        }
        let mut off = hosted(false);
        refused(&mut off, "force_spawn", "force_spawn: cheats are off");
    }

    #[test]
    fn nudge_shifts_the_hosts_origin_and_needs_a_host() {
        let mut app = hosted(true);
        feed(&mut app, "nudge 1 2 3");
        assert_eq!(said(&app), ["nudge: authority origin += (1.0 2.0 3.0)"]);
        assert!(queued(&app).is_empty(), "the host is changed directly");

        let mut guest = self::app();
        refused(
            &mut guest,
            "nudge 1 2 3",
            "nudge: no authority world (not a listen host)",
        );
    }

    #[test]
    fn nudge_that_cannot_go_ahead_says_why() {
        let mut app = hosted(true);
        refused(&mut app, "nudge 1 2", NUDGE_USAGE);
        refused(&mut app, "nudge 1 2 z", "nudge: not a finite number `z`");
        present_player(&mut app, HERE, AIM, false, "");
        refused(&mut app, "nudge 1 2 3", &format!("nudge: {NOT_ALIVE}"));
        let mut off = hosted(false);
        refused(&mut off, "nudge 1 2 3", "nudge: cheats are off");
    }

    #[test]
    fn name_shows_the_players_name_or_asks_the_host_to_change_it() {
        let mut app = app();
        feed(&mut app, "name");
        assert_eq!(said(&app), ["name is \"Tester\""]);

        feed(&mut app, "name Bob");
        let expected = [PlayerIntent::SetName {
            request_id: FIRST,
            name: entity_t6::client_info::strncpyz::<16>("Bob")
        }];
        assert_eq!(queued(&app), expected);
        assert_eq!(last(&app), "name: queued `Bob` request_id=1");

        present_player(&mut app, HERE, AIM, false, "");
        feed(&mut app, "name");
        assert_eq!(last(&app), "name is \"\"");
        refused(&mut app, "name a b", NAME_USAGE);
    }

    #[test]
    fn a_name_needs_neither_cheats_nor_an_alive_player() {
        let mut app = hosted(false);
        present_player(&mut app, HERE, AIM, false, "");
        feed(&mut app, "name Bob");
        assert_eq!(queued(&app).len(), 1);
    }

    #[test]
    fn force_match_start_asks_the_host_to_leave_warmup_and_waits_for_play() {
        let mut app = hosted(true);
        feed(&mut app, "force_match_start");
        let expected = [PlayerIntent::StageMatch {
            request_id: FIRST,
            phase: GameRoundStage::Playing
        }];
        assert_eq!(queued(&app), expected);
        let expected = ["force_match_start: queued SetMatchPhase Playing request_id=1 (sync until Playing)"];
        assert_eq!(said(&app), expected);
        assert_eq!(hold(&app), Some(Hold::Playing));

        let mut quick = hosted(true);
        feed(&mut quick, "force_match_start &");
        assert!(last(&quick).ends_with("(async)"));
        assert_eq!(hold(&quick), None);
    }

    #[test]
    fn force_match_start_needs_cheats_and_the_hosts_own_word_on_the_phase() {
        let mut off = hosted(false);
        refused(
            &mut off,
            "force_match_start",
            "force_match_start: cheats are off",
        );
        let mut guest = app();
        refused(
            &mut guest,
            "force_match_start",
            "force_match_start: no authority world (not a listen host)",
        );
    }

    fn pose_of(app: &App, spot: [f32; 3], aim: [f32; 3]) -> AwaitMotionPose {
        AwaitMotionPose {
            who: client(app),
            spot,
            facing: aim,
        }
    }

    #[test]
    fn a_move_is_seen_once_the_screen_is_within_a_unit_and_half_a_degree() {
        let app = app();
        let presented = app.world().resource::<ShownWorldFrame>();
        let matches = |spot: [f32; 3], aim: [f32; 3]| {
            presented_matches_move(presented, pose_of(&app, spot, aim))
        };
        assert!(matches(HERE, AIM));
        assert!(
            matches([100.9, 199.1, 0.0], [10.4, 89.6, 3.0]),
            "inside, roll ignored"
        );
        assert!(
            matches([100.0, 200.0, 5000.0], AIM),
            "height is not compared"
        );
        assert!(!matches([101.0, 200.0, 0.0], AIM));
        assert!(!matches([100.0, 198.9, 0.0], AIM));
        assert!(!matches(HERE, [10.6, 90.0, 0.0]));
        assert!(!matches(HERE, [10.0, 90.6, 0.0]));
    }

    #[test]
    fn angles_compare_around_the_circle_and_a_player_who_is_not_alive_never_matches() {
        let mut app = app();
        present_player(&mut app, HERE, [359.8, 0.2, 0.0], true, "");
        let presented = app.world().resource::<ShownWorldFrame>();
        assert!(presented_matches_move(
            presented,
            pose_of(&app, HERE, [0.2, 359.8, 0.0])
        ));
        assert!(!presented_matches_move(
            presented,
            pose_of(&app, HERE, [0.8, 0.2, 0.0])
        ));

        present_player(&mut app, HERE, AIM, false, "");
        let presented = app.world().resource::<ShownWorldFrame>();
        assert!(!presented_matches_move(presented, pose_of(&app, HERE, AIM)));
    }

    #[test]
    fn every_command_is_registered_once_and_tp_is_move() {
        let mut book = OrderCatalog::default();
        enlist(&mut book);
        enlist(&mut book);
        let lines = book.usage_listing();
        for command in [
            "move",
            "nearpiece",
            "kill",
            "force_spawn",
            "damage",
            "look",
            "name",
            "nudge",
            "force_match_start",
        ] {
            assert!(book.look_up(command).is_some(), "{command}");
            let expected = 1;
            assert_eq!(lines.iter().filter(|line| line.starts_with(command)).count(), expected, "{command}");
        }
        assert!(book.look_up("tp").is_some());
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app();
        feed(&mut app, "moves 1 2 3; killall; names");
        assert_eq!(said(&app), Vec::<String>::new());
        assert!(queued(&app).is_empty());
    }
}
