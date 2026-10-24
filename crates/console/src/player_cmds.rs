//! Commands that act on the local player through the host: put them
//! somewhere, point them, hurt or kill them, respawn them, rename them, and
//! start the match. Each asks the host for the action and says what it asked;
//! the host does the rest.
//!
//! Every command checks in the same order: what was typed, whether the player
//! is Alive (where that matters), whether the host allows cheats, then whether
//! the request can be queued. The first that fails is the answer, and nothing
//! is queued and no wait is started.

use bevy::{ecs::system::SystemParam, prelude::*};
use net::{LookState, PresentedSnapshot, look_angles_from_degrees};
use sim::{ActionRequestId, ClientAction, LifeSequence, MatchPhase, SpawnPick};

use crate::feature_dispatch::ConsoleEcho;
use crate::host_link::HostLink;
use crate::{CommandSpec, ConsoleCommand, ConsoleDispatch, ConsoleRegistry, Hold, WaitMovePose};

const MOVE_USAGE: &str = "usage: move <x> <y> <z> [yaw] [pitch]";
const LOOK_USAGE: &str = "usage: look <yaw> <pitch>";
const NEARPIECE_USAGE: &str = "usage: nearpiece [dist]";
const DAMAGE_USAGE: &str = "usage: damage [amount]";
const SPAWN_USAGE: &str = "usage: force_spawn [random <seed> | at <x> <y> <z> [yaw]]";
const NUDGE_USAGE: &str = "usage: nudge <dx> <dy> <dz>";
const NAME_USAGE: &str = "usage: name [string]";

/// How far from a resting piece `nearpiece` stands, when not told.
const PIECE_DISTANCE: f32 = 40.0;
/// How much `damage` does, when not told.
const DAMAGE: i32 = 40;
/// Eye height above the feet, for looking at a piece from where `nearpiece`
/// puts the player.
const EYE_HEIGHT: f32 = 60.0;

pub(crate) fn register(registry: &mut ConsoleRegistry) {
    let commands: [(&str, &[&str], &str); 9] = [
        (
            "move",
            &["tp"],
            "move <x> <y> <z> [yaw] [pitch] — place the player (needs cheats)",
        ),
        (
            "nearpiece",
            &[],
            "nearpiece [dist] — stand dist (40) from the last loose piece to come to rest, looking at it (needs cheats)",
        ),
        ("kill", &[], "kill — kill the player (needs cheats)"),
        (
            "force_spawn",
            &[],
            "force_spawn [random <seed> | at <x> <y> <z> [yaw]] — spawn now (needs cheats)",
        ),
        (
            "damage",
            &[],
            "damage [amount] — hurt the player, 40 if not told (needs cheats)",
        ),
        (
            "look",
            &[],
            "look <yaw> <pitch> — turn the player where they stand (needs cheats)",
        ),
        ("name", &[], "name [string] — show or set the player name"),
        (
            "nudge",
            &[],
            "nudge <dx> <dy> <dz> — shift the host's origin for the player (needs cheats)",
        ),
        (
            "force_match_start",
            &[],
            "force_match_start — end warmup and play (needs cheats)",
        ),
    ];
    for (name, aliases, usage) in commands {
        if registry.resolve(name).is_none() {
            let spec = aliases
                .iter()
                .fold(CommandSpec::new(name), |spec, &alias| spec.alias(alias));
            registry.register(spec.usage(usage));
        }
    }
}

/// Everything the commands touch: what the screen shows, the host link, where
/// the player looks, and the console's waits.
#[derive(SystemParam)]
pub(crate) struct Desk<'w> {
    presented: Res<'w, PresentedSnapshot>,
    link: HostLink<'w>,
    look: ResMut<'w, LookState>,
    dispatch: ResMut<'w, ConsoleDispatch>,
}

/// Where the local player stands and looks.
struct Pose {
    origin: [f32; 3],
    angles: [f32; 3],
}

impl Desk<'_> {
    fn pose(&self, verb: &str) -> Result<Pose, String> {
        self.presented
            .alive_player(self.link.client())
            .map(|player| Pose {
                origin: player.origin,
                angles: player.viewangles,
            })
            .ok_or_else(|| format!("{verb}: not Alive — spawn a class first"))
    }

    /// Asks the host to put the player there, points the player's own aim the
    /// same way, and — unless told to carry on — makes the console wait until
    /// the screen shows it.
    fn place(
        &mut self,
        verb: &str,
        background: bool,
        origin: [f32; 3],
        angles: [f32; 3],
    ) -> Result<(ActionRequestId, &'static str), String> {
        self.link.allow_cheats(verb)?;
        let request_id = self.link.queue(verb, |request_id| ClientAction::Move {
            request_id,
            origin,
            angles,
        })?;
        self.look.angles = look_angles_from_degrees(angles);
        if background {
            return Ok((request_id, "async"));
        }
        self.dispatch.hold(Hold::Move(WaitMovePose {
            client: self.link.client(),
            origin,
            angles,
        }));
        Ok((request_id, "sync until presented pose"))
    }
}

fn number(verb: &str, text: &str) -> Result<f32, String> {
    text.parse::<f32>()
        .ok()
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("{verb}: not a finite number `{text}`"))
}

fn numbers(verb: &str, args: &[String]) -> Result<Vec<f32>, String> {
    args.iter().map(|text| number(verb, text)).collect()
}

fn go(desk: &mut Desk, args: &[String], background: bool) -> Result<String, String> {
    if !(3..=5).contains(&args.len()) {
        return Err(MOVE_USAGE.into());
    }
    let given = numbers("move", args)?;
    let pose = desk.pose("move")?;
    let origin = [given[0], given[1], given[2]];
    let yaw = given.get(3).copied().unwrap_or(pose.angles[1]);
    let pitch = given.get(4).copied().unwrap_or(pose.angles[0]);
    let (request_id, how) = desk.place("move", background, origin, [pitch, yaw, pose.angles[2]])?;
    Ok(format!(
        "move: queued ({:.1} {:.1} {:.1}) yaw={yaw:.0} pitch={pitch:.0} request_id={request_id} ({how})",
        origin[0], origin[1], origin[2],
    ))
}

fn look(desk: &mut Desk, args: &[String], background: bool) -> Result<String, String> {
    if args.len() != 2 {
        return Err(LOOK_USAGE.into());
    }
    let given = numbers("look", args)?;
    let (yaw, pitch) = (given[0], given[1]);
    let pose = desk.pose("look")?;
    let (request_id, how) = desk.place(
        "look",
        background,
        pose.origin,
        [pitch, yaw, pose.angles[2]],
    )?;
    Ok(format!(
        "look: queued yaw={yaw:.0} pitch={pitch:.0} request_id={request_id} ({how})"
    ))
}

/// Where to put a player who stands at `player` so they are `dist` from
/// `piece` on the side they are on, at the piece's height, looking at it.
fn stand_near(player: [f32; 3], piece: [f32; 3], dist: f32) -> ([f32; 3], [f32; 3]) {
    let (dx, dy) = (player[0] - piece[0], player[1] - piece[1]);
    let len = dx.hypot(dy);
    let (ux, uy) = if len > 0.01 {
        (dx / len, dy / len)
    } else {
        (1.0, 0.0)
    };
    let origin = [piece[0] + ux * dist, piece[1] + uy * dist, piece[2]];
    let to = [
        piece[0] - origin[0],
        piece[1] - origin[1],
        piece[2] + 1.0 - (origin[2] + EYE_HEIGHT),
    ];
    let yaw = to[1].atan2(to[0]).to_degrees();
    let pitch = -to[2].atan2(to[0].hypot(to[1])).to_degrees();
    (origin, [pitch, yaw, 0.0])
}

/// The last loose piece to come to rest, as the animation system noted it.
fn last_rest() -> Option<[f32; 3]> {
    render_frontend::adapters::anim::dyn_ent_phys::LAST_REST
        .lock()
        .ok()
        .and_then(|rest| *rest)
}

fn near_piece(
    desk: &mut Desk,
    args: &[String],
    background: bool,
    piece: Option<[f32; 3]>,
) -> Result<String, String> {
    let dist = match args {
        [] => PIECE_DISTANCE,
        [text] => number("nearpiece", text)?,
        _ => return Err(NEARPIECE_USAGE.into()),
    };
    let pose = desk.pose("nearpiece")?;
    let piece = piece.ok_or("nearpiece: no loose piece has come to rest")?;
    let (origin, angles) = stand_near(pose.origin, piece, dist);
    diag::info!(
        Console,
        "nearpiece: piece at {piece:?}, standing at {origin:?}"
    );
    let (request_id, how) = desk.place("nearpiece", background, origin, angles)?;
    Ok(format!(
        "nearpiece: queued ({:.1} {:.1} {:.1}) request_id={request_id} ({how})",
        origin[0], origin[1], origin[2],
    ))
}

fn kill(desk: &mut Desk) -> Result<String, String> {
    desk.pose("kill")?;
    desk.link.allow_cheats("kill")?;
    let request_id = desk
        .link
        .queue("kill", |request_id| ClientAction::ForceDeath { request_id })?;
    Ok(format!("kill: queued ForceDeath request_id={request_id}"))
}

fn damage(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    let amount = match args {
        [] => DAMAGE,
        [text] => text
            .parse::<i32>()
            .map_err(|_| format!("{DAMAGE_USAGE} (got `{text}`)"))?,
        _ => return Err(DAMAGE_USAGE.into()),
    };
    if amount <= 0 {
        return Err("damage: amount must be > 0".into());
    }
    desk.pose("damage")?;
    desk.link.allow_cheats("damage")?;
    let request_id = desk
        .link
        .queue("damage", |request_id| ClientAction::DebugDamage {
            request_id,
            amount,
        })?;
    Ok(format!("damage: queued {amount} request_id={request_id}"))
}

fn spawn_pick(args: &[String]) -> Result<SpawnPick, String> {
    let bad = |text: &str| format!("{SPAWN_USAGE} (got `{text}`)");
    match args {
        [] => Ok(SpawnPick::Seeded(0)),
        [word, seed] if word == "random" => seed
            .parse()
            .map(SpawnPick::Seeded)
            .map_err(|_| bad(seed.as_str())),
        [word, rest @ ..] if word == "at" && matches!(rest.len(), 3 | 4) => {
            let at = rest
                .iter()
                .map(|text| text.parse::<f32>().ok().filter(|number| number.is_finite()))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad(&rest.join(" ")))?;
            Ok(SpawnPick::At {
                origin: [at[0], at[1], at[2]],
                yaw: at.get(3).copied().unwrap_or(0.0),
            })
        }
        _ => Err(SPAWN_USAGE.into()),
    }
}

fn force_spawn(desk: &mut Desk, args: &[String], background: bool) -> Result<String, String> {
    let pick = spawn_pick(args)?;
    desk.link.allow_cheats("force_spawn")?;
    let request_id = desk
        .link
        .queue("force_spawn", |request_id| ClientAction::ForceSpawn {
            request_id,
            pick,
        })?;
    if !background {
        let client = desk.link.client();
        let life = desk
            .link
            .world()
            .and_then(|world| world.client_meta(client))
            .map_or_else(LifeSequence::default, |meta| meta.life_sequence);
        desk.dispatch.hold(Hold::Alive(client, life));
    }
    Ok(format!(
        "force_spawn: queued {pick:?} request_id={request_id}"
    ))
}

fn nudge(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    if args.len() != 3 {
        return Err(NUDGE_USAGE.into());
    }
    let given = numbers("nudge", args)?;
    let delta = [given[0], given[1], given[2]];
    desk.pose("nudge")?;
    desk.link.allow_cheats("nudge")?;
    let client = desk.link.client();
    desk.link
        .host_mut("nudge")?
        .gate_nudge_origin(client, delta);
    Ok(format!(
        "nudge: authority origin += ({:.1} {:.1} {:.1})",
        delta[0], delta[1], delta[2]
    ))
}

fn name(desk: &mut Desk, args: &[String]) -> Result<String, String> {
    match args {
        [] => {
            let client = desk.link.client();
            let shown = desk
                .presented
                .snapshot()
                .and_then(|snapshot| snapshot.meta.for_client(client))
                .map_or("", |meta| entity_t6::client_state::c_str(&meta.name));
            Ok(format!("name is \"{shown}\""))
        }
        [typed] => {
            let packed = entity_t6::client_state::strncpyz::<16>(typed);
            let request_id = desk
                .link
                .queue("name", |request_id| ClientAction::SetName {
                    request_id,
                    name: packed,
                })?;
            Ok(format!("name: queued `{typed}` request_id={request_id}"))
        }
        _ => Err(NAME_USAGE.into()),
    }
}

fn force_match_start(desk: &mut Desk, background: bool) -> Result<String, String> {
    const VERB: &str = "force_match_start";
    desk.link.allow_cheats(VERB)?;
    match desk.link.host(VERB)?.phase() {
        MatchPhase::Warmup => {}
        MatchPhase::Playing => return Err(format!("{VERB}: already playing")),
        other => return Err(format!("{VERB}: match is not in warmup (phase={other:?})")),
    }
    let request_id = desk
        .link
        .queue(VERB, |request_id| ClientAction::SetMatchPhase {
            request_id,
            phase: MatchPhase::Playing,
        })?;
    if background {
        return Ok(format!(
            "{VERB}: queued SetMatchPhase Playing request_id={request_id} (async)"
        ));
    }
    desk.dispatch.hold(Hold::Playing);
    Ok(format!(
        "{VERB}: queued SetMatchPhase Playing request_id={request_id} (sync until Playing)"
    ))
}

pub(crate) fn route(
    mut commands: MessageReader<ConsoleCommand>,
    mut echo: ConsoleEcho,
    mut desk: Desk,
) {
    for command in commands.read() {
        let args = command.args.as_slice();
        let background = command.background;
        let said = match command.name.as_str() {
            "move" | "tp" => go(&mut desk, args, background),
            "look" => look(&mut desk, args, background),
            "nearpiece" => near_piece(&mut desk, args, background, last_rest()),
            "kill" => kill(&mut desk),
            "damage" => damage(&mut desk, args),
            "force_spawn" => force_spawn(&mut desk, args, background),
            "nudge" => nudge(&mut desk, args),
            "name" => name(&mut desk, args),
            "force_match_start" => force_match_start(&mut desk, background),
            _ => continue,
        };
        echo.write(said.unwrap_or_else(|why| why));
    }
}

/// Whether the screen now shows the player where a `move` put them: feet
/// within a unit across the ground (height is the host's to settle), and
/// aim within half a degree, however the angles wrap.
pub(crate) fn presented_matches_move(presented: &PresentedSnapshot, pose: WaitMovePose) -> bool {
    let Some(player) = presented.alive_player(pose.client) else {
        return false;
    };
    let same_angle = |a: f32, b: f32| {
        let gap = (a - b).rem_euclid(360.0);
        gap.min(360.0 - gap) < 0.5
    };
    (player.origin[0] - pose.origin[0]).abs() < 1.0
        && (player.origin[1] - pose.origin[1]).abs() < 1.0
        && same_angle(player.viewangles[0], pose.angles[0])
        && same_angle(player.viewangles[1], pose.angles[1])
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use net::{ActionRequestIds, LocalPresentClient};

    use super::*;
    use crate::feature_dispatch::testing::{console_app, feed, said};
    use crate::host_link::testing::{host_app, host_world, present_player, queued};

    const HERE: [f32; 3] = [100.0, 200.0, 0.0];
    const AIM: [f32; 3] = [10.0, 90.0, 0.0];
    const NOT_ALIVE: &str = "not Alive — spawn a class first";

    /// An alive player named Tester, a queue, and no host world — so the host
    /// is somebody else's and decides about cheats itself.
    fn app() -> App {
        let mut app = host_app();
        app.init_resource::<LookState>()
            .init_resource::<ConsoleDispatch>()
            .add_systems(Update, route);
        present_player(&mut app, HERE, AIM, true, "Tester");
        app
    }

    fn hosted(cheats: bool) -> App {
        let mut app = app();
        host_world(&mut app, cheats);
        app
    }

    fn client(app: &App) -> sim::ClientId {
        app.world().resource::<LocalPresentClient>().0
    }

    fn hold(app: &App) -> Option<Hold> {
        app.world().resource::<ConsoleDispatch>().holding()
    }

    fn aim(app: &App) -> [i32; 3] {
        app.world().resource::<LookState>().angles
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
        assert_eq!(
            queued(&app),
            [ClientAction::Move {
                request_id: 1,
                origin: [1.0, 2.5, -3.0],
                angles
            }]
        );
        assert_eq!(
            said(&app),
            [
                "move: queued (1.0 2.5 -3.0) yaw=45 pitch=-10 request_id=1 (sync until presented pose)"
            ]
        );
        assert_eq!(aim(&app), look_angles_from_degrees(angles));
        assert_eq!(
            hold(&app),
            Some(Hold::Move(WaitMovePose {
                client: client(&app),
                origin: [1.0, 2.5, -3.0],
                angles
            }))
        );
    }

    #[test]
    fn move_without_angles_keeps_the_way_the_player_looks() {
        let mut app = app();
        feed(&mut app, "move 5 6 7");
        assert_eq!(
            queued(&app),
            [ClientAction::Move {
                request_id: 1,
                origin: [5.0, 6.0, 7.0],
                angles: AIM
            }]
        );
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
        assert_eq!(aim(&app), look_angles_from_degrees(AIM));
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
        bare.init_resource::<LocalPresentClient>()
            .init_resource::<ActionRequestIds>()
            .init_resource::<LookState>()
            .init_resource::<ConsoleDispatch>()
            .add_systems(Update, route);
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
        assert_eq!(
            queued(&app),
            [ClientAction::Move {
                request_id: 1,
                origin: HERE,
                angles
            }]
        );
        assert_eq!(
            said(&app),
            ["look: queued yaw=270 pitch=-5 request_id=1 (sync until presented pose)"]
        );
        assert_eq!(aim(&app), look_angles_from_degrees(angles));

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
        let (origin, angles) = stand_near([100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 40.0);
        assert_eq!(origin, [40.0, 0.0, 0.0]);
        assert_eq!(angles[1].abs(), 180.0, "facing back toward it");
        assert!((angles[0] - 55.9).abs() < 0.1, "pitch {}", angles[0]);
        assert_eq!(angles[2], 0.0);

        // The piece's height, not the player's.
        let (origin, _) = stand_near([0.0, 30.0, 500.0], [0.0, 0.0, 8.0], 10.0);
        assert_eq!(origin, [0.0, 10.0, 8.0]);

        // Right on top of it: any side will do, and it is the same side each time.
        let (origin, angles) = stand_near([5.0, 5.0, 0.0], [5.0, 5.0, 0.0], 40.0);
        assert_eq!(origin, [45.0, 5.0, 0.0]);
        assert!(angles.iter().all(|angle| angle.is_finite()));
    }

    fn near(app: &mut App, args: &[&str], piece: Option<[f32; 3]>) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
        app.world_mut()
            .run_system_once(move |mut desk: Desk| near_piece(&mut desk, &args, true, piece))
            .expect("the desk's resources are there")
    }

    #[test]
    fn nearpiece_places_the_player_by_the_piece_at_the_distance_given_or_forty() {
        let mut app = app();
        let said = near(&mut app, &["20"], Some([60.0, 200.0, 8.0])).unwrap();
        assert_eq!(
            said,
            "nearpiece: queued (80.0 200.0 8.0) request_id=1 (async)"
        );
        let asked = queued(&app);
        let [ClientAction::Move { origin, .. }] = asked[..] else {
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
        assert_eq!(
            near(&mut app, &[], None),
            Err("nearpiece: no loose piece has come to rest".into())
        );
        assert_eq!(
            near(&mut app, &["far"], Some([0.0; 3])),
            Err("nearpiece: not a finite number `far`".into())
        );
        assert_eq!(
            near(&mut app, &["1", "2"], Some([0.0; 3])),
            Err(NEARPIECE_USAGE.into())
        );
        present_player(&mut app, HERE, AIM, false, "");
        assert_eq!(
            near(&mut app, &[], None),
            Err(format!("nearpiece: {NOT_ALIVE}"))
        );
        assert!(queued(&app).is_empty());
    }

    #[test]
    fn kill_asks_the_host_to_end_the_players_life() {
        let mut app = app();
        feed(&mut app, "kill");
        assert_eq!(queued(&app), [ClientAction::ForceDeath { request_id: 1 }]);
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
        assert_eq!(
            queued(&app),
            [
                ClientAction::DebugDamage {
                    request_id: 1,
                    amount: 40
                },
                ClientAction::DebugDamage {
                    request_id: 2,
                    amount: 75
                }
            ]
        );
        assert_eq!(
            said(&app),
            [
                "damage: queued 40 request_id=1",
                "damage: queued 75 request_id=2"
            ]
        );
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
            let request_id = before as u32 + 1;
            assert_eq!(
                queued(&app).pop(),
                Some(ClientAction::ForceSpawn { request_id, pick }),
                "{line}"
            );
            assert_eq!(
                last(&app),
                format!("force_spawn: queued {pick:?} request_id={request_id}"),
                "{line}"
            );
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
        assert_eq!(
            queued(&app),
            [ClientAction::SetName {
                request_id: 1,
                name: entity_t6::client_state::strncpyz::<16>("Bob")
            }]
        );
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
        assert_eq!(
            queued(&app),
            [ClientAction::SetMatchPhase {
                request_id: 1,
                phase: MatchPhase::Playing
            }]
        );
        assert_eq!(
            said(&app),
            ["force_match_start: queued SetMatchPhase Playing request_id=1 (sync until Playing)"]
        );
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

    fn pose_of(app: &App, origin: [f32; 3], angles: [f32; 3]) -> WaitMovePose {
        WaitMovePose {
            client: client(app),
            origin,
            angles,
        }
    }

    #[test]
    fn a_move_is_seen_once_the_screen_is_within_a_unit_and_half_a_degree() {
        let app = app();
        let presented = app.world().resource::<PresentedSnapshot>();
        let matches = |origin: [f32; 3], angles: [f32; 3]| {
            presented_matches_move(presented, pose_of(&app, origin, angles))
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
        let presented = app.world().resource::<PresentedSnapshot>();
        assert!(presented_matches_move(
            presented,
            pose_of(&app, HERE, [0.2, 359.8, 0.0])
        ));
        assert!(!presented_matches_move(
            presented,
            pose_of(&app, HERE, [0.8, 0.2, 0.0])
        ));

        present_player(&mut app, HERE, AIM, false, "");
        let presented = app.world().resource::<PresentedSnapshot>();
        assert!(!presented_matches_move(presented, pose_of(&app, HERE, AIM)));
    }

    #[test]
    fn every_command_is_registered_once_and_tp_is_move() {
        let mut registry = ConsoleRegistry::default();
        register(&mut registry);
        register(&mut registry);
        let lines = registry.help_lines();
        for name in [
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
            assert!(registry.resolve(name).is_some(), "{name}");
            assert_eq!(
                lines.iter().filter(|line| line.starts_with(name)).count(),
                1,
                "{name}"
            );
        }
        assert!(registry.resolve("tp").is_some());
    }

    #[test]
    fn other_commands_are_not_ours() {
        let mut app = app();
        feed(&mut app, "moves 1 2 3; killall; names");
        assert_eq!(said(&app), Vec::<String>::new());
        assert!(queued(&app).is_empty());
    }
}
