//! bo2mp: the engine half of Black Ops II's bots.
//!
//! The game's own bot scripts (`maps/mp/bots/_bot*`) decide everything a
//! bot does: where to go (named goals with priorities: "wander",
//! "enemy_patrol", "cover"...), whom to fight (`getthreats`), where to look,
//! when the trigger may be pulled, when to aim down the sights, throw,
//! reload, melee or dive. The engine carries it out: a test client joins
//! (`addtestclient`), walks the best goal's route over the map's path
//! nodes, turns its view toward what it looks at (or down its route) at its
//! difficulty's speed, and presses the buttons. Each tick a bot's command
//! replaces its client's own, as the zombies test player does.

use super::pairs::KeepByValue;
use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{Array, ObjKind, ObjRef, ScriptValue, Vm};
use playerstate_t6::game::buttons;

use super::{Closest, Survivor, Zm, arg, entnum, frame, list, origin_of, text};
use crate::sim_parts::ClientId;

/// A named place a bot wants to be (`addgoal`): the highest priority wins,
/// the newest among equals.
#[derive(Clone, Debug)]
struct Goal {
    name: String,
    pos: [f32; 3],
    radius: f32,
    priority: i32,
    order: u64,
}

/// What a bot's scripts set up for its custom classes (`botclassadditem`,
/// `botclassaddattachment`, `botsetdefaultclass`): read by the loadout.
#[derive(Clone, Debug, Default)]
pub(crate) struct BotClass {
    pub default: Option<String>,
    pub items: Vec<String>,
    /// (the gun, its add-on, and the slot it fills, e.g. "primaryattachment1").
    pub attachments: Vec<(String, String, String)>,
}

/// A negotiation link on a route (BO2's path nodes mark ledges, railings,
/// ladders and drops with a begin node linked to an end node): the begin
/// node's spawnflags say how it is taken (0x800000 a jump, 0x400000 up a
/// ladder, 0x2000000 a drop run straight off; none of those, a mantle
/// named by its animscript). Measured over all 30 BO2 maps: every 0x400000
/// link climbs (60 to 284 units) with ladder collision at it, and every
/// 0x2000000 link goes down.
#[derive(Clone, Copy, Debug)]
struct NegLink {
    entry: [f32; 3],
    end: [f32; 3],
    flags: u32,
    /// The node the route planned once the link is done starts from (BO2's
    /// Bot_GetPathToGoal: the route point after the link's end; the end's
    /// own node when the route ends there).
    after_node: u32,
}

const NEG_JUMP: u32 = 0x80_0000;
const NEG_LADDER: u32 = 0x40_0000;
const NEG_DROP: u32 = 0x200_0000;

/// A grenade throw: where (none: wherever it faces), its button (SAMPLE_FRAG_ASSET a
/// lethal, OBSCURANT a tactical), when it was asked for, when the button went
/// down (0: not yet) and whether the hand has started the throw.
#[derive(Clone, Copy, Debug)]
struct Throw {
    at: Option<[f32; 3]>,
    button: u32,
    asked: i64,
    pressed: i64,
    underway: bool,
}

/// A throw waits until the view is this close to the point (degrees, yaw
/// and pitch), or this long (ms); one still not in the hand this long after
/// it was asked for is dropped.
const THROW_AIM_DEG: f32 = 5.0;
const THROW_AIM_MS: i64 = 1000;
const THROW_GIVE_UP_MS: i64 = 3000;
/// BO2's Bot_TryThrowGrenade: each throw it takes up holds the next off
/// this long (its aim's grenadeTime).
const THROW_WAIT_MS: i64 = 1000;

/// BO2's Bot_TryThrowGrenade: a throw is taken only with ammo for it, none
/// still waiting to leave the hand, and the wait after the last one over.
/// A second ask while one waits is refused, not swapped in (bot_combat's
/// lethal-then-tactical pair throws the lethal).
fn takes_throw(ammo: i32, queued: bool, wait_until: i64, now_ms: i64) -> bool {
    ammo > 0 && !queued && wait_until <= now_ms
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Bot {
    goals: Vec<Goal>,
    order: u64,
    /// Route to the best goal: points, the goal last.
    path: Vec<[f32; 3]>,
    path_to: Option<[f32; 3]>,
    replan_at: i64,
    look: Option<[f32; 3]>,
    allow_attack: bool,
    ads: bool,
    attack_ticks: u32,
    use_until: i64,
    /// A grenade `throwgrenade` asked for, until it has left the hand.
    throw: Option<Throw>,
    /// No new throw is taken before then ([`THROW_WAIT_MS`]).
    throw_wait: i64,
    /// The gun its commands ask for and the gun it last saw in hand, kept
    /// as a player's client keeps them ([`follow_select`]).
    select: u32,
    select_held: u32,
    melee: bool,
    dive: bool,
    /// 0 stand, 1 crouch, 2 prone (`setstance`).
    stance: u8,
    no_sprint: bool,
    /// The node `botsetfailsafenode` gave it (its place, and the node
    /// handed back with "goal"): it walks there and nothing else.
    failsafe: Option<([f32; 3], Option<ObjRef>)>,
    tick: u32,
    /// Where it was when it last made headway, and when (stuck checks).
    last_gain: Option<([f32; 3], i64)>,
    /// The route point it was stuck short of, and when that stuck began.
    stuck_next: Option<([f32; 3], i64)>,
    /// The point its route came from to `path[0]` (where it planned, or
    /// the last point it passed): it has passed `path[0]` once it is
    /// beyond the line across that point.
    prev: Option<[f32; 3]>,
    /// Counts new routes; with the route's length it names the step a
    /// cached far-sight answer belongs to.
    path_gen: u32,
    path_new: bool,
    /// BO2's lookahead amount for this route (32768 when planned, 0 =
    /// not set) and the last lookahead point the walk there was clear to.
    la_amount: f32,
    la_pos: Option<[f32; 3]>,
    /// BO2's lookahead direction (its z the rise per unit) and distance to
    /// the point the walk headed for at its last think, and whether that
    /// walk was clear: `getlookaheaddir`/`dist` answer from them, and route
    /// points behind it along them are done.
    la_dir: Option<([f32; 3], f32)>,
    /// That point: route points are done only once the bot is level with
    /// or past it (BO2's Path_UpdateLookahead trims no sooner).
    la_at: Option<[f32; 3]>,
    la_clear: bool,
    /// The farthest route point seen from the one it heads to, for
    /// (route, points left, within 192 units of the next).
    seen: Option<(u32, usize, bool, [f32; 3])>,
    /// BO2's steering: the angles its view chases (pitch, yaw), smoothed
    /// toward what it wants to face, and the current turn speeds
    /// (degrees a second) that ramp up toward it.
    aim_avg: Option<[f32; 2]>,
    turn_speed: [f32; 2],
    /// A semi-automatic or burst weapon's trigger rests until then.
    fire_delay_until: i64,
    /// When its current sprint began.
    sprint_since: Option<i64>,
    /// The negotiation links its route crosses, the one it is taking (and
    /// since when), whether it has been at that link's begin (BO2's bot
    /// flag 0x10), last tick's buttons, and whether it was mantling.
    negs: Vec<NegLink>,
    neg: Option<(NegLink, i64)>,
    neg_began: bool,
    /// The node the next route starts from: a link's end once the link is
    /// done (BO2's Bot_GetPathToGoal from the negotiation end).
    replan_from: Option<u32>,
    last_press: u32,
    was_mantling: bool,
    /// BO2MP_BOTFIRELOG: the weapon it held and its magazine at the last
    /// tick, to tell whether the trigger pull before it let a shot out.
    fire_dbg: Option<(u32, i32)>,
    pub classes: BTreeMap<i32, BotClass>,
}

impl Bot {
    /// The level restarts (`map_restart`): what its scripts set up for its
    /// classes stays; where it was going and what it was doing go.
    pub(crate) fn restarted(self) -> Self {
        Self {
            classes: self.classes,
            ..Self::default()
        }
    }

    fn best_goal(&self) -> Option<&Goal> {
        self.goals
            .iter()
            .max_by_key(|g| (g.priority, g.order))
    }
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn dist2d(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// How high above its origin a target's middle is: a ground car is low.
fn centroid_height(world: &World, n: u32) -> f32 {
    if super::killstreaks::ground_vehicle(world, n) { 8.0 } else { 32.0 }
}

/// Yaw and pitch, degrees, the game's: pitch down positive.
struct LookAngles {
    yaw: f32,
    pitch: f32,
}

/// The look angles from a to b.
fn angles_to(lhs: [f32; 3], rhs: [f32; 3]) -> LookAngles {
    let d = [rhs[0] - lhs[0], rhs[1] - lhs[1], rhs[2] - lhs[2]];
    let yaw = d[1].atan2(d[0]).to_degrees();
    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let pitch = -d[2].atan2(flat).to_degrees();
    LookAngles { yaw, pitch }
}

fn angle_delta(from: f32, to: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// Weapon states (the movement code's numbering): reloading, any kind.
/// Reloading, by BO2's gun state (a dual-wield gun's one-hand reloads too).
fn reloading(state: i32) -> bool {
    weapon_t6::state::is_any_reload(state)
}

/// A player's state, as `read` sees it (not a player: false).
fn player_is(vm: &Vm<World>, sim: &mut World, st: &ScriptValue, read: fn(&playerstate_t6::game::AvatarState) -> bool) -> ScriptValue {
    let n = entnum(vm, st).unwrap_or(u32::MAX);
    ScriptValue::bool(frame(sim).avatar(ClientId(n)).is_some_and(read))
}

/// The danger count's slot for a team (BO2 keeps one per team).
fn danger_slot(team: &str) -> Option<usize> {
    match team {
        "allies" => Some(0),
        "axis" => Some(1),
        _ => None,
    }
}

/// The settings BO2's engine drives bots by (its own dvars; the bot
/// scripts raise `bot_yawspeed` and set the fire gaps per difficulty).
struct BotDvars {
    heading_turn_rate: f32,
    min_fire: i64,
    max_fire: i64,
    sprint_distance: f32,
    /// aim_turnrate_pitch / _ads, aim_turnrate_yaw / _ads (degrees a
    /// second) and aim_accel_turnrate_lerp (degrees a second, a second).
    pitch_rate: [f32; 2],
    yaw_rate: [f32; 2],
    speed_up_rate: f32,
}

impl BotDvars {
    fn read(world: &mut World) -> Self {
        super::with_vm(world, |vm, _| {
            let f = |k: &str, d: f32| {
                vm.dvars
                    .get(k)
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(d)
            };
            let (min_fire, max_fire) = (f("bot_minfiretime", 400.0) as i64, f("bot_maxfiretime", 600.0) as i64);
            Self {
                heading_turn_rate: f("bot_yawspeed", 1.0),
                min_fire,
                max_fire: max_fire.max(min_fire),
                sprint_distance: f("bot_sprintdistance", 256.0),
                pitch_rate: [f("aim_turnrate_pitch", 90.0), f("aim_turnrate_pitch_ads", 55.0)],
                yaw_rate: [f("aim_turnrate_yaw", 260.0), f("aim_turnrate_yaw_ads", 90.0)],
                speed_up_rate: f("aim_accel_turnrate_lerp", 1200.0),
            }
        })
        .unwrap_or(Self {
            heading_turn_rate: 1.0,
            min_fire: 400,
            max_fire: 600,
            sprint_distance: 256.0,
            pitch_rate: [90.0, 55.0],
            yaw_rate: [260.0, 90.0],
            speed_up_rate: 1200.0,
        })
    }
}

/// A number in lo..hi from the bot and the time (splitmix).
fn irand(n: u32, now: i64, lo: i64, hi: i64) -> i64 {
    const MIX_1: u64 = 0xBF58_476D_1CE4_E5B9;
    const MIX_2: u64 = 0x94D0_49BB_1331_11EB;
    let span = hi - lo;
    if span < 1 {
        return lo;
    }
    let mut z = (now as u64) ^ ((n as u64) << 40) ^ 0x9E37_79B9_7F4A_7C15;
    z = (z ^ z >> 30).wrapping_mul(MIX_1);
    z = (z ^ z >> 27).wrapping_mul(MIX_2);
    z ^= z >> 31;
    lo + (z % span as u64) as i64
}

/// BO2's lookahead down a route: along the route from where the bot is
/// until the corner it would cut (each leg's length times how far its line
/// passes from the bot, summed) reaches the amount; straight legs cost
/// nothing. The point there, and how far it is.
fn lookahead(origin: [f32; 3], path: &[[f32; 3]], amount: f32) -> Option<([f32; 3], f32)> {
    const AMOUNT_START: f32 = 32768.0;
    let amount = if amount > 0.0 { amount } else { AMOUNT_START };
    let mut area = 0.0f32;
    let mut a = origin;
    for &b in path {
        let d = [b[0] - a[0], b[1] - a[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len < 1e-3 {
            a = b;
            continue;
        }
        let dir = [d[0] / len, d[1] / len];
        let off = (dir[0] * (b[1] - origin[1]) - dir[1] * (b[0] - origin[0])).abs();
        let before = area;
        area += len * off;
        if off > 0.0 && area >= amount {
            let s = ((amount - before) / off).min(len) / len;
            let p = [a[0] + d[0] * s, a[1] + d[1] * s, a[2] + (b[2] - a[2]) * s];
            return Some((p, dist2d(origin, p)));
        }
        a = b;
    }
    path.last().map(|&p| (p, dist2d(origin, p)))
}

/// BO2's far sight down a route: from the point it heads to (the one
/// after when that is within 192 units), the farthest later point seen
/// from there; it looks toward that while it walks.
fn farthest_seen(game: &mut World, spot: [f32; 3], route: &[[f32; 3]]) -> Option<[f32; 3]> {
    // The nodes: all but the goal point at the end.
    let nodes = &route[..route.len().saturating_sub(1)];
    let start = usize::from(nodes.len() > 1 && dist2d(spot, nodes[1]) < 192.0);
    let raise = |p: [f32; 3]| [p[0], p[1], p[2] + 60.0];
    let from = raise(*nodes.get(start)?);
    let last = nodes.len().checked_sub(1)?.min(start + 20);
    nodes
        .get(start + 1..=last)?
        .iter()
        .rev()
        .copied()
        .find(|&p| dist2d(nodes[start], p) < 2048.0 && sight(game, from, raise(p)))
}

/// BO2's bot steering, one 50 ms tick: the view (pitch, yaw) turns toward
/// the angles it chases at a speed that scales with how far off it is
/// (a quarter second to close the gap from the hip, a fifth aimed in), up
/// to the turn rates, ramping up by the acceleration and dropping at once.
fn steer(view: [f32; 2], aim: [f32; 2], speed: &mut [f32; 2], ads: f32, dv: &BotDvars) -> [f32; 2] {
    let lerp = |r: [f32; 2]| r[0] + (r[1] - r[0]) * ads;
    let time = 0.2 * ads + 0.25 * (1.0 - ads);
    let rate = [lerp(dv.pitch_rate), lerp(dv.yaw_rate)];
    let mut axis: [f32; 2] =
        std::array::from_fn(|i| (angle_delta(view[i], aim[i]) / (time * rate[i]).max(1e-3)).clamp(-1.0, 1.0));
    // The smaller axis gives way to the larger (aim_scale_view_axis).
    let (p, y) = (axis[0].abs(), axis[1].abs());
    if y < p {
        axis[1] *= 1.0 - (p - y);
    } else {
        axis[0] *= 1.0 - (y - p);
    }
    let mut out = view;
    for i in 0..2 {
        let target = axis[i].abs() * rate[i] * dv.heading_turn_rate;
        speed[i] = if target <= speed[i] {
            target
        } else {
            (speed[i] + dv.speed_up_rate * dv.heading_turn_rate * 0.05).min(target)
        };
        out[i] = view[i] + speed[i] * 0.05 * if axis[i] >= 0.0 { 1.0 } else { -1.0 };
    }
    out
}

/// BO2's crowd avoidance: each bot keeps a circle (30 units; a player's is
/// 50) and picks the velocity nearest the way it wants to go that misses
/// its teammates for the next two seconds (reciprocal velocity obstacles).
const AVOID_HORIZON: f32 = 2.0;
const BOT_AVOID_RADIUS: f32 = 30.0;
const BOT_AVOID_SPEED: f32 = 100.0;
const PLAYER_AVOID_RADIUS: f32 = 50.0;
/// Clip brushes the avoidance check traces against (AI clip).
const AVOID_CLIP: u32 = 0x0002_0000;

/// A player in the crowd, as the bots avoid it.
struct Avoider {
    id: u32,
    pos: [f32; 3],
    motion_vec: [f32; 2],
    radius: f32,
    /// A player (not a bot): it does not dodge, so a bot takes all of it.
    firm: bool,
    team: String,
}

/// Everyone in play this tick, for the bots' avoidance; without teams
/// every team is left blank (BO2's free-for-all: all on team 0, Avoid_GetNeighbors
/// avoid.c 460-470).
fn crowd(world: &mut World) -> Vec<Avoider> {
    let members: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for cid in members {
        if !alive(world, cid) {
            continue;
        }
        let client = ClientId(cid);
        let Some((pos, motion_vec)) = frame(world)
            .avatar(client)
            .map(|pstate| (pstate.origin, [pstate.velocity[0], pstate.velocity[1]]))
        else {
            continue;
        };
        let bot = world.resource::<Zm>().bots.contains_key(&cid);
        let radius = if bot { BOT_AVOID_RADIUS } else { PLAYER_AVOID_RADIUS };
        out.push(Avoider { id: cid, pos, motion_vec, radius, firm: !bot, team: String::new() });
    }
    let teams = super::with_vm(world, |vm, world| {
        teambased(vm).then(|| out.iter().map(|a| team_of(vm, world, a.id)).collect::<Vec<_>>())
    })
    .flatten();
    let Some(teams) = teams else {
        return out;
    };
    for (a, t) in out.iter_mut().zip(teams) {
        a.team = t;
    }
    out
}

fn det2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// The velocity nearest `pref` (no faster than `top_velocity`) that keeps
/// `me` clear of `others` over the horizon: each neighbour gives a
/// half-plane of allowed velocities (half the dodge each between bots, all
/// of it around a player), met one at a time; when one cannot be met it
/// keeps what it had.
fn avoid_velocity(me: &Avoider, pref: [f32; 2], top_velocity: f32, others: &[&Avoider]) -> [f32; 2] {
    let inv_t = 1.0 / AVOID_HORIZON;
    // Each half-plane as (a point on its edge, the edge's direction); the
    // allowed side is to the left.
    let mut lines: Vec<([f32; 2], [f32; 2])> = Vec::with_capacity(others.len());
    for o in others {
        let rel_pos = [o.pos[0] - me.pos[0], o.pos[1] - me.pos[1]];
        let rel_vel = [me.motion_vec[0] - o.motion_vec[0], me.motion_vec[1] - o.motion_vec[1]];
        let squared_gap = dot2(rel_pos, rel_pos);
        let r = me.radius + o.radius;
        let share = if o.firm { 1.0 } else { 0.5 };
        let (dir, u);
        // Off the edge nearest the velocity: the cut-off circle at the
        // horizon or either side cone leg (AvoidUpdateOne, _misc.c
        // 32588-32623); already touching, only closing in is ruled out
        // (32625-32636).
        let toward = |w: [f32; 2], k: f32| {
            let len = dot2(w, w).sqrt().max(1e-6);
            let unit = [w[0] / len, w[1] / len];
            ([unit[1], -unit[0]], [(k - len) * unit[0], (k - len) * unit[1]])
        };
        if squared_gap >= r * r {
            let w = [rel_vel[0] - inv_t * rel_pos[0], rel_vel[1] - inv_t * rel_pos[1]];
            let along = dot2(w, rel_pos);
            if along < 0.0 && along * along > r * r * dot2(w, w) {
                (dir, u) = toward(w, r * inv_t);
            } else {
                let leg = (squared_gap - r * r).sqrt();
                dir = if det2(rel_pos, w) > 0.0 {
                    [(rel_pos[0] * leg - rel_pos[1] * r) / squared_gap, (rel_pos[0] * r + rel_pos[1] * leg) / squared_gap]
                } else {
                    [-(rel_pos[0] * leg + rel_pos[1] * r) / squared_gap, -(-rel_pos[0] * r + rel_pos[1] * leg) / squared_gap]
                };
                let d = dot2(rel_vel, dir);
                u = [d * dir[0] - rel_vel[0], d * dir[1] - rel_vel[1]];
            }
        } else {
            // BO2's edge runs square to the gap through its own point
            // (_misc.c 32626-32635, taken as the binary computes it).
            let gap = squared_gap.sqrt();
            let gap = if gap > 0.0 { gap } else { 1.0 };
            dir = [-rel_pos[1] / gap, rel_pos[0] / gap];
            u = [o.motion_vec[0] - 3.0 * me.motion_vec[0], o.motion_vec[1] - 3.0 * me.motion_vec[1]];
        }
        lines.push(([me.motion_vec[0] + share * u[0], me.motion_vec[1] + share * u[1]], dir));
    }
    let len = dot2(pref, pref).sqrt();
    let mut out = if len > top_velocity { [pref[0] / len * top_velocity, pref[1] / len * top_velocity] } else { pref };
    for i in 0..lines.len() {
        let (p, d) = lines[i];
        if det2(d, [p[0] - out[0], p[1] - out[1]]) > 0.0 {
            let Some(fixed) = on_line(&lines[..=i], top_velocity, pref) else {
                break;
            };
            out = fixed;
        }
    }
    out
}

/// The point on the last line nearest `opt` inside the speed circle and the
/// earlier half-planes, if any.
fn on_line(text_lines: &[([f32; 2], [f32; 2])], reach: f32, opt: [f32; 2]) -> Option<[f32; 2]> {
    let (&(p, d), rest) = text_lines.split_last()?;
    let dp = dot2(p, d);
    let disc = dp * dp + reach * reach - dot2(p, p);
    if matches!(disc.partial_cmp(&0.0), Some(std::cmp::Ordering::Less)) {
        return None;
    }
    let s = disc.sqrt();
    let (mut lo, mut hi) = (-dp - s, -dp + s);
    for &(q, e) in rest {
        let den = det2(d, e);
        let num = det2(e, [p[0] - q[0], p[1] - q[1]]);
        if den.abs() <= 1e-3 {
            if num < 0.0 {
                return None;
            }
            continue;
        }
        let t = num / den;
        if den >= 0.0 {
            hi = hi.min(t);
        } else {
            lo = lo.max(t);
        }
        if lo > hi {
            return None;
        }
    }
    let t = dot2(d, [opt[0] - p[0], opt[1] - p[1]]).max(lo).min(hi);
    Some([p[0] + t * d[0], p[1] + t * d[1]])
}

/// BO2's check on a dodge: a thin line at knee height toward the bent
/// lookahead, failing inside a clip brush or on one within 15 units of the
/// bot (measured where the line halves).
fn avoid_clear(game: &mut World, spot: [f32; 3], goal: [f32; 3]) -> bool {
    let a = [spot[0], spot[1], spot[2] + 18.0];
    let b = [goal[0], goal[1], goal[2] + 18.0];
    let t = frame(game).trace_static_world(a, b, [0.0; 3], [0.0; 3], AVOID_CLIP);
    if t.starts_inside || t.fully_embedded {
        return false;
    }
    if t.fraction < 1.0 {
        let h = t.fraction * 0.5;
        let m = [a[0] + (b[0] - a[0]) * h, a[1] + (b[1] - a[1]) * h];
        let d = [spot[0] - m[0], spot[1] - m[1]];
        return dot2(d, d) >= 225.0;
    }
    true
}

/// The eye of a player: his origin plus his view height.
fn eye(world: &mut World, n: u32) -> Option<[f32; 3]> {
    frame(world)
        .avatar(ClientId(n))
        .map(|pstate| [pstate.origin[0], pstate.origin[1], pstate.origin[2] + pstate.eye_height_now])
}

/// bo2mp mod menu "Bots Don't Shoot": a bot whose `ignoreall` is set never
/// fires, knifes or throws (BO2's bot scripts keep shooting a target they
/// already have without asking `getthreats` again).
fn holds_fire(vm: &mut Vm<World>, world: &World, n: u32) -> bool {
    let f = vm.intern("ignoreall");
    world.resource::<Zm>().players.get(&n).is_some_and(|p| gsc_t6::truthy(&vm.raw_field(p.obj, f)))
}

/// A player in play: on a team, playing, with health.
fn alive(world: &mut World, n: u32) -> bool {
    let playing = world
        .resource::<Zm>()
        .players
        .get(&n)
        .is_some_and(|p| p.sessionstate == "playing");
    playing && frame(world).avatar(ClientId(n)).is_some_and(|ps| ps.health > 0)
}

fn team_of(vm: &mut Vm<World>, game: &World, nn: u32) -> String {
    let Some(p) = game.resource::<Zm>().players.get(&nn) else {
        return String::new();
    };
    let f = vm.intern("team");
    let v = vm.raw_field(p.obj, f);
    vm.to_text(&v)
}

/// Is the way toward `to` blocked at step height (a player's box raised by
/// the walk's step, 24 units on)?
/// BO2's bot at a jump link's begin, there the tick before too, looks for
/// glass on the whole second: a line 64 units from its eye along its view
/// (solid and glass). A standing pane it meets first tells the bot "glass"
/// with the point it met (bot_glass_think keeps it; bot_update_glass looks
/// at it and melees, which breaks the pane).
fn glass_ahead(world: &mut World, n: u32, eye: [f32; 3], view: [f32; 3]) {
    const MASK_SOLID_GLASS: u32 = 0x11;
    let (fwd, _, _) = game_math::euler_basis(view);
    let end = [0, 1, 2].map(|i| eye[i] + fwd[i] * 64.0);
    let (with, without) = {
        let f = frame(world);
        (
            f.trace_clip(eye, end, [0.0; 3], [0.0; 3], MASK_SOLID_GLASS),
            f.trace_navigation(eye, end, [0.0; 3], [0.0; 3], MASK_SOLID_GLASS),
        )
    };
    // Glass is what stops the line only when the line runs on without it.
    if with.starts_inside || with.fraction >= 1.0 || with.fraction >= without.fraction {
        return;
    }
    let at = with.finish_location(eye, end);
    if botlog() {
        diag::info!(Sim, "bo2mp bot {n} glass ahead at {:.0} {:.0} {:.0}", at[0], at[1], at[2]);
    }
    let obj = world.resource::<Zm>().players.get(&n).map(|p| p.obj);
    if let Some(obj) = obj {
        super::with_vm(world, |vm, world| vm.notify_str(world, obj, "glass", &[ScriptValue::Vec3(at)]));
    }
}

fn blocked_ahead(game: &mut World, spot: [f32; 3], goal: [f32; 3]) -> bool {
    let dd = [goal[0] - spot[0], goal[1] - spot[1]];
    let len = (dd[0] * dd[0] + dd[1] * dd[1]).sqrt();
    if len < 1.0 {
        return false;
    }
    let finish = [spot[0] + dd[0] / len * 24.0, spot[1] + dd[1] / len * 24.0, spot[2]];
    let t = step_trace(game, spot, finish);
    t.fully_embedded || t.fraction < 1.0
}

/// A player's box raised by the 18-unit step, from `from` to `to` (both at
/// the feet), as BO2's Path_PredictionTrace sweeps it: a box that starts
/// inside the map (a spawn set into a clip brush) is shrunk by 5 units
/// (lookaheadTraceFudge) across and on top, down to 10 across, and is then
/// judged by the sweep alone, so a bot can walk out of the brush.
fn step_trace(game: &mut World, from: [f32; 3], goal: [f32; 3]) -> trace_t6::Trace {
    use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
    let from_pt = [from[0], from[1], from[2] + 18.0];
    let finish = [goal[0], goal[1], goal[2] + 18.0];
    let mut r = BODY_BOX_UPPER[0];
    let mut top = BODY_BOX_UPPER[2] - 18.0;
    loop {
        let tt = frame(game).trace_clip(from_pt, finish, [-r, -r, BODY_BOX_LOWER[2]], [r, r, top], PLAYER_BODY_MASK);
        if !tt.starts_inside || r <= 10.0 {
            return tt;
        }
        r -= 5.0;
        top -= 5.0;
    }
}

/// BO2's prediction trace to the lookahead: can a player walk straight
/// from `origin` to `to` (its box raised by the 18-unit step, against the
/// map and what stands in it, as the walk itself collides)?
fn walk_clear(game: &mut World, spot: [f32; 3], goal: [f32; 3]) -> bool {
    if dist2d(spot, goal) < 1.0 {
        return true;
    }
    let t = step_trace(game, spot, goal);
    !t.fully_embedded && t.fraction >= 1.0
}

/// BO2's nearest node to a place (Path_NearestNode, as its path search
/// starts and ends): of `near_cylinder`'s nodes, the first the place stands
/// in (the node's box: 15 each way across, 48 up) or that a body-sized box
/// (-10 -10 17 to 10 10 48) reaches in a straight line; a bot upstairs no
/// longer starts from the node on the floor below. None: our closest node.
fn nearest_reachable(game: &mut World, pt: [f32; 3]) -> Option<u32> {
    use crate::shot_trace::PLAYER_BODY_MASK;
    let near = game.resource::<Zm>().nav.near_cylinder(pt);
    let reached = near.into_iter().find_map(|(i, o)| {
        let inside = (pt[0] - o[0]).abs() < 16.0 && (pt[1] - o[1]).abs() < 16.0 && pt[2] > o[2] - 1.0 && pt[2] < o[2] + 49.0;
        let mut clear = || {
            let tt = frame(game).trace_clip(pt, o, [-10.0, -10.0, 17.0], [10.0, 10.0, 48.0], PLAYER_BODY_MASK);
            !tt.starts_inside && tt.fraction >= 1.0
        };
        (inside || clear()).then_some(i)
    });
    if reached.is_some() {
        return reached;
    }
    game.resource::<Zm>().nav.nearest(pt)
}

/// BO2MP_BOTWALL=1 (test aid), read once.
fn botwall() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_BOTWALL").is_some())
}

/// BO2MP_BOTLOG=1 (test aid), read once.
fn botlog() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_BOTLOG").is_some())
}

/// BO2MP_BOTNEGLOG=1 (test aid), read once: a line every 100 ms while a bot
/// takes a link (where it is, how fast, on the ground or not, the mantle hint,
/// the jump pressed), to show why it falls short.
fn botneglog() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_BOTNEGLOG").is_some())
}

/// BO2MP_BOTFIRELOG=1 (test aid), read once: one line per bot per tick while
/// an enemy is in sight or within 400 units, to show why a bot holds fire.
fn botfirelog() -> bool {
    static SWITCH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWITCH.get_or_init(|| std::env::var_os("BO2MP_BOTFIRELOG").is_some())
}

/// What a bot's think settled on for the tick, for the fire log.
struct FireLog {
    now: i64,
    /// The view yaw the command carries (after the steering).
    yaw_cmd: f32,
    /// How far the steering still has to turn toward what it chases.
    steer_err: f32,
    arm: u32,
    wstate: i32,
    sprint_on: bool,
    /// Its think wanted to sprint (before the trigger took it off).
    sprint_wanted: bool,
    allow: bool,
    delay_until: i64,
    looking: bool,
    /// The buttons in its command.
    press: u32,
}

/// The fire log's line (BO2MP_BOTFIRELOG): the nearest enemy that is in
/// sight or within 400 units, how far off its aim is, what the trigger and
/// sprint did, the gun's state, and whether the pull before this tick let a
/// shot out (its magazine dropped since).
fn log_botfire(world: &mut World, n: u32, l: &FireLog) {
    let clip = frame(world).damage_profile_of(l.arm).and_then(|facts| {
        let key = weapon_t6::ammo_table::clip_table_key(facts.motion_clip_slot, l.arm);
        frame(world)
            .avatar(ClientId(n))
            .filter(|ps| weapon_t6::ammo_table::clip_row_present(&ps.magazine_rows, key))
            .map(|ps| weapon_t6::ammo_table::get_clip_for_hand(&ps.magazine_rows, key, 0))
    });
    let wtime = frame(world).avatar(ClientId(n)).map_or(0, |ps| ps.weapon_timer);
    let shot = {
        let mut zm = world.resource_mut::<Zm>();
        let Some(b) = zm.bots.get_mut(&n) else { return };
        let prev = b.fire_dbg.replace((l.arm, clip.unwrap_or(-1)));
        prev.is_some_and(|(w, c)| w == l.arm && c >= 0 && clip.is_some_and(|now| now < c))
    };
    let Some(from) = eye(world, n) else { return };
    let enemies = super::with_vm(world, |vm, world| enemies_of(vm, world, n, true)).unwrap_or_default();
    let mut best = Closest::new();
    for e in enemies {
        let Some(to) = eye(world, e) else { continue };
        let d = dist3(from, to);
        if !best.beaten_by(d) {
            continue;
        }
        let seen = sight(world, from, to);
        if seen || d < 400.0 {
            best.offer(d, (e, seen, to));
        }
    }
    let Some((d, (e, seen, to))) = best.into_scored() else { return };
    let LookAngles { yaw, .. } = angles_to(from, to);
    diag::info!(
        Sim,
        "bo2mp botfire t={} bot={n} enemy={e} dist={d:.0} sight={} yawerr={:.1} steer={:.1} look={} allow={} attack={} sprintbtn={} sprintwant={} sprinting={} wstate={} wtime={wtime} clip={} delay_until={} wait={} shot={}",
        l.now,
        u8::from(seen),
        angle_delta(l.yaw_cmd, yaw).abs(),
        l.steer_err,
        u8::from(l.looking),
        u8::from(l.allow),
        u8::from(l.press & buttons::ATTACK != 0),
        u8::from(l.press & buttons::SPRINT != 0),
        u8::from(l.sprint_wanted),
        u8::from(l.sprint_on),
        l.wstate,
        clip.unwrap_or(-1),
        l.delay_until,
        (l.delay_until - l.now).max(0),
        u8::from(shot),
    );
}

/// A clear line: nothing in the map in the way, static models (fences,
/// cars, crates) included, as BO2's sight trace sees them.
fn sight(game: &mut World, from: [f32; 3], goal: [f32; 3]) -> bool {
    let t = frame(game).trace_static_world_models(from, goal, crate::shot_trace::MASK_SHOT);
    t.fraction >= 1.0
}

/// bo2mp: can a bot see from `from` to `to` (BO2's `Bot_SightCheck`):
/// nothing solid between, and the line not hidden by smoke.
fn bot_sees(game: &mut World, from: [f32; 3], goal: [f32; 3]) -> bool {
    super::natives_fx::fx_visibility(game, from, goal) >= 0.2 && sight(game, from, goal)
}

/// Players on other teams (or every other player without teams), in play.
/// Is this a team game (`level.teambased`)? Free-for-all players all
/// stand on one team name and are each other's enemies.
fn teambased(vm: &mut Vm<World>) -> bool {
    let f = vm.intern("teambased");
    gsc_t6::truthy(&vm.raw_field(vm.level, f))
}

pub(crate) fn enemies_of(vm: &mut Vm<World>, game: &mut World, nn: u32, alive_only: bool) -> Vec<u32> {
    let mine = team_of(vm, game, nn);
    let teambased = teambased(vm) && mine != "free" && !mine.is_empty();
    let members: Vec<u32> = game.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for cid in members {
        if cid == nn {
            continue;
        }
        let theirs = team_of(vm, game, cid);
        if theirs == "spectator" || theirs.is_empty() {
            continue;
        }
        if teambased && theirs == mine {
            continue;
        }
        if alive_only && !alive(game, cid) {
            continue;
        }
        out.push(cid);
    }
    out
}

fn friends_of(vm: &mut Vm<World>, game: &mut World, nn: u32, alive_only: bool) -> Vec<u32> {
    if !teambased(vm) {
        return Vec::new();
    }
    let mine = team_of(vm, game, nn);
    let members: Vec<u32> = game.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for cid in members {
        if cid == nn || team_of(vm, game, cid) != mine {
            continue;
        }
        if alive_only && !alive(game, cid) {
            continue;
        }
        out.push(cid);
    }
    out
}

fn objs(world: &World, ids: &[u32]) -> Vec<ScriptValue> {
    let zm = world.resource::<Zm>();
    ids.iter()
        .filter_map(|id| zm.players.get(id).map(|p| ScriptValue::Object(p.obj)))
        .collect()
}

/// A target's place: a point, an entity, a path node (struct origin).
fn place(vm: &mut Vm<World>, game: &mut World, v: &ScriptValue) -> Option<[f32; 3]> {
    let direct = v.as_vec3().or_else(|| origin_of(vm, game, v));
    if direct.is_some() {
        return direct;
    }
    let ScriptValue::Object(o) = v else { return None };
    let f = vm.intern("origin");
    vm.raw_field(*o, f).as_vec3()
}

fn bot_mut<'w>(world: &'w mut World, n: u32) -> Option<bevy_ecs::prelude::Mut<'w, Zm>> {
    let zm = world.resource_mut::<Zm>();
    zm.bots.contains_key(&n).then_some(zm)
}

/// The free client slots: the local player is 0; bots take the next free.
fn free_client(vm: &Vm<World>, world: &mut World) -> Option<u32> {
    let max = vm
        .dvars
        .get("sv_maxclients")
        .and_then(|raw| raw.parse::<u32>().ok())
        .unwrap_or(18);
    let sorted_clients = frame(world).ordered_player_ids();
    let mut used: Vec<u32> = sorted_clients.into_iter().map(|c| c.0).collect();
    let zm = world.resource::<Zm>();
    used.extend(zm.players.keys().copied());
    used.extend(zm.test_client_requests.iter().copied());
    (1..max).find(|i| !used.contains(i))
}

pub(super) fn bind(vm: &mut Vm<World>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    macro_rules! m {
        ($name:literal, $body:expr) => {
            vm.bind($name, true, $body);
        };
    }
    // A bot joins: its player entity at once (the script marks it lhs bot and
    // starts its spawn thread before it connects), its client next frames.
    f!("addtestclient", |vm, sim, _, _| {
        // bo2mp mod menu "Kick All Bots": no more bots join this match
        // (BO2's bot_local_think would refill them in 3 s).
        let held = vm.intern("menu_nobots");
        if gsc_t6::truthy(&vm.raw_field(vm.level, held)) {
            return Ok(ScriptValue::Undefined);
        }
        let Some(n) = free_client(vm, sim) else {
            return Ok(ScriptValue::Undefined);
        };
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let pers = vm.intern("pers");
        vm.set_raw_field(obj, pers, ScriptValue::array(Array::new()));
        let mut zm = sim.resource_mut::<Zm>();
        let mut player = Survivor::new(obj);
        player.test_client = true;
        player.pending_since = Some(zm.clock_millis);
        zm.players.insert(n, player);
        zm.bots.insert(n, Bot::default());
        zm.test_client_requests.push(n);
        diag::info!(Sim, "bo2mp bot: test client {n} joins");
        Ok(ScriptValue::Object(obj))
    });
    m!("istestclient", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(ScriptValue::bool(
            sim
                .resource::<Zm>()
                .players
                .get(&n)
                .is_some_and(|p| p.test_client),
        ))
    });
    m!("botleavegame", |vm, sim, s, _| {
        if let Some(n) = entnum(vm, s) {
            sim.resource_mut::<Zm>().bot_leaves.push(n);
        }
        Ok(ScriptValue::Undefined)
    });
    // Goals.
    m!("addgoal", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let Some(pos) = place(vm, sim, arg(lhs, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let radius = arg(lhs, 1).as_float().unwrap_or(24.0);
        let priority = arg(lhs, 2).as_int().unwrap_or(1);
        let name = text(vm, lhs, 3);
        if let Some(mut zm) = bot_mut(sim, n) {
            let b = zm.bots.get_mut(&n).unwrap();
            b.order += 1;
            let order = b.order;
            b.goals.retain(|g| g.name != name);
            b.goals.push(Goal {
                name,
                pos,
                radius,
                priority,
                order,
            });
        }
        Ok(ScriptValue::Undefined)
    });
    m!("cancelgoal", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, lhs, 0);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().goals.retain(|g| g.name != name);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("hasgoal", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, lhs, 0);
        Ok(ScriptValue::bool(sim.resource::<Zm>().bots.get(&n).is_some_and(|b| {
            b.goals.iter().any(|g| g.name == name)
        })))
    });
    m!("getgoal", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, lhs, 0);
        Ok(sim
            .resource::<Zm>()
            .bots
            .get(&n)
            .and_then(|b| b.goals.iter().find(|g| g.name == name))
            .map_or(ScriptValue::Undefined, |g| ScriptValue::Vec3(g.pos)))
    });
    // At lhs goal (named, else the one it walks to): within its radius. No
    // goal at all: where it wants to be.
    m!("atgoal", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = arg(lhs, 0);
        let name = if vm.is_defined(name) {
            Some(vm.to_text(name))
        } else {
            None
        };
        let origin = frame(sim).avatar(ClientId(n)).map(|ps| ps.origin);
        let zm = sim.resource::<Zm>();
        let Some(b) = zm.bots.get(&n) else {
            return Ok(ScriptValue::Int(0));
        };
        let goal = match &name {
            Some(name) => b.goals.iter().find(|g| &g.name == name),
            None => b.best_goal(),
        };
        let at = match (goal, origin) {
            (Some(g), Some(o)) => dist2d(o, g.pos) <= g.radius.max(16.0) && (o[2] - g.pos[2]).abs() < 72.0,
            (None, _) => name.is_none(),
            _ => false,
        };
        Ok(ScriptValue::bool(at))
    });
    // BO2's lookahead: the direction (z the rise per unit) and distance to
    // the point its route walk heads for; undefined and 0 with no route or
    // at its goal (its scripts then pick somewhere to look).
    m!("getlookaheaddir", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let la = sim.resource::<Zm>().bots.get(&n).and_then(|b| b.la_dir);
        Ok(la.map_or(ScriptValue::Undefined, |(d, _)| ScriptValue::Vec3(d)))
    });
    m!("getlookaheaddist", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let la = sim.resource::<Zm>().bots.get(&n).and_then(|b| b.la_dir);
        Ok(ScriptValue::Float(la.map_or(0.0, |(_, d)| d)))
    });
    m!("botsetfailsafenode", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let node = match arg(lhs, 0) {
            ScriptValue::Object(o) => Some(*o),
            _ => None,
        };
        let p = place(vm, sim, arg(lhs, 0));
        if botlog() {
            diag::info!(Sim, "bo2mp bot {n} failsafe node {:?}", p.map(|p| [p[0] as i32, p[1] as i32, p[2] as i32]));
        }
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().failsafe = p.map(|p| (p, node));
        }
        Ok(ScriptValue::Undefined)
    });
    // Looking and buttons.
    m!("lookat", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let p = place(vm, sim, arg(lhs, 0));
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().look = p;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("clearlookat", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().look = None;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("allowattack", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(lhs, 0)) && !holds_fire(vm, sim, n);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().allow_attack = on;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("pressads", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(lhs, 0));
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().ads = on;
        }
        Ok(ScriptValue::Undefined)
    });
    // One pull of the trigger (an argument: held that many ticks).
    m!("pressattackbutton", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let ticks = arg(lhs, 0).as_int().unwrap_or(1).clamp(1, 40) as u32;
        if holds_fire(vm, sim, n) {
            return Ok(ScriptValue::Undefined);
        }
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().attack_ticks = ticks;
        }
        Ok(ScriptValue::Undefined)
    });
    // Use held for that long (reloads when nothing is there to use).
    m!("pressusebutton", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let secs = arg(lhs, 0).as_float().unwrap_or(0.1);
        if let Some(mut zm) = bot_mut(sim, n) {
            let until = zm.clock_millis + (secs * 1000.0) as i64;
            zm.bots.get_mut(&n).unwrap().use_until = until;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("pressmelee", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if holds_fire(vm, sim, n) {
            return Ok(ScriptValue::Undefined);
        }
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().melee = true;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("pressdtpbutton", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().dive = true;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("setstance", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let stance = match text(vm, lhs, 0).as_str() {
            "crouch" => 1,
            "prone" => 2,
            _ => 0,
        };
        // BO2: lhs bot only changes the stance it picks; anyone else's real
        // stance changes at once (this bind replaces the player one in MP).
        if bot_mut(sim, n).is_none() {
            let st = text(vm, lhs, 0);
            if let Some(ps) = frame(sim).occupant_mut(ClientId(n)) {
                super::natives_player::set_stance(ps, &st);
            }
            return Ok(ScriptValue::Undefined);
        }
        let at = frame(sim).avatar(ClientId(n)).map(|ps| ps.origin);
        if let Some(mut zm) = bot_mut(sim, n) {
            let b = zm.bots.get_mut(&n).unwrap();
            if botlog() && b.stance != stance {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} stance {} -> {stance} at {:?} lookahead {:?} on link {}",
                    b.stance,
                    at.map(|o| o.map(|c| c as i32)),
                    b.la_dir.map(|(d, l)| ([d[0], d[1], d[2]].map(|c| (c * 100.0) as i32), l as i32)),
                    b.neg.is_some(),
                );
            }
            b.stance = stance;
        }
        Ok(ScriptValue::Undefined)
    });
    m!("allowsprint", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(lhs, 0));
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().no_sprint = !on;
        }
        // bo2mp: the same switch every player has (the server drops sprint).
        let cid = crate::sim_parts::ClientId(n);
        let mut f = super::frame(sim);
        if f.client_meta(cid).is_some() {
            f.client_meta_mut(cid).controls.sprint_disabled = !on;
        }
        Ok(ScriptValue::Undefined)
    });
    // A grenade at lhs point (BO2's scripts give the gun and the point,
    // no look-at): it turns there, throws once it faces it, and keeps
    // facing it until the grenade has left its hand (`drive`). A tactical
    // goes on its own button. True when it takes the throw up (BO2's
    // PlayerCmd_BotThrowGrenade; the scripts branch on it).
    m!("throwgrenade", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if holds_fire(vm, sim, n) {
            return Ok(ScriptValue::Int(0));
        }
        let p = place(vm, sim, arg(lhs, 1));
        let Some(w) = super::weapon(sim, &text(vm, lhs, 0)).ok().filter(|w| *w != 0) else {
            return Ok(ScriptValue::Int(0));
        };
        let (tactical, ammo) = {
            let f = frame(sim);
            (
                f.gun_facts().get(w).core.offhand_slot == weapon_t6::facts::offhand_slot::TACTICAL_GRENADE,
                crate::client_script::ammo_clip(&f, ClientId(n), w)
                    + crate::client_script::rounds_held(&f, ClientId(n), w),
            )
        };
        let Some(mut zm) = bot_mut(sim, n) else {
            return Ok(ScriptValue::Int(0));
        };
        let now = zm.clock_millis;
        let b = zm.bots.get_mut(&n).unwrap();
        if !takes_throw(ammo, b.throw.is_some(), b.throw_wait, now) {
            return Ok(ScriptValue::Int(0));
        }
        b.throw_wait = now + THROW_WAIT_MS;
        // It turns from where the view is now (lhs script may just have set
        // it with setplayerangles).
        b.aim_avg = None;
        let button = if tactical { buttons::OBSCURANT } else { buttons::SAMPLE_FRAG_ASSET };
        b.throw = Some(Throw { at: p, button, asked: now, pressed: 0, underway: false });
        b.tick = 0;
        b.attack_ticks = 0;
        Ok(ScriptValue::Int(1))
    });
    // Seeing.
    m!("botsighttracepassed", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let Some(from) = eye(sim, n) else {
            return Ok(ScriptValue::Int(0));
        };
        let target = arg(lhs, 0);
        let to = match entnum(vm, target) {
            Some(t) if sim.resource::<Zm>().players.contains_key(&t) => eye(sim, t),
            _ => place(vm, sim, target).map(|p| [p[0], p[1], p[2] + 32.0]),
        };
        Ok(ScriptValue::bool(to.is_some_and(|to| bot_sees(sim, from, to))))
    });
    // Enemies it can see, nearest first, inside its field of view: the
    // argument is the smallest cosine between its view and the enemy (its
    // scripts' `bot.fov`: 0.0872 = 85 degrees each side on normal; -1 =
    // all round).
    m!("getthreats", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let fov = arg(lhs, 0).as_float().unwrap_or(-1.0);
        let Some(from) = eye(sim, n) else {
            return Ok(list(Vec::new()));
        };
        // bo2mp mod menu: lhs bot told to `ignoreall` sees nobody, and lhs
        // player set `ignoreme` (Invisible) is never seen.
        let script_flag = |vm: &mut Vm<World>, sim: &World, en: u32, name: &str| {
            let f = vm.intern(name);
            sim.resource::<Zm>().players.get(&en).is_some_and(|p| gsc_t6::truthy(&vm.raw_field(p.obj, f)))
        };
        if script_flag(vm, sim, n, "ignoreall") {
            return Ok(list(Vec::new()));
        }
        let view = frame(sim).avatar(ClientId(n)).map_or([0.0; 3], |ps| ps.look_angles);
        let mut seen: Vec<(f32, u32)> = Vec::new();
        for en in enemies_of(vm, sim, n, true) {
            if script_flag(vm, sim, en, "ignoreme") {
                continue;
            }
            let Some(to) = eye(sim, en) else { continue };
            if fov > -1.0 {
                let (yw, pit) = (view[1].to_radians(), view[0].to_radians());
                let ahead = [pit.cos() * yw.cos(), pit.cos() * yw.sin(), -pit.sin()];
                let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
                let dot = (ahead[0] * d[0] + ahead[1] * d[1] + ahead[2] * d[2]) / len;
                if dot < fov {
                    continue;
                }
            }
            if bot_sees(sim, from, to) {
                seen.push((dist2d(from, to), en));
            }
        }
        // Its enemies' vehicles too (BO2's threats are everything that
        // takes damage; its scripts shoot an RC-XD and pass over what
        // needs lhs launcher).
        let mine = team_of(vm, sim, n);
        let tb = teambased(vm) && mine != "free" && !mine.is_empty();
        let me = sim.resource::<Zm>().players.get(&n).map(|p| p.obj);
        let cars: Vec<(u32, ObjRef, [f32; 3])> = sim
            .resource::<Zm>()
            .ents
            .iter()
            .where_value(|en| en.class_tag == "script_vehicle" && !en.unseen)
            .filter(|(cid, _)| !sim.resource::<Zm>().no_bot_target.contains(*cid))
            .filter_map(|(cid, en)| en.obj.map(|o| (*cid, o, en.origin)))
            .collect();
        let mut things: Vec<(f32, ScriptValue)> = Vec::new();
        for (cid, o, at) in cars {
            if !vm.alive(o) || !super::killstreaks::damage_target(sim, cid) {
                continue;
            }
            let enemy = if tb {
                let f = vm.intern("team");
                let t = vm.raw_field(o, f);
                let t = vm.to_text(&t);
                !t.is_empty() && t != mine
            } else {
                let f = vm.intern("owner");
                !matches!((vm.raw_field(o, f), me), (ScriptValue::Object(lhs), Some(b)) if lhs == b)
            };
            if !enemy {
                continue;
            }
            let to = [at[0], at[1], at[2] + centroid_height(sim, cid)];
            if fov > -1.0 {
                let (yw, pit) = (view[1].to_radians(), view[0].to_radians());
                let ahead = [pit.cos() * yw.cos(), pit.cos() * yw.sin(), -pit.sin()];
                let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
                if (ahead[0] * d[0] + ahead[1] * d[1] + ahead[2] * d[2]) / len < fov {
                    continue;
                }
            }
            if bot_sees(sim, from, to) {
                things.push((dist2d(from, to), ScriptValue::Object(o)));
            }
        }
        let zm = sim.resource::<Zm>();
        let mut all: Vec<(f32, ScriptValue)> = seen
            .iter()
            .filter_map(|(d, en)| zm.players.get(en).map(|p| (*d, ScriptValue::Object(p.obj))))
            .chain(things)
            .collect();
        all.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(list(all.into_iter().map(|(_, v)| v).collect()))
    });
    m!("getenemies", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let alive_only = gsc_t6::truthy(arg(lhs, 0));
        let ids = enemies_of(vm, sim, n, alive_only);
        Ok(list(objs(sim, &ids)))
    });
    m!("getfriendlies", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let alive_only = gsc_t6::truthy(arg(lhs, 0));
        let ids = friends_of(vm, sim, n, alive_only);
        Ok(list(objs(sim, &ids)))
    });
    // Where lhs player will be in that many frames at his speed.
    m!("predictposition", |vm, sim, _, lhs| {
        let Some(t) = entnum(vm, arg(lhs, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let frames = arg(lhs, 1).as_float().unwrap_or(1.0);
        if let Some(ps) = frame(sim).avatar(ClientId(t)) {
            return Ok(ScriptValue::Vec3(std::array::from_fn(|i| {
                ps.origin[i] + ps.velocity[i] * frames * 0.05
            })));
        }
        // A vehicle: where it is going at its speed.
        let Some(at) = sim.resource::<Zm>().ents.get(&t).map(|en| en.origin) else {
            return Ok(ScriptValue::Undefined);
        };
        let v = super::killstreaks::driven_velocity(sim, t).unwrap_or([0.0; 3]);
        Ok(ScriptValue::Vec3(std::array::from_fn(|i| at[i] + v[i] * frames * 0.05)))
    });
    // A player's eye height over his feet (where bots aim: torso = feet +
    // height / 1.6).
    m!("getplayerviewheight", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(ScriptValue::Float(
            frame(sim)
                .avatar(ClientId(n))
                .map_or(60.0, |ps| ps.eye_height_now),
        ))
    });
    // An entity's middle: lhs player's chest, else 32 over its origin.
    m!("getcentroid", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(ps) = frame(sim).avatar(ClientId(n)) {
            let o = ps.origin;
            return Ok(ScriptValue::Vec3([o[0], o[1], o[2] + ps.eye_height_now * 0.6]));
        }
        let h = centroid_height(sim, n);
        Ok(origin_of(vm, sim, s).map_or(ScriptValue::Undefined, |o| ScriptValue::Vec3([o[0], o[1], o[2] + h])))
    });
    f!("weaponisdualwield", |vm, _, _, lhs| {
        let w = text(vm, lhs, 0);
        Ok(ScriptValue::bool(w.contains("+dw") || w.ends_with("dw_mp")))
    });
    // A scoped gun (BO2: it has an overlay reticle): its bots hold
    // full sights lhs second before the shot.
    f!("isweaponscopeoverlay", |vm, sim, _, lhs| {
        let w = super::weapon(sim, &text(vm, lhs, 0)).ok();
        let facts = w.and_then(|w| frame(sim).damage_profile_of(w));
        Ok(ScriptValue::bool(facts.is_some_and(|f| f.hud_crosshair_layer != 0)))
    });
    f!("target_istarget", |_, _, _, _| Ok(ScriptValue::Int(0)));
    m!("getplayercamerapos", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(eye(sim, n).map_or(ScriptValue::Undefined, ScriptValue::Vec3))
    });
    // A bot's own classes (its scripts build five from his item table).
    m!("botsetdefaultclass", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(lhs, 0).as_int().unwrap_or(0);
        let name = text(vm, lhs, 1);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().classes.entry(class).or_default().default = Some(name);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("botclassadditem", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(lhs, 0).as_int().unwrap_or(0);
        let item = text(vm, lhs, 1);
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots.get_mut(&n).unwrap().classes.entry(class).or_default().items.push(item);
        }
        Ok(ScriptValue::Undefined)
    });
    m!("botclassaddattachment", |vm, sim, s, lhs| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(lhs, 0).as_int().unwrap_or(0);
        let (gun, attach, slot_no) = (text(vm, lhs, 1), text(vm, lhs, 2), text(vm, lhs, 3));
        if let Some(mut zm) = bot_mut(sim, n) {
            zm.bots
                .get_mut(&n)
                .unwrap()
                .classes
                .entry(class)
                .or_default()
                .attachments
                .push((gun, attach, slot_no));
        }
        Ok(ScriptValue::Undefined)
    });
    m!("botclasssetweaponoption", |_, _, _, _| Ok(ScriptValue::Undefined));
    // The player the bots count teams around: the local player (client 0).
    f!("gethostplayerforbots", |_, sim, _, _| Ok(sim
        .resource::<Zm>()
        .players
        .get(&0)
        .map_or(ScriptValue::Undefined, |p| ScriptValue::Object(p.obj))));
    m!("ishostforbots", |vm, _, s, _| Ok(ScriptValue::bool(entnum(vm, s) == Some(0))));
    // The map's path nodes as the bot scripts ask about them.
    fn node_obj(sim: &World, i: u32) -> ScriptValue {
        sim
            .resource::<Zm>()
            .node_objs
            .get(usize::try_from(i).unwrap_or(usize::MAX))
            .map_or(ScriptValue::Undefined, |o| ScriptValue::Object(*o))
    }
    fn node_index(vm: &mut Vm<World>, sim: &World, v: &ScriptValue) -> Option<u32> {
        let ScriptValue::Object(o) = v else { return None };
        sim
            .resource::<Zm>()
            .node_of
            .get(o)
            .copied()
            .or_else(|| {
                let f = vm.intern("origin");
                let p = vm.raw_field(*o, f).as_vec3()?;
                sim.resource::<Zm>().nav.nearest(p)
            })
    }
    fn node_eye(sim: &World, i: u32) -> [f32; 3] {
        let o = sim.resource::<Zm>().nav.graph[i as usize].origin;
        [o[0], o[1], o[2] + 32.0]
    }
    f!("getnearestnode", |vm, sim, _, lhs| {
        let Some(p) = place(vm, sim, arg(lhs, 0)) else {
            return Ok(ScriptValue::Undefined);
        };
        let i = sim.resource::<Zm>().nav.nearest(p);
        Ok(i.map_or(ScriptValue::Undefined, |i| node_obj(sim, i)))
    });
    // Two nodes see each other: BO2 reads the map compiler's sight table
    // (`Path_NodesVisible`); lhs node past it (or lhs map without one) is traced.
    fn nodes_visible(sim: &mut World, i: u32, j: u32) -> bool {
        if let Some(seen) = sim.resource::<Zm>().nav.baked.visible(i, j) {
            return seen;
        }
        let (x, y) = (node_eye(sim, i), node_eye(sim, j));
        sight(sim, x, y)
    }
    // BO2's `Scr_GetVisibleNode`: the route from `from` to `to`, and on it,
    // from `from` on, the first node that sees the route's last; none
    // without lhs route. No distance limit (the bot's turret spot wants one
    // 512 or more from it, toward the enemy).
    f!("getvisiblenode", |vm, sim, _, lhs| {
        let (Some(from), Some(to)) = (place(vm, sim, arg(lhs, 0)), place(vm, sim, arg(lhs, 1)))
        else {
            return Ok(ScriptValue::Undefined);
        };
        let route = match (nearest_reachable(sim, from), nearest_reachable(sim, to)) {
            (Some(x), Some(y)) => sim.resource::<Zm>().nav.path(x, y),
            _ => None,
        };
        let Some(&last) = route.as_ref().and_then(|r| r.last()) else {
            return Ok(ScriptValue::Undefined);
        };
        let route = route.unwrap_or_default();
        Ok(route
            .into_iter()
            .find(|&i| i == last || nodes_visible(sim, i, last))
            .map_or(ScriptValue::Undefined, |i| node_obj(sim, i)))
    });
    // The nodes lhs node sees: its linked neighbours that it can see.
    f!("getvisiblenodes", |vm, sim, _, lhs| {
        let Some(i) = node_index(vm, sim, arg(lhs, 0)) else {
            return Ok(list(Vec::new()));
        };
        let links: Vec<u32> = sim.resource::<Zm>().nav.graph[i as usize]
            .links
            .iter()
            .map(|l| u32::from(l.0))
            .collect();
        let from = node_eye(sim, i);
        let mut out = Vec::new();
        for j in links {
            if sight(sim, from, node_eye(sim, j)) {
                out.push(node_obj(sim, j));
            }
        }
        Ok(list(out))
    });
    f!("nodesvisible", |vm, sim, _, lhs| {
        let (Some(i), Some(j)) = (node_index(vm, sim, arg(lhs, 0)), node_index(vm, sim, arg(lhs, 1)))
        else {
            return Ok(ScriptValue::Int(0));
        };
        Ok(ScriptValue::bool(nodes_visible(sim, i, j)))
    });
    // BO2's `Scr_NodesCanPath`: lhs straight walk joins them (the map
    // compiler's smooth table) or the first links to the second; not lhs
    // search for lhs route.
    f!("nodescanpath", |vm, sim, _, lhs| {
        let (Some(i), Some(j)) = (node_index(vm, sim, arg(lhs, 0)), node_index(vm, sim, arg(lhs, 1)))
        else {
            return Ok(ScriptValue::Int(0));
        };
        let nav = &sim.resource::<Zm>().nav;
        let linked = nav.graph[i as usize].links.iter().any(|l| u32::from(l.0) == j);
        Ok(ScriptValue::bool(nav.baked.smooth(i, j) || linked))
    });
    // BO2's `Scr_FindPath`: whether lhs route joins two points, true or
    // false (the route itself is not handed back; every BO2 script only
    // asks yes or no).
    f!("findpath", |vm, sim, _, lhs| {
        let (Some(from), Some(to)) = (place(vm, sim, arg(lhs, 0)), place(vm, sim, arg(lhs, 1)))
        else {
            return Ok(ScriptValue::Int(0));
        };
        let route = match (nearest_reachable(sim, from), nearest_reachable(sim, to)) {
            (Some(x), Some(y)) => sim.resource::<Zm>().nav.path(x, y),
            _ => None,
        };
        Ok(ScriptValue::bool(route.is_some()))
    });
    f!("canclaimnode", |_, _, _, _| Ok(ScriptValue::Int(1)));
    // BO2's danger marks: lhs count per team on lhs node (0..15), raised and
    // lowered by `setdangerous(team, on)`, dangerous while above 0 (bots
    // keep clear of lhs killstreak's landing spot).
    m!("setdangerous", |vm, sim, s, lhs| {
        let (Some(i), Some(t)) = (node_index(vm, sim, s), danger_slot(&text(vm, lhs, 0))) else {
            return Ok(ScriptValue::Undefined);
        };
        let on = arg(lhs, 1).as_int().unwrap_or(0) != 0;
        let mut zm = sim.resource_mut::<Zm>();
        let c = &mut zm.node_danger.entry(i).or_default()[t];
        *c = if on { (*c + 1).min(15) } else { c.saturating_sub(1) };
        Ok(ScriptValue::Undefined)
    });
    m!("isdangerous", |vm, sim, s, lhs| {
        let (Some(i), Some(t)) = (node_index(vm, sim, s), danger_slot(&text(vm, lhs, 0))) else {
            return Ok(ScriptValue::Int(0));
        };
        Ok(ScriptValue::bool(sim.resource::<Zm>().node_danger.get(&i).is_some_and(|c| c[t] > 0)))
    });
    f!("claimnode", |_, _, _, _| Ok(ScriptValue::Undefined));
    f!("releaseclaimednode", |_, _, _, _| Ok(ScriptValue::Undefined));
    // Bots have every item (their scripts ask before building lhs class).
    m!("isitemlocked", |_, _, _, _| Ok(ScriptValue::Int(0)));
    // What lhs player is doing, from his state.
    m!("isreloading", |vm, sim, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(ScriptValue::bool(frame(sim).avatar(ClientId(n)).is_some_and(|ps| {
            reloading(ps.weapon_state_t6)
        })))
    });
    // As BO2 reads them: switching = the gun raising or dropping,
    // throwing = an offhand (grenade) state, mantling and on lhs ladder =
    // the move flags.
    m!("isswitchingweapons", |vm, sim, s, _| {
        Ok(player_is(vm, sim, s, |ps| {
            // TODO(fact): the swim in and out count, as our old MW2 mirror read them.
            let s = ps.weapon_state_t6;
            weapon_t6::state::is_raising_or_swim_out(s) || weapon_t6::state::is_dropping_or_swim_in(s)
        }))
    });
    m!("isthrowinggrenade", |vm, sim, s, _| {
        Ok(player_is(vm, sim, s, |ps| {
            weapon_t6::state::is_secondary_gear(ps.weapon_state_t6)
                && ps.weapon_state_t6 != weapon_t6::state::OFFHAND_DETONATE
        }))
    });
    m!("ismantling", |vm, sim, s, _| {
        Ok(player_is(vm, sim, s, |ps| ps.movement_state_bits & playerstate_t6::game::movement_state_bits::MANTLE != 0))
    });
    m!("isonladder", |vm, sim, s, _| {
        Ok(player_is(vm, sim, s, |ps| ps.movement_state_bits & playerstate_t6::game::movement_state_bits::LADDER != 0))
    });
    for name in ["isfiring", "isremotecontrolling", "iscarryingturret"] {
        vm.bind(name, true, |_, _, _, _| Ok(ScriptValue::Int(0)));
    }
}

/// Dead, its steering starts over: BO2's G_BotThink zeroes the whole bot_t
/// each think while its health is under 1 (the failsafe node with it).
fn clear_for_new_life(rhs: &mut Bot) {
    rhs.throw = None;
    // A new life picks from what it spawns holding.
    rhs.select = 0;
    rhs.select_held = 0;
    // Its route goes too: a new life plans from where it spawns
    // (one kept from the last life is trimmed as walked past and
    // aims it at the far goal through walls).
    rhs.path.clear();
    rhs.negs.clear();
    rhs.neg = None;
    rhs.neg_began = false;
    rhs.path_to = None;
    rhs.replan_from = None;
    rhs.replan_at = 0;
    rhs.prev = None;
    rhs.la_amount = 0.0;
    rhs.la_pos = None;
    rhs.la_dir = None;
    rhs.la_at = None;
    rhs.la_clear = false;
    // A failsafe kept from the last life walks it at that life's node with
    // the view held (Frostbite 10-30: 20 s into a wall after a respawn).
    rhs.failsafe = None;
    if rhs.aim_avg.is_some() {
        rhs.aim_avg = None;
        rhs.turn_speed = [0.0; 2];
        rhs.sprint_since = None;
        rhs.fire_delay_until = 0;
    }
}

/// Each tick: the bots' commands (route, view, buttons) replace their
/// clients' own; a bot whose client never came is dropped.
pub(super) fn drive(sim: &mut World, now: i64) {
    let ids: Vec<u32> = sim.resource::<Zm>().bots.keys().copied().collect();
    if ids.first().is_none() {
        return;
    }
    log_negotiations(sim);
    let dv = BotDvars::read(sim);
    let crowd = crowd(sim);
    let held = super::console_bots::frozen(sim);
    for n in ids {
        if !alive(sim, n) {
            // Dead, its steering starts over (BO2 clears its bot state).
            if let Some(rhs) = sim.resource_mut::<Zm>().bots.get_mut(&n) {
                clear_for_new_life(rhs);
            }
            continue;
        }
        let Some((spot, view, height, ladder, mantling, low_climb, sprinting, ads_frac, wstate, standing, weapon, grounded_now, ledge)) = frame(sim)
            .avatar(ClientId(n))
            .map(|pstate| {
                use playerstate_t6::game::movement_state_bits;
                let (eye_angles, eye_h) = (pstate.look_angles, pstate.eye_height_now);
                (
                    pstate.origin,
                    eye_angles,
                    eye_h,
                    pstate.movement_state_bits & movement_state_bits::LADDER != 0,
                    pstate.movement_state_bits & movement_state_bits::MANTLE != 0,
                    // A climb with no room to stand at the ledge (BO2's mantle flag 4).
                    pstate.movement_state_bits & movement_state_bits::MANTLE != 0 && pstate.mantle_state.flags & movement_t6::MANTLE_FORCE_CROUCH != 0,
                    pstate.movement_state_bits & movement_state_bits::SPRINTING != 0,
                    pstate.weapon_raise_fraction.clamp(0.0, 1.0),
                    pstate.weapon_state_t6,
                    pstate.movement_state_bits & (movement_state_bits::DUCK_BUTTON_BIT | movement_state_bits::LYING_FLAT) == 0,
                    pstate.weapon,
                    pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT,
                    // The mantle hint: a ledge in reach (BO2's mantle flag 0x10).
                    pstate.mantle_state.flags & playerstate_t6::game::ledge_climb_bits::ACTIVE != 0,
                )
            })
        else {
            continue;
        };
        let select = sim.resource_mut::<Zm>().bots.get_mut(&n).map_or(0, |rhs| {
            follow_select(&mut rhs.select, &mut rhs.select_held, weapon);
            rhs.select
        });
        // Full-automatic weapons (miniguns too) hold the trigger down.
        let full_auto = frame(sim).damage_profile_of(weapon).is_none_or(|f| f.trigger_mode == 0);
        // Taking a negotiation link (BO2's Bot_AtNegotiationEnd): once it
        // has been at the begin, done once its mantle finishes, or (a ladder
        // or a drop) once it leaves the ground, or (any other) once it stands
        // within 15 units of the link's finish; then it plans on at once,
        // from the route point after the link's end (Bot_GetPathToGoal's
        // pts[start - 1]), so a bot that grabbed a ladder at its foot climbs
        // on toward the top. BO2 gives no link up; ours drops one only
        // when its route is gone.
        {
            let mut zm = sim.resource_mut::<Zm>();
            let rhs = zm.bots.get_mut(&n).unwrap();
            let was_mantling = std::mem::replace(&mut rhs.was_mantling, mantling);
            if let Some((l, since)) = rhs.neg {
                let landed = rhs.neg_began
                    && link_done(l.flags, was_mantling && !mantling, grounded_now, mantling, dist3(spot, l.end));
                let gave_up = rhs.path.is_empty();
                if landed || gave_up {
                    let began = std::mem::take(&mut rhs.neg_began);
                    rhs.neg = None;
                    rhs.path.clear();
                    rhs.path_to = None;
                    rhs.replan_at = now;
                    rhs.replan_from = landed.then_some(l.after_node);
                    if botlog() {
                        diag::info!(
                            Sim,
                            "bo2mp bot {n} negotiation {} flags {:#x} at {:.0} {:.0} {:.0} begin {:.0} {:.0} {:.0} end {:.0} {:.0} {:.0} after {} ms{}",
                            if landed { "done" } else { "gave up" },
                            l.flags,
                            spot[0],
                            spot[1],
                            spot[2],
                            l.entry[0],
                            l.entry[1],
                            l.entry[2],
                            l.end[0],
                            l.end[1],
                            l.end[2],
                            now - since,
                            if began { "" } else { " (never at the begin)" }
                        );
                    }
                }
            }
        }
        // The route: replan when the best goal moved or the route ran out.
        // (BO2's Bot_Seek: moved = the goal is its radius or more from
        // where the route ends, in 3D.)
        let (goal, path_to, replan_at) = {
            let zm = sim.resource::<Zm>();
            let rhs = &zm.bots[&n];
            (rhs.best_goal().cloned(), rhs.path_to, rhs.replan_at)
        };
        if let Some(g) = &goal {
            let moved = path_to.is_none_or(|pt| {
                let d = [pt[0] - g.pos[0], pt[1] - g.pos[1], pt[2] - g.pos[2]];
                g.radius * g.radius <= d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            });
            let far = dist2d(spot, g.pos) > g.radius;
            let empty = sim.resource::<Zm>().bots[&n].path.is_empty();
            if (moved || (empty && far)) && now >= replan_at {
                let start = sim.resource_mut::<Zm>().bots.get_mut(&n).unwrap().replan_from.take();
                let (route, negs) = plan(sim, spot, start, g.pos);
                let mut zm = sim.resource_mut::<Zm>();
                let rhs = zm.bots.get_mut(&n).unwrap();
                rhs.path = route;
                rhs.negs = negs;
                rhs.neg = None;
                rhs.neg_began = false;
                rhs.path_to = Some(g.pos);
                rhs.replan_at = now + 500;
                rhs.prev = Some(spot);
                rhs.path_gen = rhs.path_gen.wrapping_add(1);
                rhs.path_new = true;
                rhs.la_amount = 0.0;
                rhs.la_pos = None;
                rhs.la_dir = None;
                rhs.la_at = None;
                rhs.la_clear = false;
                // BO2MP_BOTNEGLOG (test aid): each new route's first points.
                if botneglog() {
                    let first: Vec<[i32; 3]> = rhs.path.iter().take(4).map(|pt| [pt[0] as i32, pt[1] as i32, pt[2] as i32]).collect();
                    diag::info!(
                        Sim,
                        "bo2mp bot {n} route {} at {:.0} {:.0} {:.0} from node {start:?}: {} points, first {first:?}",
                        rhs.path_gen, spot[0], spot[1], spot[2], rhs.path.len()
                    );
                }
            }
        } else {
            let mut zm = sim.resource_mut::<Zm>();
            let rhs = zm.bots.get_mut(&n).unwrap();
            rhs.path.clear();
            rhs.path_to = None;
            rhs.replan_from = None;
        }
        // Something at step height between it and the next route point
        // (a ledge, a railing, a low wall), as opposed to stairs or a slope
        // the walk steps up by itself: only then does a jump (and the
        // mantle it starts) help; jumping on stairs under a ceiling holds
        // a bot in place.
        let climb_blocked: Option<([f32; 3], bool)> = {
            let peek = {
                let zm = sim.resource::<Zm>();
                let path = &zm.bots[&n].path;
                path.iter()
                    .find(|q| dist2d(spot, **q) >= 24.0)
                    .or(path.last())
                    .copied()
            };
            peek.map(|pt| (pt, dist2d(spot, pt) < 160.0 && blocked_ahead(sim, spot, pt)))
        };
        // Blocked at step height toward the next point: going up (a ledge)
        // it jumps at once; level or down (a railing it drops over, a low
        // fence) it jumps once it makes no headway.
        let ahead_blocked = climb_blocked.is_some_and(|(_, rhs)| rhs);
        let climb_blocked = climb_blocked
            .is_some_and(|(pt, rhs)| rhs && pt[2] - spot[2] > 18.0 && dist2d(spot, pt) < 96.0);
        // BO2MP_BOTLOG (test aid): is it standing inside the map or a prop,
        // or against another player?
        let (in_solid, touching) = if botlog() {
            use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
            let tt = frame(sim).trace_clip(spot, spot, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
            let ids: Vec<u32> = sim.resource::<Zm>().players.keys().copied().filter(|&id| id != n).collect();
            let touching = ids.into_iter().find(|&id| {
                frame(sim).avatar(ClientId(id)).is_some_and(|pstate| {
                    dist2d(pstate.origin, spot) < 32.0 && (pstate.origin[2] - spot[2]).abs() < 72.0
                })
            });
            (tt.starts_inside, touching)
        } else {
            (false, None)
        };
        let at_goal = goal
            .as_ref()
            .is_none_or(|g| dist2d(spot, g.pos) <= g.radius.max(16.0));
        // Past a route point once beyond the line across it (the walk cuts
        // corners toward its lookahead, so it seldom touches one), or (BO2's
        // Path_TrimCompletedPath) once it lies behind the bot along its last
        // lookahead, when that walk was clear or the amount has shrunk to
        // half: a nearest node behind it is no walk back.
        let (route, seen_key, seen) = {
            let mut zm = sim.resource_mut::<Zm>();
            let rhs = zm.bots.get_mut(&n).unwrap();
            // BO2 trims only once its last lookahead point is no longer
            // ahead of it, so never past that point's leg (a U-turn around
            // a wall stays on the route).
            let la_reached = |(d, _): &([f32; 3], f32)| {
                rhs.la_at.is_none_or(|q| d[0] * (q[0] - spot[0]) + d[1] * (q[1] - spot[1]) <= 0.0)
            };
            let trim = rhs
                .la_dir
                .filter(|_| rhs.la_clear || (rhs.la_amount > 0.0 && rhs.la_amount <= 16384.0))
                .filter(la_reached)
                .map(|(d, _)| d);
            // A link's begin stays until the link is done (BO2's trim never
            // passes the negotiation start).
            let had = rhs.path.len();
            let mut why = Vec::new();
            while rhs.path.len() > 1 {
                let pt = rhs.path[0];
                if rhs.negs.iter().any(|l| l.entry == pt) {
                    break;
                }
                let from = rhs.prev.unwrap_or(spot);
                let d = [pt[0] - from[0], pt[1] - from[1]];
                let beyond = d[0] * d[0] + d[1] * d[1] > 1.0
                    && (spot[0] - pt[0]) * d[0] + (spot[1] - pt[1]) * d[1] >= 0.0;
                let behind = trim.is_some_and(|tt| tt[0] * (pt[0] - spot[0]) + tt[1] * (pt[1] - spot[1]) <= 0.0);
                if !(beyond || behind || dist2d(spot, pt) < 24.0) {
                    break;
                }
                why.push(if beyond { 'b' } else if behind { 'h' } else { 'n' });
                rhs.prev = Some(pt);
                rhs.path.remove(0);
            }
            // BO2MP_BOTNEGLOG (test aid): a route trimmed down to its goal.
            if had > 1 && rhs.path.len() == 1 && botneglog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} route down to the goal at {:.0} {:.0} {:.0}: dropped {} ({}) la {:?} clear {} amount {:.0}",
                    spot[0], spot[1], spot[2], had - 1, why.iter().collect::<String>(),
                    rhs.la_dir.map(|(d, l)| [d[0], d[1], l]), rhs.la_clear, rhs.la_amount
                );
            }
            // BO2MP_BOTNEGLOG (test aid): two or more points trimmed at once.
            if why.len() >= 2 && botneglog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} trimmed {} ({}) at {:.0} {:.0} {:.0} route {} next {:?}",
                    why.len(), why.iter().collect::<String>(), spot[0], spot[1], spot[2], rhs.path_gen,
                    rhs.path.first().map(|pt| [pt[0] as i32, pt[1] as i32, pt[2] as i32])
                );
            }
            let near = rhs.path.get(1).is_some_and(|pt| dist2d(spot, *pt) < 192.0);
            let key = (rhs.path_gen, rhs.path.len(), near);
            let seen = rhs.seen.filter(|s| (s.0, s.1, s.2) == key).map(|s| s.3);
            ((!at_goal && !rhs.path.is_empty()).then(|| rhs.path.clone()), key, seen)
        };
        // The link its route crosses next, when it heads for the link's
        // begin now (BO2: the lookahead's next node is the begin), and the
        // one it is taking: the walk goes no farther than the begin until
        // the link is done. It is at the begin (BO2's Bot_AtNegotiationBegin)
        // while no farther from the finish than the begin is (half a unit
        // spare); it starts taking the link there.
        let (neg_ahead, link) = {
            let zm = sim.resource::<Zm>();
            let rhs = &zm.bots[&n];
            match (&route, rhs.neg) {
                (Some(r), None) => {
                    let l = rhs.negs.iter().find(|l| l.entry == r[0]).copied();
                    (l, l)
                }
                (Some(r), Some((l, _))) if l.entry == r[0] => (None, Some(l)),
                (_, _) => (None, None),
            }
        };
        let bo2_at_begin = link.is_some_and(|l| dist3(spot, l.end) < dist3(l.entry, l.end) + 0.5);
        let at_begin = bo2_at_begin;
        let route = match (route, link) {
            (Some(r), Some(_)) => Some(r[..2.min(r.len())].to_vec()),
            (r, _) => r,
        };
        let far_seen = match (&route, seen) {
            (Some(_), Some(pt)) => Some(pt),
            (Some(r), None) => {
                let pt = farthest_seen(sim, spot, r);
                if let Some(pt) = pt {
                    let mut zm = sim.resource_mut::<Zm>();
                    zm.bots.get_mut(&n).unwrap().seen = Some((seen_key.0, seen_key.1, seen_key.2, pt));
                }
                pt
            }
            _ => None,
        };
        // BO2's Path_UpdateLookaheadAmount: the walk to the lookahead is
        // traced each think; clear, it heads there and the amount grows
        // (x1.18 + 6.4, up to 65536); blocked, the amount shrinks (x0.8),
        // which pulls the point in toward the route's next bend. Its bots
        // drop the kept point every think (Bot_UpdateLookahead), so blocked
        // it heads for the new point, but for the one think right after a
        // clear walk (that walk's heading holds).
        let amount = sim.resource::<Zm>().bots[&n].la_amount;
        let la_route = route.as_deref().map(|r| if link.is_some() { &r[..1] } else { r });
        let la = match la_route.and_then(|r| lookahead(spot, r, amount)) {
            Some((pt, reach)) => {
                let clear = walk_clear(sim, spot, pt);
                let mut zm = sim.resource_mut::<Zm>();
                let rhs = zm.bots.get_mut(&n).unwrap();
                let amount = if rhs.la_amount > 0.0 { rhs.la_amount } else { 32768.0 };
                let was_clear = std::mem::replace(&mut rhs.la_clear, clear);
                if clear {
                    rhs.la_amount = (amount * 1.176_470_5 + 6.4).min(65536.0);
                    rhs.la_pos = Some(pt);
                    Some((pt, reach))
                } else {
                    rhs.la_amount = (amount * 0.8).max(0.001);
                    match rhs.la_pos.filter(|q| was_clear && dist2d(*q, spot) > 1.0) {
                        Some(q) => Some((q, dist2d(spot, q))),
                        None => Some((pt, reach)),
                    }
                }
            }
            None => None,
        };
        let next = la.map(|(pt, _)| pt);
        {
            let mut zm = sim.resource_mut::<Zm>();
            zm.bots.get_mut(&n).unwrap().la_at = la.map(|(pt, _)| pt);
            zm.bots.get_mut(&n).unwrap().la_dir = la.and_then(|(pt, _)| {
                let d = [pt[0] - spot[0], pt[1] - spot[1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                (len > 1e-3).then(|| ([d[0] / len, d[1] / len, (pt[2] - spot[2]) / len], len))
            });
        }
        // BO2MP_BOTWALL=1 (test aid): every 250 ms, does a level line 32
        // units out from its eye along its view hit the map?
        if botwall() && now % 250 == 0 {
            let eye = [spot[0], spot[1], spot[2] + height];
            let yaw = view[1].to_radians();
            let end = [eye[0] + 32.0 * yaw.cos(), eye[1] + 32.0 * yaw.sin(), eye[2]];
            let wall = frame(sim).trace_static_world(eye, end, [0.0; 3], [0.0; 3], 0x0001_0001).fraction < 1.0;
            let (look, path, fs, gain) = {
                let zm = sim.resource::<Zm>();
                let b = &zm.bots[&n];
                (b.look.is_some(), b.path.len(), b.failsafe.is_some(), b.last_gain.map(|(at, t)| (dist2d(at, spot) as i32, now - t)))
            };
            diag::info!(Sim, "bo2mp botwall {n} {} at {:.0} {:.0} {:.0} yaw {:.0} la {} look {look} path {path} failsafe {fs} gain {gain:?}", u8::from(wall), spot[0], spot[1], spot[2], view[1], la.is_some());
        }
        // BO2MP_BOTLOG (test aid): a crouched bot's stand-up trace, as its
        // bot_update_crouch runs it, once a second.
        if botlog() && now % 1000 == 0 {
            let (stance, la_dir) = {
                let zm = sim.resource::<Zm>();
                (zm.bots[&n].stance, zm.bots[&n].la_dir)
            };
            if stance == 1 {
                let s = [spot[0], spot[1], spot[2] + 70.0];
                let (frac, e) = match la_dir {
                    Some((d, l)) => {
                        let e = [s[0] + d[0] * l, s[1] + d[1] * l, s[2] + d[2] * l];
                        (frame(sim).trace_static_world(s, e, [0.0; 3], [0.0; 3], 0x0001_0001).fraction, Some(e))
                    }
                    None => (-1.0, None),
                };
                diag::info!(Sim, "bo2mp bot {n} crouched at {:.0} {:.0} {:.0} stand trace to {:?} frac {frac:.2}", spot[0], spot[1], spot[2], e.map(|e| e.map(|c| c as i32)));
            }
        }
        // Avoidance (Bot_UpdateAvoidance, bot.c 3092-3158; skipped on
        // ladders and ledges, bot.c 3511-3513): the lookahead is bent around
        // teammates close by, keeping its distance, and a bend into a clip
        // brush right in front of the bot holds it still for the tick
        // (Bot_AvoidTracePassed, bot.c 3171-3227). The bot walks the bend
        // only on a team and not on the way to (or over) a jump link
        // (Bot_ShouldAvoid, bot.c 2328-2350); the hold is checked either way.
        let mut walk_to = next;
        let mut hold = false;
        if let Some((lp, _)) = la
            && !ladder
            && !mantling
            && let Some(me) = crowd.iter().find(|a| a.id == n)
        {
            let dodge = !me.team.is_empty() && me.team != "free" && !link.is_some_and(|l| l.flags & NEG_JUMP != 0);
            let d = [lp[0] - spot[0], lp[1] - spot[1]];
            let dist = dot2(d, d).sqrt();
            let speed = me.motion_vec[0].hypot(me.motion_vec[1]);
            let range = (speed + BOT_AVOID_SPEED) * 0.5 * AVOID_HORIZON;
            let others: Vec<&Avoider> = crowd
                .iter()
                .filter(|a| {
                    a.id != n
                        && a.team == me.team
                        && dist2d(a.pos, spot) < range
                        && (a.pos[2] - spot[2]).abs() < 90.0
                })
                .take(32)
                .collect();
            if dist > 1.0 && !others.is_empty() {
                let pref = [d[0] / dist * speed, d[1] / dist * speed];
                let v = avoid_velocity(me, pref, BOT_AVOID_SPEED, &others);
                let vl = v[0].hypot(v[1]);
                if vl > 0.01 {
                    let bent = [spot[0] + v[0] / vl * dist, spot[1] + v[1] / vl * dist, lp[2]];
                    hold = dist2d(bent, lp) > 15.0 && !avoid_clear(sim, spot, bent);
                    if dodge {
                        walk_to = Some(bent);
                    }
                }
            }
        }
        let mut zm = sim.resource_mut::<Zm>();
        let rhs = zm.bots.get_mut(&n).unwrap();
        rhs.tick = rhs.tick.wrapping_add(1);
        // The view (BO2's Bot_Think / Bot_UpdateLookahead): the angles it
        // chases move toward what it looks at (an eighth of the way in
        // pitch, a quarter in yaw each tick), else toward the farthest route
        // point it can see, slower the farther its lookahead reaches; the
        // view then steers toward them. A grenade it is about to throw (or
        // throwing) comes before what it looks at.
        let eye_at = [spot[0], spot[1], spot[2] + height];
        // BO2's Bot_HasLookAt: while its lookahead heads for a jump or ladder
        // link's begin (or takes it), what a script has it look at waits, so
        // it faces the link (else, on Drone's crates, ~30 degrees off, it
        // finds no ledge and gives up).
        let climbing = link.is_some_and(|l| l.flags & (NEG_JUMP | NEG_LADDER) != 0);
        let look = rhs.throw.and_then(|tt| tt.at).or(rhs.look.filter(|_| !climbing));
        // A ladder has no look of its own (BO2's bot code sets no pitch for
        // one): it faces along its route like any walk, and the climb goes
        // up because the route does.
        let desired: Option<[f32; 2]> = if let Some(l) = look {
            let LookAngles { yaw, pitch } = angles_to(eye_at, l);
            let avg = rhs.aim_avg.unwrap_or([view[0], view[1]]);
            rhs.aim_avg = Some([avg[0] + angle_delta(avg[0], pitch) / 8.0, avg[1] + angle_delta(avg[1], yaw) / 4.0]);
            Some([pitch, yaw])
        } else if let Some((lp, reach)) = la {
            // Ours: at a link's begin, and while taking it, it faces the
            // link's finish. Its route stops at the begin until the link is
            // done, so it faced the begin it stood on (any way at all) or,
            // a step past it, back the way it came; BO2's mantle check finds
            // only a ledge within 60 degrees of the view.
            // BO2 faces a jump link's finish from the eye_at, any other from the
            // spot.
            let across = link.filter(|_| at_begin || rhs.neg.is_some());
            let tt = across.map(|l| l.end).or(far_seen.filter(|pt| dist2d(spot, *pt) >= 1.0)).unwrap_or(lp);
            let from = if across.is_some_and(|l| l.flags & NEG_JUMP != 0) { eye_at } else { spot };
            if dist2d(spot, tt) >= 1.0 {
                let LookAngles { yaw, pitch } = angles_to(from, tt);
                let h = if rhs.path_new || reach < 64.0 {
                    1.0
                } else if reach < 128.0 {
                    2.0
                } else {
                    4.0
                };
                let avg = rhs.aim_avg.unwrap_or([view[0], view[1]]);
                rhs.aim_avg = Some([avg[0] + angle_delta(avg[0], pitch) / (2.0 * h), avg[1] + angle_delta(avg[1], yaw) / h]);
                Some([pitch, yaw])
            } else {
                None
            }
        } else {
            rhs.aim_avg = None;
            None
        };
        rhs.path_new = false;
        let mut want = view;
        match (desired, rhs.aim_avg) {
            (Some(_), Some(avg)) => {
                let out = steer([view[0], view[1]], avg, &mut rhs.turn_speed, ads_frac, &dv);
                want[0] = out[0];
                want[1] = out[1];
            }
            _ => rhs.turn_speed = [0.0; 2],
        }
        let yaw_err = desired.map_or(0.0, |d| angle_delta(want[1], d[1]).abs());
        // Speed (Bot_GetMaxSpeed): full until the lookahead falls under 64
        // units, down to half; walking its route (no look-at) it also slows
        // while it faces away from where it heads, down to half at 35°.
        let speed = la.map_or(0.0, |(_, reach)| {
            let mut s = (reach / 64.0).clamp(0.5, 1.0);
            if look.is_none() && desired.is_some() {
                s = s.min((1.0 - yaw_err / 70.0).clamp(0.5, 1.0));
            }
            s * 127.0
        });
        // Moving toward the lookahead, relative to where it faces; on a
        // ladder straight on (up it), while mantling not at all.
        let (mut forward, mut right) = (0i8, 0i8);
        let taking = rhs.neg.map(|(l, _)| l).or(neg_ahead.filter(|_| at_begin));
        // BO2's Bot_UpdateMovement on a link: at the begin, toward its finish
        // at half speed (from the eye_at, in the view's yaw axes); a drop, run
        // straight off, once it has been at the begin, full speed and
        // flat toward the finish. Away from the begin (fallen short of a
        // ledge) it walks back to the begin, at half speed within 64 units.
        // (BO2: the half-speed walk toward the finish needs the lookahead
        // under 64 units; farther, it walks to the lookahead.)
        let began = rhs.neg_began || bo2_at_begin;
        let close = la.is_some_and(|(_, reach)| reach < 64.0);
        let toward_end = link.filter(|l| if l.flags & NEG_DROP != 0 { began } else { bo2_at_begin && close });
        let speed = match (link, la) {
            (Some(l), Some((_, reach))) if l.flags & NEG_DROP == 0 && reach < 64.0 => 64.0,
            _ => speed,
        };
        if ladder {
            forward = 127;
        } else if let Some(l) = toward_end {
            if !mantling {
                if l.flags & NEG_DROP != 0 {
                    let wish = [l.end[0] - spot[0], l.end[1] - spot[1]];
                    let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
                    if len > 1e-3 {
                        let (s, c) = want[1].to_radians().sin_cos();
                        forward = (((wish[0] * c + wish[1] * s) / len).clamp(-1.0, 1.0) * 127.0) as i8;
                        right = (((wish[0] * s - wish[1] * c) / len).clamp(-1.0, 1.0) * 127.0) as i8;
                    }
                } else {
                    let e = [l.end[0] - eye_at[0], l.end[1] - eye_at[1], l.end[2] - eye_at[2]];
                    let len = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
                    if len > 1e-3 {
                        let e = [e[0] / len, e[1] / len];
                        let (sy, cy) = want[1].to_radians().sin_cos();
                        let ahead = (e[0] * cy + e[1] * sy).clamp(-1.0, 1.0);
                        let left = (e[1] * cy - e[0] * sy).clamp(-1.0, 1.0);
                        forward = (ahead * 64.0) as i8;
                        right = (left * -64.0) as i8;
                    }
                }
            }
        } else if !mantling && !hold && let Some(pt) = walk_to {
            let wish = [pt[0] - spot[0], pt[1] - spot[1]];
            let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
            if len > 1e-3 {
                let (s, c) = want[1].to_radians().sin_cos();
                forward = (((wish[0] * c + wish[1] * s) / len).clamp(-1.0, 1.0) * speed) as i8;
                right = (((wish[0] * s - wish[1] * c) / len).clamp(-1.0, 1.0) * speed) as i8;
            }
        }
        let mut press = 0u32;
        // Sprint (Bot_UpdateMovement): on a long straight run while facing
        // the way it goes, standing and not reloading; a sprint under 3 s
        // old carries on through a turn.
        if !sprinting {
            rhs.sprint_since = None;
        } else if rhs.sprint_since.is_none() {
            rhs.sprint_since = Some(now);
        }
        let reach = la.map_or(0.0, |(_, r)| r);
        let sprint = if speed == 0.0 || yaw_err >= 5.0 || reach < dv.sprint_distance {
            sprinting && rhs.sprint_since.is_some_and(|tt| now - tt < 3000)
        } else {
            !reloading(wstate) && wstate != weapon_t6::state::MELEE_INIT && standing
        };
        if sprint && !rhs.no_sprint && rhs.stance == 0 && !ladder && !mantling {
            press |= buttons::SPRINT;
        }
        match rhs.stance {
            1 => press |= buttons::DUCK_BUTTON_BIT,
            2 => press |= buttons::LYING_FLAT,
            _ => {}
        }
        // BO2: through a climb with no room to stand, the bot holds crouch
        // and lets go of prone, so it ends the climb ducked under the lintel.
        if low_climb {
            press = (press & !buttons::LYING_FLAT) | buttons::DUCK_BUTTON_BIT;
        }
        // Holding use, it presses nothing else (a plant, a defuse).
        if rhs.use_until > now {
            press = (press & (buttons::DUCK_BUTTON_BIT | buttons::LYING_FLAT)) | buttons::INTERACT_RELOAD_KEY;
        }
        // The trigger: one press when allowed, then a rest of
        // bot_minfiretime..bot_maxfiretime ms; a full-automatic holds it.
        // Firing lets go of sprint.
        if rhs.attack_ticks > 0 {
            press = (press & !buttons::SPRINT) | buttons::ATTACK;
            rhs.attack_ticks -= 1;
        } else if rhs.allow_attack && now >= rhs.fire_delay_until {
            press = (press & !buttons::SPRINT) | buttons::ATTACK;
            rhs.fire_delay_until = if full_auto { 0 } else { now + irand(n, now, dv.min_fire, dv.max_fire) };
        }
        if rhs.ads {
            press |= buttons::ADS;
        }
        // A melee only while not raising, lowering, meleeing or throwing (BO2's
        // states: the knife's swing and finish, the offhands, a detonator; the swim
        // in and out as a raise and drop, TODO(fact)).
        if std::mem::take(&mut rhs.melee)
            && !weapon_t6::state::is_raising_or_swim_out(wstate)
            && !weapon_t6::state::is_dropping_or_swim_in(wstate)
            && !(20..=29).contains(&wstate)
            && wstate != weapon_t6::state::DETONATING
        {
            press |= buttons::MELEE_CHARGE;
        }
        if std::mem::take(&mut rhs.dive) {
            press |= buttons::LYING_FLAT;
        }
        // The throw: its button once the view and the angles it chases both
        // face the point (so the view does not swing off it while the arm
        // comes up), or after THROW_AIM_MS of turning, and not while the
        // failsafe holds its buttons; then it faces there until the grenade
        // has left its hand (into the offhand states and out again). A press
        // the gun refused (still ready) is pressed again; the gun going down
        // first is part of the throw.
        let chasing = rhs.aim_avg;
        let free = rhs.failsafe.is_none();
        if let Some(tt) = rhs.throw.as_mut() {
            use weapon_t6::state;
            let offhand = state::is_secondary_gear(wstate) && wstate != state::OFFHAND_DETONATE;
            if offhand {
                tt.underway = true;
            } else if tt.underway || now - tt.asked >= THROW_GIVE_UP_MS {
                rhs.throw = None;
            } else if tt.pressed == 0 {
                let off = tt.at.map_or(0.0, |at| {
                    let LookAngles { yaw, pitch } = angles_to(eye_at, at);
                    let off = |a: [f32; 2]| angle_delta(a[1], yaw).abs().max(angle_delta(a[0], pitch).abs());
                    off([want[0], want[1]]).max(chasing.map_or(0.0, off))
                });
                if free && (off <= THROW_AIM_DEG || now - tt.asked >= THROW_AIM_MS) {
                    press |= tt.button;
                    tt.pressed = now;
                }
            } else if state::is_ready_as_mirrored(wstate) && now - tt.pressed >= 200 {
                tt.pressed = 0;
            }
        }
        // Jumps and vaults: BO2's engine takes a bot over the negotiation
        // links its path nodes mark (ledges, railings, low walls); ours
        // jumps where the route climbs close ahead (the movement code
        // mantles from a jump), and when walking makes no headway.
        // On a ladder it climbs (looking up its route, walking forward):
        // a jump would let go of it.
        // It takes a link from its begin.
        // At a jump link's begin, not mantling (BO2's Bot_Seek), it jumps
        // the first time; then while a ledge is in reach (the mantle hint,
        // so a jump climbs it on the way up), else again on the whole second
        // if it let go of jump the tick before. At a mantle link (none of
        // the jump, ladder or drop marks) it presses the climb once the
        // ledge is in reach.
        if let Some(l) = neg_ahead.filter(|_| at_begin) {
            rhs.neg = Some((l, now));
            rhs.neg_began = false;
            if botlog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} negotiation begin flags {:#x} at {:.0} {:.0} {:.0} begin {:.0} {:.0} {:.0} end {:.0} {:.0} {:.0}",
                    l.flags,
                    spot[0],
                    spot[1],
                    spot[2],
                    l.entry[0],
                    l.entry[1],
                    l.entry[2],
                    l.end[0],
                    l.end[1],
                    l.end[2],
                );
            }
        }
        // At a jump link's begin, there the tick before too, it looks for a
        // window pane in its way on the whole second (after this tick's
        // moves are set, below).
        let mut glass_check = false;
        if let Some((l, _)) = rhs.neg
            && bo2_at_begin
        {
            let jump = l.flags & NEG_JUMP != 0
                && (ledge || !rhs.neg_began || (rhs.last_press & buttons::LEAP == 0 && now % 1000 == 0));
            if !mantling && jump {
                press |= buttons::LEAP;
            }
            glass_check = l.flags & NEG_JUMP != 0 && rhs.neg_began && now % 1000 == 0;
            rhs.neg_began = true;
        }
        if next.is_some() && !ladder && climb_blocked && taking.is_none() {
            press |= buttons::LEAP;
        }
        let mut stuck_for = None;
        // At a jump link's begin the net below stays out of it: BO2 keeps the
        // route there and presses on the whole second, and a crouch from
        // bot_update_crouch (held 2.5 s, no jump while ducked) is waited out.
        // Backing off and dropping the route at 2.4 s gave up 28 of
        // Nuketown's 29 link failures (10-30).
        let at_jump_begin = bo2_at_begin && rhs.neg.is_some_and(|(l, _)| l.flags & NEG_JUMP != 0);
        if next.is_some() && rhs.failsafe.is_none() && !at_jump_begin {
            match rhs.last_gain {
                Some((at, since)) if dist2d(at, spot) < 12.0 => {
                    let stuck = now - since;
                    stuck_for = Some(stuck);
                    if stuck > 600 && (stuck / 300) % 2 == 0 && !ladder && ahead_blocked && taking.is_none() {
                        press |= buttons::LEAP;
                    }
                    // Not BO2: its think has no stuck handling, its only rescue
                    // is the script's bot_update_failsafe (3.5 s under 16 units
                    // of headway). A fan safety net: without it bots sat up to
                    // 8 s where our routes or collision are still off (Raid's
                    // y 3904 stairs; 10-30 runs). No headway: back off and step aside for a moment
                    // (a doorway corner, another player, a prop), then on.
                    if (1200..2400).contains(&stuck) && !ladder {
                        forward = -127;
                        right = if (n + (since / 3000) as u32) % 2 == 0 { 127 } else { -127 };
                    }
                    // Stuck again short of the same route point: the back-off
                    // restarts the stuck clock, so the 3 s drop below never
                    // came and bots looped there (Raid: back below the y 3904
                    // planter with the route's next point up on top). Drop the
                    // route now; the plan starts again from the node it is at.
                    let mut again = false;
                    if stuck >= 1200 && !ladder && let Some(pt) = next {
                        match rhs.stuck_next {
                            Some((was, began)) if began != since && dist2d(was, pt) < 1.0 && (was[2] - pt[2]).abs() < 1.0 => again = true,
                            Some((_, began)) if began == since => {}
                            _ => rhs.stuck_next = Some((pt, since)),
                        }
                    }
                    if again {
                        if botlog() {
                            diag::info!(Sim, "bo2mp bot {n} stuck again at {:.0} {:.0} {:.0}: new route", spot[0], spot[1], spot[2]);
                        }
                        rhs.stuck_next = None;
                        rhs.path.clear();
                        rhs.path_to = None;
                        rhs.replan_at = now + 250;
                        rhs.last_gain = Some((spot, now));
                    }
                    if stuck % 1000 == 0 && stuck > 0 && botlog() {
                        diag::info!(
                            Sim,
                            "bo2mp bot {n} stuck {stuck} ms at {:.0} {:.0} {:.0} next {:?} ahead_blocked {ahead_blocked} in_solid {in_solid} touching {touching:?} goal {:?} routes {} left {} hold {hold} look {} stick {forward} {right} stance {} lookahead {:?}",
                            spot[0],
                            spot[1],
                            spot[2],
                            next.map(|pt| [pt[0] as i32, pt[1] as i32, pt[2] as i32]),
                            goal.as_ref().map(|g| (g.name.as_str(), [g.pos[0] as i32, g.pos[1] as i32, g.pos[2] as i32], g.radius as i32)),
                            rhs.path_gen,
                            rhs.path.len(),
                            rhs.look.is_some(),
                            rhs.stance,
                            rhs.la_dir.map(|(_, l)| l as i32)
                        );
                        // BO2MP_BOTNEGLOG (test aid): the route it is stuck on.
                        if botneglog() {
                            let first: Vec<[i32; 3]> = rhs.path.iter().take(4).map(|pt| [pt[0] as i32, pt[1] as i32, pt[2] as i32]).collect();
                            diag::info!(
                                Sim,
                                "bo2mp bot {n} stuck route {} first {first:?} prev {:?}",
                                rhs.path_gen,
                                rhs.prev.map(|pt| [pt[0] as i32, pt[1] as i32, pt[2] as i32])
                            );
                        }
                    }
                    // Still stuck: drop the route; the next tick plans again
                    // (not on a ladder: climbing makes no flat headway).
                    if stuck > 3000 && !ladder {
                        rhs.path.clear();
                        rhs.path_to = None;
                        rhs.replan_at = now + 250;
                        rhs.last_gain = Some((spot, now));
                    }
                }
                _ => rhs.last_gain = Some((spot, now)),
            }
        } else {
            rhs.last_gain = None;
        }
        // The failsafe (BO2's Bot_UpdateFailsafe), set by the stuck script:
        // it overrides the rest of the think. No sprint and no jump of its
        // own (G_BotThink still applies the script's buttons: stance, use,
        // fire, aim, melee), the view left alone; on a ladder straight on,
        // else straight at the node from the eye_at in the view's yaw axes;
        // within 15 units it stands and notifies "goal" with the node each
        // think until the script clears it.
        let mut failsafe_goal = None;
        if let Some((pt, node)) = rhs.failsafe {
            press &= !(buttons::SPRINT | buttons::LEAP);
            want = view;
            (forward, right) = (0, 0);
            if ladder {
                forward = 127;
            } else if (0..3).map(|i| (pt[i] - spot[i]).powi(2)).sum::<f32>() >= 225.0 {
                let e = [pt[0] - eye_at[0], pt[1] - eye_at[1], pt[2] - eye_at[2]];
                let len = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt();
                let e = if len > 0.0 { [e[0] / len, e[1] / len] } else { [e[0], e[1]] };
                let (sy, cy) = view[1].to_radians().sin_cos();
                let ahead = (e[0] * cy + e[1] * sy).clamp(-1.0, 1.0);
                let left = (e[1] * cy - e[0] * sy).clamp(-1.0, 1.0);
                forward = (ahead * 127.0) as i8;
                right = (left * -127.0) as i8;
            } else {
                failsafe_goal = Some(node);
            }
            rhs.last_gain = None;
        }
        rhs.last_press = press;
        let fire_dbg = botfirelog().then(|| (rhs.allow_attack, rhs.fire_delay_until, rhs.look.is_some()));
        // BO2MP_BOTLOG=1: each bot every 2 s (test aid).
        if now % 2000 == 0 && botlog() {
            diag::info!(
                Sim,
                "bo2mp botlog {n}: at {:.0} {:.0} {:.0} view {:.0} {:.0} goal {:?} path {} walk {:?} face {:?} look {:?} attack {} ads {} press {press:#x} move {forward} {right} sprint {sprint} dodge {} hold {hold}",
                spot[0],
                spot[1],
                spot[2],
                view[0],
                view[1],
                goal.as_ref().map(|g| g.name.as_str()),
                rhs.path.len(),
                walk_to.map(|pt| [pt[0] as i32, pt[1] as i32]),
                far_seen.map(|pt| [pt[0] as i32, pt[1] as i32]),
                rhs.look.map(|l| [l[0] as i32, l[1] as i32, l[2] as i32]),
                rhs.allow_attack,
                rhs.ads,
                walk_to != next,
            );
        }
        let neg_dbg = rhs
            .neg
            .filter(|_| now % 100 == 0 && botneglog())
            .map(|(l, since)| (l.entry, now - since, press, forward, right, want[0], bo2_at_begin));
        let stuck_dbg = stuck_for
            .filter(|&s| s >= 200 && now % 100 == 0 && botneglog())
            .map(|s| (s, forward, right, view[1], walk_to, rhs.look, rhs.neg.map(|(l, _)| l.entry)));
        drop(zm);
        // BO2MP_BOTNEGLOG (test aid): a bot making no headway, every 100 ms:
        // how it moves, and what a player's box meets 24 units toward its walk.
        if let Some((stuck, forward, right, yaw, to, look, on_link)) = stuck_dbg
            && let Some((o, v, ground)) = frame(sim)
                .avatar(ClientId(n))
                .map(|pstate| (pstate.origin, pstate.velocity, pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT))
        {
            use crate::shot_trace::{PLAYER_BODY_MASK, BODY_BOX_UPPER, BODY_BOX_LOWER};
            let to = to.unwrap_or(o);
            let d = [to[0] - o[0], to[1] - o[1]];
            let len = d[0].hypot(d[1]).max(1e-3);
            let finish = [o[0] + d[0] / len * 24.0, o[1] + d[1] / len * 24.0, o[2]];
            let tt = frame(sim).trace_clip(o, finish, BODY_BOX_LOWER, BODY_BOX_UPPER, PLAYER_BODY_MASK);
            diag::info!(
                Sim,
                "bo2mp bot {n} stuckdbg {stuck} ms: at {:.1} {:.1} {:.1} vel {:.0} {:.0} {:.0} ground {ground} yaw {yaw:.0} move {forward} {right} walk {:.0} {:.0} box frac {:.2} normal {:.2} {:.2} {:.2} contents {:#x} start {} look {:?} link {:?}",
                o[0],
                o[1],
                o[2],
                v[0],
                v[1],
                v[2],
                to[0],
                to[1],
                tt.fraction,
                tt.normal[0],
                tt.normal[1],
                tt.normal[2],
                tt.cflags,
                tt.starts_inside,
                look.map(|pt| pt.map(|c| c as i32)),
                on_link.map(|pt| pt.map(|c| c as i32))
            );
        }
        if let Some((begin, took, press, forward, right, pitch, at)) = neg_dbg
            && let Some(pstate) = frame(sim).avatar(ClientId(n))
        {
            let [ox, oy, oz] = pstate.origin;
            let [vx, vy, vz] = pstate.velocity;
            diag::info!(
                Sim,
                "bo2mp bot {n} on link {:?} {took} ms: at {:.0} {:.0} {:.0} vel {:.0} {:.0} {:.0} ground {} ledge {} mantling {} jump {} move {forward} {right} pitch {pitch:.0} at begin {at}",
                begin.map(|c| c as i32),
                ox,
                oy,
                oz,
                vx,
                vy,
                vz,
                pstate.standing_on != playerstate_t6::game::NO_ENTITY_SLOT,
                pstate.mantle_state.flags & playerstate_t6::game::ledge_climb_bits::ACTIVE != 0,
                pstate.movement_state_bits & playerstate_t6::game::movement_state_bits::MANTLE != 0,
                press & buttons::LEAP != 0,
            );
        }
        if glass_check {
            glass_ahead(sim, n, eye_at, view);
        }
        if let Some(node) = failsafe_goal {
            let obj = sim.resource::<Zm>().players.get(&n).map(|pt| pt.obj);
            if let Some(obj) = obj {
                let args: Vec<ScriptValue> = node.map(ScriptValue::Object).into_iter().collect();
                super::with_vm(sim, |vm, sim| vm.notify_str(sim, obj, "goal", &args));
            }
        }
        // BO2MP_BOTS_STILL=1: the bots stand where they are and do nothing;
        // =fire:N (or fire:N,M) and those bots fire where they face too
        // (test aids, for a target that stays put and a shooter that does).
        if let Some(still) = std::env::var_os("BO2MP_BOTS_STILL") {
            let firing = still
                .to_str()
                .and_then(|s| s.strip_prefix("fire:"))
                .is_some_and(|ids| ids.split(',').any(|i| i.trim().parse::<u32>() == Ok(n)));
            let attack = if firing && (now / 100) % 2 == 0 { buttons::ATTACK } else { 0 };
            (forward, right, press, want) = (0, 0, attack, view);
        }
        // `bot hold on`: keep looking where it looks, move and press nothing.
        if held {
            (forward, right, press, want) = (0, 0, 0, view);
        }
        if let Some((allow, delay_until, looking)) = fire_dbg {
            let line = FireLog {
                now,
                yaw_cmd: want[1],
                steer_err: yaw_err,
                arm: weapon,
                wstate,
                sprint_on: sprinting,
                sprint_wanted: sprint,
                allow,
                delay_until,
                looking,
                press,
            };
            log_botfire(sim, n, &line);
        }
        // Its command for the tick, the view turned as a script's
        // setplayerangles turns it (the delta under the command angles).
        let mut cmd_angles = None;
        {
            let mut req = sim.resource_mut::<crate::frame_step::FrameTicket>();
            for (id, cmd) in &mut req.input.cmds {
                if id.0 != n {
                    continue;
                }
                cmd.buttons = press;
                cmd.thrust_ahead = forward;
                cmd.thrust_lateral = right;
                // Its gun, not the hand: an empty hand mid-throw would
                // otherwise ask for no gun, and the throw never ends.
                if select != 0 {
                    cmd.weapon = select as u16;
                    cmd.weapon_slot_code = select as u16;
                }
                cmd_angles = Some(cmd.angles);
            }
        }
        if let Some(angles) = cmd_angles {
            let mut f = frame(sim);
            if let Some(pt) = f.occupant_mut(ClientId(n)) {
                pt.view_offset_angles = std::array::from_fn(|i| {
                    let cmd_deg = (angles[i] as u16) as f32 * (360.0 / 65536.0);
                    want[i] - cmd_deg
                });
                pt.look_angles = want;
            }
        }
    }
}

/// The gun a bot's command asks for, by a player's client rule
/// (`follow_held_weapon_select`): an empty hand changes nothing, so the gun an
/// offhand throw puts away is asked back when the throw ends (BO2 brings it
/// back by the usercmd's weapon); a gun the game puts in hand is followed
/// while the bot was asking for the one it held.
fn follow_select(select: &mut u32, held: &mut u32, gun: u32) {
    if gun == 0 {
        return;
    }
    if *select == 0 || (*select == *held && gun != *held) {
        *select = gun;
    }
    *held = gun;
}

/// The route straight to `goal`, crossing no links.
fn straight_to(goal: [f32; 3]) -> (Vec<[f32; 3]>, Vec<NegLink>) {
    (vec![goal], vec![])
}

/// A route from a place to a goal over the path nodes (the goal last),
/// and the negotiation links it crosses; straight there when the map has no
/// route between them. It starts at `start` when given (a link's end), else
/// at the node nearest the place.
fn plan(world: &mut World, from: [f32; 3], start: Option<u32>, goal: [f32; 3]) -> (Vec<[f32; 3]>, Vec<NegLink>) {
    let a = start.or_else(|| nearest_reachable(world, from));
    let (Some(a), Some(b)) = (a, nearest_reachable(world, goal)) else {
        return straight_to(goal);
    };
    let nav = &world.resource::<Zm>().nav;
    let Some(nodes) = nav.bot_path(a, b) else {
        if botneglog() {
            diag::info!(Sim, "bo2mp plan no route: from {from:?} node {a} to {goal:?} node {b}");
        }
        return straight_to(goal);
    };
    let mut out: Vec<[f32; 3]> = nodes
        .iter()
        .map(|&i| nav.graph[i as usize].origin)
        .collect();
    // BO2MP_BOTNEGLOG (test aid): a leg between linked nodes over 512 units.
    if botneglog() {
        for (w, p) in nodes.windows(2).zip(out.windows(2)) {
            if dist2d(p[0], p[1]) > 512.0 {
                diag::info!(Sim, "bo2mp plan long leg: node {} {:?} to node {} {:?}", w[0], p[0], w[1], p[1]);
            }
        }
    }
    let negs: Vec<NegLink> = (0..nodes.len().saturating_sub(1))
        .filter(|&i| nav.graph[nodes[i] as usize].ty == super::nav::NODE_NEGOTIATION_BEGIN && nav.negotiation(nodes[i], nodes[i + 1]))
        .map(|i| NegLink {
            entry: nav.graph[nodes[i] as usize].origin,
            end: nav.graph[nodes[i + 1] as usize].origin,
            flags: nav.graph[nodes[i] as usize].spawnflags,
            after_node: after_link_end(&nodes, i),
        })
        .collect();
    out.push(goal);
    // BO2 smooths each path it plans (Path_SmoothPath: from a node straight
    // on goal the farthest later one it can walk goal). Ours does it from where
    // the bot stands, up goal three nodes on (a trace each), so a nearest
    // node behind it is no U-turn it then rocks on.
    for _ in 0..3 {
        if out.len() <= 1 || !walk_clear(world, from, out[1]) {
            break;
        }
        out.remove(0);
    }
    (out, negs)
}

/// Whether a link being taken is done (BO2's Bot_AtNegotiationEnd, once the
/// bot has been at its begin): its mantle just finished, or a ladder or drop
/// once it is off the ground, or any other link once it stands (not
/// mantling) within 15 units of the link's end.
fn link_done(flags: u32, mantle_finished: bool, grounded: bool, mantling: bool, to_end: f32) -> bool {
    if mantle_finished {
        return true;
    }
    if flags & (NEG_LADDER | NEG_DROP) != 0 {
        return !grounded;
    }
    grounded && !mantling && to_end < 15.0
}

/// The node a route planned once the link starting at `nodes[begin]` is
/// done starts from: the one after the link's end (BO2's pts[start - 1],
/// its route being stored goal first), else the end itself.
fn after_link_end(nodes: &[u32], begin: usize) -> u32 {
    nodes.get(begin + 2).or(nodes.get(begin + 1)).copied().unwrap_or(nodes[begin])
}

/// The map's negotiation links by kind, once a run (test aid).
fn log_negotiations(world: &World) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let nav = &world.resource::<Zm>().nav;
    if nav.graph.is_empty() {
        return;
    }
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut kinds: BTreeMap<String, u32> = BTreeMap::new();
    for node in nav.graph.iter().filter(|n| n.ty == super::nav::NODE_NEGOTIATION_BEGIN) {
        let f = node.spawnflags;
        let kind = if f & NEG_LADDER != 0 {
            "ladder"
        } else if f & NEG_DROP != 0 {
            "drop"
        } else if f & NEG_JUMP != 0 {
            "jump"
        } else if !node.animscript.is_empty() {
            "mantle"
        } else {
            "other"
        };
        *kinds.entry(format!("{kind} {:#x}", f & (NEG_JUMP | NEG_LADDER | NEG_DROP))).or_default() += 1;
    }
    diag::info!(Sim, "bo2mp negotiation links: {kinds:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ladder_link_is_done_once_the_bot_leaves_the_ground() {
        // BO2's Bot_AtNegotiationEnd: a ladder (or a drop) is done the
        // moment the bot is off the ground, at the foot, with no top check.
        assert!(link_done(NEG_LADDER, false, false, false, 200.0));
        assert!(link_done(NEG_DROP, false, false, false, 200.0));
        // Still on the ground at the foot: not yet, even right at the end.
        assert!(!link_done(NEG_LADDER, false, true, false, 5.0));
        // A mantle that just finished ends any link.
        assert!(link_done(NEG_LADDER, true, true, false, 200.0));
        // A jump: on the ground within 15 units of the end, not mantling.
        assert!(link_done(NEG_JUMP, false, true, false, 14.0));
        assert!(!link_done(NEG_JUMP, false, false, false, 14.0));
        assert!(!link_done(NEG_JUMP, false, true, true, 14.0));
        assert!(!link_done(NEG_JUMP, false, true, false, 15.0));
    }

    #[test]
    fn the_route_after_a_link_starts_past_its_end() {
        // Route nodes 4 (begin) -> 9 (end) -> 12: the next route starts at 12,
        // as BO2's pts[start - 1]; a route ending at the link's end, there.
        assert_eq!(after_link_end(&[3, 4, 9, 12], 1), 12);
        assert_eq!(after_link_end(&[3, 4, 9], 1), 9);
    }

    #[test]
    fn a_throw_is_taken_only_as_bo2_takes_it() {
        // Ammo, nothing waiting, the wait over: taken.
        assert!(takes_throw(1, false, 0, 5000));
        assert!(takes_throw(2, false, 5000, 5000));
        // No ammo for it.
        assert!(!takes_throw(0, false, 0, 5000));
        // One still waiting to leave the hand: the second ask is refused
        // (the lethal of bot_combat's lethal-then-tactical pair stays).
        assert!(!takes_throw(1, true, 0, 5000));
        // Before the wait after the last one taken is over.
        assert!(!takes_throw(1, false, 5001, 5000));
    }

    #[test]
    fn the_gun_put_away_for_a_throw_is_asked_back() {
        let (mut select, mut held) = (0, 0);
        // Spawns holding gun 7: asks for it.
        follow_select(&mut select, &mut held, 7);
        assert_eq!(select, 7);
        // The throw empties the hand: it still asks for gun 7.
        follow_select(&mut select, &mut held, 0);
        assert_eq!((select, held), (7, 7));
        // The game (switchtoweapon) puts gun 9 in hand: followed.
        follow_select(&mut select, &mut held, 9);
        assert_eq!(select, 9);
        // Asking for another gun than the one held: a new gun in hand does
        // not move the ask (a player's pick stands).
        select = 4;
        follow_select(&mut select, &mut held, 5);
        assert_eq!((select, held), (4, 5));
    }

    #[test]
    fn a_failsafe_does_not_outlive_the_bot() {
        // The script's failsafe set, then the bot dies: the new life walks
        // its own route, not at the last life's node.
        let mut b = Bot { failsafe: Some(([-808.0, -496.0, 68.0], None)), ..Default::default() };
        b.path.push([864.0, 838.0, -1.0]);
        clear_for_new_life(&mut b);
        assert!(b.failsafe.is_none());
        assert!(b.path.is_empty());
    }

    fn avoider(id: u32, pos: [f32; 2], motion_vec: [f32; 2]) -> Avoider {
        Avoider { id, pos: [pos[0], pos[1], 0.0], motion_vec, radius: BOT_AVOID_RADIUS, firm: false, team: "allies".into() }
    }

    #[test]
    fn touching_teammates_slide_apart_instead_of_freezing() {
        // Two stopped bots 40 units apart (their circles touch at 60) each
        // want to walk through the other: BO2 only rules out closing in
        // (dead in line they wait), it no longer flings them apart.
        let a = avoider(1, [0.0, 0.0], [0.0, 0.0]);
        let b = avoider(2, [40.0, 0.0], [0.0, 0.0]);
        let va = avoid_velocity(&a, [100.0, 0.0], BOT_AVOID_SPEED, &[&b]);
        let vb = avoid_velocity(&b, [-100.0, 0.0], BOT_AVOID_SPEED, &[&a]);
        for (v, toward) in [(va, 1.0), (vb, -1.0)] {
            assert!(v[0] * toward <= 0.01, "closes in: {v:?}");
        }
        // Off to one side of the gap, the slide keeps a real walk.
        let c = avoider(3, [40.0, 20.0], [0.0, 0.0]);
        let v = avoid_velocity(&a, [100.0, 0.0], BOT_AVOID_SPEED, &[&c]);
        assert!(v[0] * 40.0 + v[1] * 20.0 <= 0.01 && v[0].hypot(v[1]) > 40.0, "{v:?}");
    }

    #[test]
    fn head_on_teammates_pass_on_opposite_sides() {
        // Walking at each other from 200 units, slightly off line: each
        // takes half the dodge and they pass, neither stopping.
        let a = avoider(1, [0.0, 0.0], [100.0, 0.0]);
        let b = avoider(2, [200.0, 5.0], [-100.0, 0.0]);
        let va = avoid_velocity(&a, [100.0, 0.0], BOT_AVOID_SPEED, &[&b]);
        let vb = avoid_velocity(&b, [-100.0, 0.0], BOT_AVOID_SPEED, &[&a]);
        assert!(va[1] < 0.0 && vb[1] > 0.0, "{va:?} {vb:?}");
        assert!(va[0] > 0.0 && vb[0] < 0.0 && va[0].hypot(va[1]) > 50.0 && vb[0].hypot(vb[1]) > 50.0);
    }

    #[test]
    fn alone_a_bot_keeps_its_way_capped_at_the_avoid_speed() {
        let a = avoider(1, [0.0, 0.0], [190.0, 0.0]);
        assert_eq!(avoid_velocity(&a, [190.0, 0.0], BOT_AVOID_SPEED, &[]), [100.0, 0.0]);
    }
}
