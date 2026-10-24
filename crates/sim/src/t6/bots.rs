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

use std::collections::BTreeMap;

use bevy_ecs::prelude::World;
use gsc_t6::{Array, ObjKind, ObjRef, Value, Vm};
use playerstate_t6::game::buttons;

use super::{Player, Zm, arg, entnum, frame, list, origin_of, text};
use crate::world::ClientId;

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
    /// (weapon, attachment, slot such as "primaryattachment1").
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
    begin: [f32; 3],
    end: [f32; 3],
    flags: u32,
    /// The end's node: the route planned once the link is done starts there.
    end_node: u32,
}

const NEG_JUMP: u32 = 0x80_0000;
const NEG_LADDER: u32 = 0x40_0000;
const NEG_DROP: u32 = 0x200_0000;
/// Looking up a ladder (degrees, up): on one, BO2's climb goes up at full
/// speed from about 9 degrees up and down from 15 degrees down.
const LADDER_LOOK_PITCH: f32 = -30.0;
/// A ladder link is given up after this long (ms; other links 4 s): the
/// tallest climbs 284 units.
const LADDER_GIVE_UP_MS: i64 = 8000;

/// A grenade throw: where (none: wherever it faces), its button (FRAG a
/// lethal, SMOKE a tactical), when it was asked for, when the button went
/// down (0: not yet) and whether the hand has started the throw.
#[derive(Clone, Copy, Debug)]
struct Throw {
    at: Option<[f32; 3]>,
    button: u32,
    asked: i64,
    pressed: i64,
    started: bool,
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
fn takes_throw(ammo: i32, pending: bool, wait_until: i64, now: i64) -> bool {
    ammo > 0 && !pending && wait_until <= now
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
    progress: Option<([f32; 3], i64)>,
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

/// Yaw and pitch (degrees, the game's: pitch down positive) from a to b.
fn angles_to(a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let yaw = d[1].atan2(d[0]).to_degrees();
    let flat = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let pitch = -d[2].atan2(flat).to_degrees();
    (yaw, pitch)
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
fn player_is(vm: &Vm<World>, world: &mut World, s: &Value, read: fn(&playerstate_t6::game::PlayerState) -> bool) -> Value {
    let n = entnum(vm, s).unwrap_or(u32::MAX);
    Value::bool(frame(world).player(ClientId(n)).is_some_and(read))
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
    yaw_speed: f32,
    min_fire: i64,
    max_fire: i64,
    sprint_distance: f32,
    /// aim_turnrate_pitch / _ads, aim_turnrate_yaw / _ads (degrees a
    /// second) and aim_accel_turnrate_lerp (degrees a second, a second).
    pitch_rate: [f32; 2],
    yaw_rate: [f32; 2],
    accel: f32,
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
                yaw_speed: f("bot_yawspeed", 1.0),
                min_fire,
                max_fire: max_fire.max(min_fire),
                sprint_distance: f("bot_sprintdistance", 256.0),
                pitch_rate: [f("aim_turnrate_pitch", 90.0), f("aim_turnrate_pitch_ads", 55.0)],
                yaw_rate: [f("aim_turnrate_yaw", 260.0), f("aim_turnrate_yaw_ads", 90.0)],
                accel: f("aim_accel_turnrate_lerp", 1200.0),
            }
        })
        .unwrap_or(Self {
            yaw_speed: 1.0,
            min_fire: 400,
            max_fire: 600,
            sprint_distance: 256.0,
            pitch_rate: [90.0, 55.0],
            yaw_rate: [260.0, 90.0],
            accel: 1200.0,
        })
    }
}

/// A number in lo..hi from the bot and the time (splitmix).
fn irand(n: u32, now: i64, lo: i64, hi: i64) -> i64 {
    if hi <= lo {
        return lo;
    }
    let mut z = (now as u64) ^ ((n as u64) << 40) ^ 0x9E37_79B9_7F4A_7C15;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    lo + (z % (hi - lo) as u64) as i64
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
fn farthest_seen(world: &mut World, origin: [f32; 3], path: &[[f32; 3]]) -> Option<[f32; 3]> {
    // The nodes: all but the goal point at the end.
    let nodes = &path[..path.len().saturating_sub(1)];
    let start = usize::from(nodes.len() > 1 && dist2d(origin, nodes[1]) < 192.0);
    let raise = |p: [f32; 3]| [p[0], p[1], p[2] + 60.0];
    let from = raise(*nodes.get(start)?);
    let last = nodes.len().checked_sub(1)?.min(start + 20);
    for &p in nodes.get(start + 1..=last)?.iter().rev() {
        if dist2d(nodes[start], p) < 2048.0 && sight(world, from, raise(p)) {
            return Some(p);
        }
    }
    None
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
        let target = axis[i].abs() * rate[i] * dv.yaw_speed;
        speed[i] = if target <= speed[i] {
            target
        } else {
            (speed[i] + dv.accel * dv.yaw_speed * 0.05).min(target)
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
    vel: [f32; 2],
    radius: f32,
    /// A player (not a bot): it does not dodge, so a bot takes all of it.
    firm: bool,
    team: String,
}

/// Everyone in play this tick, for the bots' avoidance; empty without
/// teams (free-for-all bots do not avoid).
fn crowd(world: &mut World) -> Vec<Avoider> {
    let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for id in ids {
        if !alive(world, id) {
            continue;
        }
        let Some((pos, vel)) = frame(world)
            .player(ClientId(id))
            .map(|ps| (ps.origin, [ps.velocity[0], ps.velocity[1]]))
        else {
            continue;
        };
        let bot = world.resource::<Zm>().bots.contains_key(&id);
        let radius = if bot { BOT_AVOID_RADIUS } else { PLAYER_AVOID_RADIUS };
        out.push(Avoider { id, pos, vel, radius, firm: !bot, team: String::new() });
    }
    let teams = super::with_vm(world, |vm, world| {
        teambased(vm).then(|| out.iter().map(|a| team_of(vm, world, a.id)).collect::<Vec<_>>())
    })
    .flatten();
    match teams {
        Some(teams) => {
            for (a, t) in out.iter_mut().zip(teams) {
                a.team = t;
            }
            out
        }
        None => Vec::new(),
    }
}

fn det2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot2(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// The velocity nearest `pref` (no faster than `max_speed`) that keeps
/// `me` clear of `others` over the horizon: each neighbour gives a
/// half-plane of allowed velocities (half the dodge each between bots, all
/// of it around a player), met one at a time; when one cannot be met it
/// keeps what it had.
fn avoid_velocity(me: &Avoider, pref: [f32; 2], max_speed: f32, others: &[&Avoider]) -> [f32; 2] {
    let inv_t = 1.0 / AVOID_HORIZON;
    let inv_tick = 1000.0 / crate::MATCH_TICK_MS as f32;
    // Each half-plane as (a point on its edge, the edge's direction); the
    // allowed side is to the left.
    let mut lines: Vec<([f32; 2], [f32; 2])> = Vec::with_capacity(others.len());
    for o in others {
        let rel_pos = [o.pos[0] - me.pos[0], o.pos[1] - me.pos[1]];
        let rel_vel = [me.vel[0] - o.vel[0], me.vel[1] - o.vel[1]];
        let dist_sq = dot2(rel_pos, rel_pos);
        let r = me.radius + o.radius;
        let share = if o.firm { 1.0 } else { 0.5 };
        let (dir, u);
        // Off the edge nearest the velocity: the cut-off circle at the
        // horizon, either side cone leg, or (already touching) the circle
        // one tick ahead.
        let toward = |w: [f32; 2], k: f32| {
            let len = dot2(w, w).sqrt().max(1e-6);
            let unit = [w[0] / len, w[1] / len];
            ([unit[1], -unit[0]], [(k - len) * unit[0], (k - len) * unit[1]])
        };
        if dist_sq > r * r {
            let w = [rel_vel[0] - inv_t * rel_pos[0], rel_vel[1] - inv_t * rel_pos[1]];
            let along = dot2(w, rel_pos);
            if along < 0.0 && along * along > r * r * dot2(w, w) {
                (dir, u) = toward(w, r * inv_t);
            } else {
                let leg = (dist_sq - r * r).sqrt();
                dir = if det2(rel_pos, w) > 0.0 {
                    [(rel_pos[0] * leg - rel_pos[1] * r) / dist_sq, (rel_pos[0] * r + rel_pos[1] * leg) / dist_sq]
                } else {
                    [-(rel_pos[0] * leg + rel_pos[1] * r) / dist_sq, -(-rel_pos[0] * r + rel_pos[1] * leg) / dist_sq]
                };
                let d = dot2(rel_vel, dir);
                u = [d * dir[0] - rel_vel[0], d * dir[1] - rel_vel[1]];
            }
        } else {
            let w = [rel_vel[0] - inv_tick * rel_pos[0], rel_vel[1] - inv_tick * rel_pos[1]];
            (dir, u) = toward(w, r * inv_tick);
        }
        lines.push(([me.vel[0] + share * u[0], me.vel[1] + share * u[1]], dir));
    }
    let len = dot2(pref, pref).sqrt();
    let mut out = if len > max_speed { [pref[0] / len * max_speed, pref[1] / len * max_speed] } else { pref };
    for i in 0..lines.len() {
        let (p, d) = lines[i];
        if det2(d, [p[0] - out[0], p[1] - out[1]]) > 0.0 {
            match on_line(&lines[..=i], max_speed, pref) {
                Some(v) => out = v,
                None => break,
            }
        }
    }
    out
}

/// The point on the last line nearest `opt` inside the speed circle and the
/// earlier half-planes, if any.
fn on_line(lines: &[([f32; 2], [f32; 2])], radius: f32, opt: [f32; 2]) -> Option<[f32; 2]> {
    let (&(p, d), rest) = lines.split_last()?;
    let dp = dot2(p, d);
    let disc = dp * dp + radius * radius - dot2(p, p);
    if disc < 0.0 {
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
fn avoid_clear(world: &mut World, origin: [f32; 3], to: [f32; 3]) -> bool {
    let a = [origin[0], origin[1], origin[2] + 18.0];
    let b = [to[0], to[1], to[2] + 18.0];
    let t = frame(world).trace_static_world(a, b, [0.0; 3], [0.0; 3], AVOID_CLIP);
    if t.startsolid || t.allsolid {
        return false;
    }
    if t.fraction < 1.0 {
        let h = t.fraction * 0.5;
        let m = [a[0] + (b[0] - a[0]) * h, a[1] + (b[1] - a[1]) * h];
        let d = [origin[0] - m[0], origin[1] - m[1]];
        return dot2(d, d) >= 225.0;
    }
    true
}

/// The eye of a player: his origin plus his view height.
fn eye(world: &mut World, n: u32) -> Option<[f32; 3]> {
    frame(world)
        .player(ClientId(n))
        .map(|ps| [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current])
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
    playing && frame(world).player(ClientId(n)).is_some_and(|ps| ps.health > 0)
}

fn team_of(vm: &mut Vm<World>, world: &World, n: u32) -> String {
    let Some(p) = world.resource::<Zm>().players.get(&n) else {
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
    let (fwd, _, _) = game_math::angle_vectors(view);
    let end = [0, 1, 2].map(|i| eye[i] + fwd[i] * 64.0);
    let (with, without) = {
        let f = frame(world);
        (
            f.trace_clip(eye, end, [0.0; 3], [0.0; 3], MASK_SOLID_GLASS),
            f.trace_navigation(eye, end, [0.0; 3], [0.0; 3], MASK_SOLID_GLASS),
        )
    };
    // Glass is what stops the line only when the line runs on without it.
    if with.startsolid || with.fraction >= 1.0 || with.fraction >= without.fraction {
        return;
    }
    let at = with.end_pos(eye, end);
    if botlog() {
        diag::info!(Sim, "bo2mp bot {n} glass ahead at {:.0} {:.0} {:.0}", at[0], at[1], at[2]);
    }
    let obj = world.resource::<Zm>().players.get(&n).map(|p| p.obj);
    if let Some(obj) = obj {
        super::with_vm(world, |vm, world| vm.notify_str(world, obj, "glass", &[Value::Vec3(at)]));
    }
}

fn blocked_ahead(world: &mut World, origin: [f32; 3], to: [f32; 3]) -> bool {
    let d = [to[0] - origin[0], to[1] - origin[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len < 1.0 {
        return false;
    }
    let end = [origin[0] + d[0] / len * 24.0, origin[1] + d[1] / len * 24.0, origin[2]];
    let t = step_trace(world, origin, end);
    t.allsolid || t.fraction < 1.0
}

/// A player's box raised by the 18-unit step, from `from` to `to` (both at
/// the feet), as BO2's Path_PredictionTrace sweeps it: a box that starts
/// inside the map (a spawn set into a clip brush) is shrunk by 5 units
/// (lookaheadTraceFudge) across and on top, down to 10 across, and is then
/// judged by the sweep alone, so a bot can walk out of the brush.
fn step_trace(world: &mut World, from: [f32; 3], to: [f32; 3]) -> trace_t6::Trace {
    use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
    let start = [from[0], from[1], from[2] + 18.0];
    let end = [to[0], to[1], to[2] + 18.0];
    let mut r = PLAYER_MAXS[0];
    let mut top = PLAYER_MAXS[2] - 18.0;
    loop {
        let t = frame(world).trace_clip(start, end, [-r, -r, PLAYER_MINS[2]], [r, r, top], MASK_PLAYER_SOLID);
        if !t.startsolid || r <= 10.0 {
            return t;
        }
        r -= 5.0;
        top -= 5.0;
    }
}

/// BO2's prediction trace to the lookahead: can a player walk straight
/// from `origin` to `to` (its box raised by the 18-unit step, against the
/// map and what stands in it, as the walk itself collides)?
fn walk_clear(world: &mut World, origin: [f32; 3], to: [f32; 3]) -> bool {
    if dist2d(origin, to) < 1.0 {
        return true;
    }
    let t = step_trace(world, origin, to);
    !t.allsolid && t.fraction >= 1.0
}

/// BO2's nearest node to a place (Path_NearestNode, as its path search
/// starts and ends): of `near_cylinder`'s nodes, the first the place stands
/// in (the node's box: 15 each way across, 48 up) or that a body-sized box
/// (-10 -10 17 to 10 10 48) reaches in a straight line; a bot upstairs no
/// longer starts from the node on the floor below. None: our closest node.
fn nearest_reachable(world: &mut World, p: [f32; 3]) -> Option<u32> {
    use crate::bullet_collision::MASK_PLAYER_SOLID;
    let near = world.resource::<Zm>().nav.near_cylinder(p);
    for (i, o) in near {
        let inside = (p[0] - o[0]).abs() < 16.0 && (p[1] - o[1]).abs() < 16.0 && p[2] > o[2] - 1.0 && p[2] < o[2] + 49.0;
        if inside {
            return Some(i);
        }
        let t = frame(world).trace_clip(p, o, [-10.0, -10.0, 17.0], [10.0, 10.0, 48.0], MASK_PLAYER_SOLID);
        if !t.startsolid && t.fraction >= 1.0 {
            return Some(i);
        }
    }
    world.resource::<Zm>().nav.nearest(p)
}

/// BO2MP_BOTLOG=1 (test aid), read once.
fn botlog() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_BOTLOG").is_some())
}

/// BO2MP_BOTNEGLOG=1 (test aid), read once: a line every 100 ms while a bot
/// takes a link (where it is, how fast, on the ground or not, the mantle hint,
/// the jump pressed), to show why it falls short.
fn botneglog() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_BOTNEGLOG").is_some())
}

/// BO2MP_BOTFIRELOG=1 (test aid), read once: one line per bot per tick while
/// an enemy is in sight or within 400 units, to show why a bot holds fire.
fn botfirelog() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_BOTFIRELOG").is_some())
}

/// What a bot's think settled on for the tick, for the fire log.
struct FireLog {
    now: i64,
    /// The view yaw the command carries (after the steering).
    view_yaw: f32,
    /// How far the steering still has to turn toward what it chases.
    steer_err: f32,
    weapon: u32,
    wstate: i32,
    sprinting: bool,
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
    let clip = frame(world).combat_facts_for(l.weapon).and_then(|facts| {
        let key = weapon_t6::ammo_table::clip_table_key(facts.clip_index, l.weapon);
        frame(world)
            .player(ClientId(n))
            .filter(|ps| weapon_t6::ammo_table::clip_row_present(&ps.ammoclip, key))
            .map(|ps| weapon_t6::ammo_table::get_clip_for_hand(&ps.ammoclip, key, 0))
    });
    let wtime = frame(world).player(ClientId(n)).map_or(0, |ps| ps.weapon_time);
    let shot = {
        let mut zm = world.resource_mut::<Zm>();
        let Some(b) = zm.bots.get_mut(&n) else { return };
        let prev = b.fire_dbg.replace((l.weapon, clip.unwrap_or(-1)));
        prev.is_some_and(|(w, c)| w == l.weapon && c >= 0 && clip.is_some_and(|now| now < c))
    };
    let Some(from) = eye(world, n) else { return };
    let enemies = super::with_vm(world, |vm, world| enemies_of(vm, world, n, true)).unwrap_or_default();
    let mut best: Option<(f32, u32, bool, [f32; 3])> = None;
    for e in enemies {
        let Some(to) = eye(world, e) else { continue };
        let d = dist3(from, to);
        if best.is_some_and(|(bd, ..)| bd <= d) {
            continue;
        }
        let seen = sight(world, from, to);
        if seen || d < 400.0 {
            best = Some((d, e, seen, to));
        }
    }
    let Some((d, e, seen, to)) = best else { return };
    let (yaw, _) = angles_to(from, to);
    diag::info!(
        Sim,
        "bo2mp botfire t={} bot={n} enemy={e} dist={d:.0} sight={} yawerr={:.1} steer={:.1} look={} allow={} attack={} sprintbtn={} sprintwant={} sprinting={} wstate={} wtime={wtime} clip={} delay_until={} wait={} shot={}",
        l.now,
        u8::from(seen),
        angle_delta(l.view_yaw, yaw).abs(),
        l.steer_err,
        u8::from(l.looking),
        u8::from(l.allow),
        u8::from(l.press & buttons::ATTACK != 0),
        u8::from(l.press & buttons::SPRINT != 0),
        u8::from(l.sprint_wanted),
        u8::from(l.sprinting),
        l.wstate,
        clip.unwrap_or(-1),
        l.delay_until,
        (l.delay_until - l.now).max(0),
        u8::from(shot),
    );
}

/// A clear line: nothing in the map in the way, static models (fences,
/// cars, crates) included, as BO2's sight trace sees them.
fn sight(world: &mut World, from: [f32; 3], to: [f32; 3]) -> bool {
    let t = frame(world).trace_static_world_models(from, to, crate::bullet_collision::MASK_SHOT);
    t.fraction >= 1.0
}

/// bo2mp: can a bot see from `from` to `to` (BO2's `Bot_SightCheck`):
/// nothing solid between, and the line not hidden by smoke.
fn bot_sees(world: &mut World, from: [f32; 3], to: [f32; 3]) -> bool {
    super::natives_fx::fx_visibility(world, from, to) >= 0.2 && sight(world, from, to)
}

/// Players on other teams (or every other player without teams), in play.
/// Is this a team game (`level.teambased`)? Free-for-all players all
/// stand on one team name and are each other's enemies.
fn teambased(vm: &mut Vm<World>) -> bool {
    let f = vm.intern("teambased");
    gsc_t6::truthy(&vm.raw_field(vm.level, f))
}

pub(crate) fn enemies_of(vm: &mut Vm<World>, world: &mut World, n: u32, alive_only: bool) -> Vec<u32> {
    let mine = team_of(vm, world, n);
    let teambased = teambased(vm) && mine != "free" && !mine.is_empty();
    let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for id in ids {
        if id == n {
            continue;
        }
        let theirs = team_of(vm, world, id);
        if theirs == "spectator" || theirs.is_empty() {
            continue;
        }
        if teambased && theirs == mine {
            continue;
        }
        if alive_only && !alive(world, id) {
            continue;
        }
        out.push(id);
    }
    out
}

fn friends_of(vm: &mut Vm<World>, world: &mut World, n: u32, alive_only: bool) -> Vec<u32> {
    if !teambased(vm) {
        return Vec::new();
    }
    let mine = team_of(vm, world, n);
    let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().collect();
    let mut out = Vec::new();
    for id in ids {
        if id == n || team_of(vm, world, id) != mine {
            continue;
        }
        if alive_only && !alive(world, id) {
            continue;
        }
        out.push(id);
    }
    out
}

fn objs(world: &World, ids: &[u32]) -> Vec<Value> {
    let zm = world.resource::<Zm>();
    ids.iter()
        .filter_map(|id| zm.players.get(id).map(|p| Value::Object(p.obj)))
        .collect()
}

/// A target's place: a point, an entity, a path node (struct origin).
fn place(vm: &mut Vm<World>, world: &mut World, v: &Value) -> Option<[f32; 3]> {
    if let Some(p) = v.as_vec3() {
        return Some(p);
    }
    if let Some(p) = origin_of(vm, world, v) {
        return Some(p);
    }
    if let Value::Object(o) = v {
        let f = vm.intern("origin");
        return vm.raw_field(*o, f).as_vec3();
    }
    None
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
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(18);
    let mut used: Vec<u32> = frame(world)
        .client_ids_sorted()
        .into_iter()
        .map(|c| c.0)
        .collect();
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
    // A bot joins: its player entity at once (the script marks it a bot and
    // starts its spawn thread before it connects), its client next frames.
    f!("addtestclient", |vm, world, _, _| {
        // bo2mp mod menu "Kick All Bots": no more bots join this match
        // (BO2's bot_local_think would refill them in 3 s).
        let held = vm.intern("menu_nobots");
        if gsc_t6::truthy(&vm.raw_field(vm.level, held)) {
            return Ok(Value::Undefined);
        }
        let Some(n) = free_client(vm, world) else {
            return Ok(Value::Undefined);
        };
        let obj = vm.alloc_object(ObjKind::Entity(n));
        let pers = vm.intern("pers");
        vm.set_raw_field(obj, pers, Value::array(Array::new()));
        let mut zm = world.resource_mut::<Zm>();
        let mut player = Player::new(obj);
        player.test_client = true;
        player.pending_since = Some(zm.now_ms);
        zm.players.insert(n, player);
        zm.bots.insert(n, Bot::default());
        zm.test_client_requests.push(n);
        diag::info!(Sim, "bo2mp bot: test client {n} joins");
        Ok(Value::Object(obj))
    });
    m!("istestclient", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(Value::bool(
            world
                .resource::<Zm>()
                .players
                .get(&n)
                .is_some_and(|p| p.test_client),
        ))
    });
    m!("botleavegame", |vm, world, s, _| {
        if let Some(n) = entnum(vm, s) {
            world.resource_mut::<Zm>().bot_leaves.push(n);
        }
        Ok(Value::Undefined)
    });
    // Goals.
    m!("addgoal", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let Some(pos) = place(vm, world, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let radius = arg(a, 1).as_float().unwrap_or(24.0);
        let priority = arg(a, 2).as_int().unwrap_or(1);
        let name = text(vm, a, 3);
        if let Some(mut zm) = bot_mut(world, n) {
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
        Ok(Value::Undefined)
    });
    m!("cancelgoal", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, a, 0);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().goals.retain(|g| g.name != name);
        }
        Ok(Value::Undefined)
    });
    m!("hasgoal", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, a, 0);
        Ok(Value::bool(world.resource::<Zm>().bots.get(&n).is_some_and(|b| {
            b.goals.iter().any(|g| g.name == name)
        })))
    });
    m!("getgoal", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = text(vm, a, 0);
        Ok(world
            .resource::<Zm>()
            .bots
            .get(&n)
            .and_then(|b| b.goals.iter().find(|g| g.name == name))
            .map_or(Value::Undefined, |g| Value::Vec3(g.pos)))
    });
    // At a goal (named, else the one it walks to): within its radius. No
    // goal at all: where it wants to be.
    m!("atgoal", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let name = arg(a, 0);
        let name = if vm.is_defined(name) {
            Some(vm.to_text(name))
        } else {
            None
        };
        let origin = frame(world).player(ClientId(n)).map(|ps| ps.origin);
        let zm = world.resource::<Zm>();
        let Some(b) = zm.bots.get(&n) else {
            return Ok(Value::Int(0));
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
        Ok(Value::bool(at))
    });
    // BO2's lookahead: the direction (z the rise per unit) and distance to
    // the point its route walk heads for; undefined and 0 with no route or
    // at its goal (its scripts then pick somewhere to look).
    m!("getlookaheaddir", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let la = world.resource::<Zm>().bots.get(&n).and_then(|b| b.la_dir);
        Ok(la.map_or(Value::Undefined, |(d, _)| Value::Vec3(d)))
    });
    m!("getlookaheaddist", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let la = world.resource::<Zm>().bots.get(&n).and_then(|b| b.la_dir);
        Ok(Value::Float(la.map_or(0.0, |(_, d)| d)))
    });
    m!("botsetfailsafenode", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let node = match arg(a, 0) {
            Value::Object(o) => Some(*o),
            _ => None,
        };
        let p = place(vm, world, arg(a, 0));
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().failsafe = p.map(|p| (p, node));
        }
        Ok(Value::Undefined)
    });
    // Looking and buttons.
    m!("lookat", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let p = place(vm, world, arg(a, 0));
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().look = p;
        }
        Ok(Value::Undefined)
    });
    m!("clearlookat", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().look = None;
        }
        Ok(Value::Undefined)
    });
    m!("allowattack", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(a, 0)) && !holds_fire(vm, world, n);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().allow_attack = on;
        }
        Ok(Value::Undefined)
    });
    m!("pressads", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(a, 0));
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().ads = on;
        }
        Ok(Value::Undefined)
    });
    // One pull of the trigger (an argument: held that many ticks).
    m!("pressattackbutton", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let ticks = arg(a, 0).as_int().unwrap_or(1).clamp(1, 40) as u32;
        if holds_fire(vm, world, n) {
            return Ok(Value::Undefined);
        }
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().attack_ticks = ticks;
        }
        Ok(Value::Undefined)
    });
    // Use held for that long (reloads when nothing is there to use).
    m!("pressusebutton", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let secs = arg(a, 0).as_float().unwrap_or(0.1);
        if let Some(mut zm) = bot_mut(world, n) {
            let until = zm.now_ms + (secs * 1000.0) as i64;
            zm.bots.get_mut(&n).unwrap().use_until = until;
        }
        Ok(Value::Undefined)
    });
    m!("pressmelee", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if holds_fire(vm, world, n) {
            return Ok(Value::Undefined);
        }
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().melee = true;
        }
        Ok(Value::Undefined)
    });
    m!("pressdtpbutton", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().dive = true;
        }
        Ok(Value::Undefined)
    });
    m!("setstance", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let stance = match text(vm, a, 0).as_str() {
            "crouch" => 1,
            "prone" => 2,
            _ => 0,
        };
        // BO2: a bot only changes the stance it picks; anyone else's real
        // stance changes at once (this bind replaces the player one in MP).
        if bot_mut(world, n).is_none() {
            let st = text(vm, a, 0);
            if let Some(ps) = frame(world).player_mut(ClientId(n)) {
                super::natives_player::set_stance(ps, &st);
            }
            return Ok(Value::Undefined);
        }
        let at = frame(world).player(ClientId(n)).map(|ps| ps.origin);
        if let Some(mut zm) = bot_mut(world, n) {
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
        Ok(Value::Undefined)
    });
    m!("allowsprint", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let on = gsc_t6::truthy(arg(a, 0));
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().no_sprint = !on;
        }
        // bo2mp: the same switch every player has (the server drops sprint).
        let id = crate::world::ClientId(n);
        let mut f = super::frame(world);
        if f.client_meta(id).is_some() {
            f.client_meta_mut(id).controls.sprint_disabled = !on;
        }
        Ok(Value::Undefined)
    });
    // A grenade at a point (BO2's scripts give the weapon and the point,
    // no look-at): it turns there, throws once it faces it, and keeps
    // facing it until the grenade has left its hand (`drive`). A tactical
    // goes on its own button. True when it takes the throw up (BO2's
    // PlayerCmd_BotThrowGrenade; the scripts branch on it).
    m!("throwgrenade", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if holds_fire(vm, world, n) {
            return Ok(Value::Int(0));
        }
        let p = place(vm, world, arg(a, 1));
        let Some(w) = super::weapon(world, &text(vm, a, 0)).ok().filter(|w| *w != 0) else {
            return Ok(Value::Int(0));
        };
        let (tactical, ammo) = {
            let f = frame(world);
            (
                f.gun_facts().get(w).core.offhand_slot == weapon_t6::facts::offhand_slot::TACTICAL_GRENADE,
                crate::script_player::ammo_clip(&f, ClientId(n), w)
                    + crate::script_player::ammo_stock(&f, ClientId(n), w),
            )
        };
        let Some(mut zm) = bot_mut(world, n) else {
            return Ok(Value::Int(0));
        };
        let now = zm.now_ms;
        let b = zm.bots.get_mut(&n).unwrap();
        if !takes_throw(ammo, b.throw.is_some(), b.throw_wait, now) {
            return Ok(Value::Int(0));
        }
        b.throw_wait = now + THROW_WAIT_MS;
        // It turns from where the view is now (a script may just have set
        // it with setplayerangles).
        b.aim_avg = None;
        let button = if tactical { buttons::SMOKE } else { buttons::FRAG };
        b.throw = Some(Throw { at: p, button, asked: now, pressed: 0, started: false });
        b.tick = 0;
        b.attack_ticks = 0;
        Ok(Value::Int(1))
    });
    // Seeing.
    m!("botsighttracepassed", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let Some(from) = eye(world, n) else {
            return Ok(Value::Int(0));
        };
        let target = arg(a, 0);
        let to = match entnum(vm, target) {
            Some(t) if world.resource::<Zm>().players.contains_key(&t) => eye(world, t),
            _ => place(vm, world, target).map(|p| [p[0], p[1], p[2] + 32.0]),
        };
        Ok(Value::bool(to.is_some_and(|to| bot_sees(world, from, to))))
    });
    // Enemies it can see, nearest first, inside its field of view: the
    // argument is the smallest cosine between its view and the enemy (its
    // scripts' `bot.fov`: 0.0872 = 85 degrees each side on normal; -1 =
    // all round).
    m!("getthreats", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let fov = arg(a, 0).as_float().unwrap_or(-1.0);
        let Some(from) = eye(world, n) else {
            return Ok(list(Vec::new()));
        };
        // bo2mp mod menu: a bot told to `ignoreall` sees nobody, and a
        // player set `ignoreme` (Invisible) is never seen.
        let script_flag = |vm: &mut Vm<World>, world: &World, e: u32, name: &str| {
            let f = vm.intern(name);
            world.resource::<Zm>().players.get(&e).is_some_and(|p| gsc_t6::truthy(&vm.raw_field(p.obj, f)))
        };
        if script_flag(vm, world, n, "ignoreall") {
            return Ok(list(Vec::new()));
        }
        let view = frame(world).player(ClientId(n)).map_or([0.0; 3], |ps| ps.viewangles);
        let mut seen: Vec<(f32, u32)> = Vec::new();
        for e in enemies_of(vm, world, n, true) {
            if script_flag(vm, world, e, "ignoreme") {
                continue;
            }
            let Some(to) = eye(world, e) else { continue };
            if fov > -1.0 {
                let (yaw, pitch) = (view[1].to_radians(), view[0].to_radians());
                let forward = [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin()];
                let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
                let dot = (forward[0] * d[0] + forward[1] * d[1] + forward[2] * d[2]) / len;
                if dot < fov {
                    continue;
                }
            }
            if bot_sees(world, from, to) {
                seen.push((dist2d(from, to), e));
            }
        }
        // Its enemies' vehicles too (BO2's threats are everything that
        // takes damage; its scripts shoot an RC-XD and pass over what
        // needs a launcher).
        let mine = team_of(vm, world, n);
        let tb = teambased(vm) && mine != "free" && !mine.is_empty();
        let me = world.resource::<Zm>().players.get(&n).map(|p| p.obj);
        let cars: Vec<(u32, ObjRef, [f32; 3])> = world
            .resource::<Zm>()
            .ents
            .iter()
            .filter(|(_, e)| e.classname == "script_vehicle" && !e.hidden)
            .filter(|(id, _)| !world.resource::<Zm>().no_bot_target.contains(*id))
            .filter_map(|(id, e)| e.obj.map(|o| (*id, o, e.origin)))
            .collect();
        let mut things: Vec<(f32, Value)> = Vec::new();
        for (id, o, at) in cars {
            if !vm.alive(o) || !super::killstreaks::damage_target(world, id) {
                continue;
            }
            let enemy = if tb {
                let f = vm.intern("team");
                let t = vm.raw_field(o, f);
                let t = vm.to_text(&t);
                !t.is_empty() && t != mine
            } else {
                let f = vm.intern("owner");
                !matches!((vm.raw_field(o, f), me), (Value::Object(a), Some(b)) if a == b)
            };
            if !enemy {
                continue;
            }
            let to = [at[0], at[1], at[2] + centroid_height(world, id)];
            if fov > -1.0 {
                let (yaw, pitch) = (view[1].to_radians(), view[0].to_radians());
                let forward = [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin()];
                let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
                if (forward[0] * d[0] + forward[1] * d[1] + forward[2] * d[2]) / len < fov {
                    continue;
                }
            }
            if bot_sees(world, from, to) {
                things.push((dist2d(from, to), Value::Object(o)));
            }
        }
        let zm = world.resource::<Zm>();
        let mut all: Vec<(f32, Value)> = seen
            .iter()
            .filter_map(|(d, e)| zm.players.get(e).map(|p| (*d, Value::Object(p.obj))))
            .chain(things)
            .collect();
        all.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(list(all.into_iter().map(|(_, v)| v).collect()))
    });
    m!("getenemies", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let alive_only = gsc_t6::truthy(arg(a, 0));
        let ids = enemies_of(vm, world, n, alive_only);
        Ok(list(objs(world, &ids)))
    });
    m!("getfriendlies", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let alive_only = gsc_t6::truthy(arg(a, 0));
        let ids = friends_of(vm, world, n, alive_only);
        Ok(list(objs(world, &ids)))
    });
    // Where a player will be in that many frames at his speed.
    m!("predictposition", |vm, world, _, a| {
        let Some(t) = entnum(vm, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let frames = arg(a, 1).as_float().unwrap_or(1.0);
        if let Some(ps) = frame(world).player(ClientId(t)) {
            return Ok(Value::Vec3(std::array::from_fn(|i| {
                ps.origin[i] + ps.velocity[i] * frames * 0.05
            })));
        }
        // A vehicle: where it is going at its speed.
        let Some(at) = world.resource::<Zm>().ents.get(&t).map(|e| e.origin) else {
            return Ok(Value::Undefined);
        };
        let v = super::killstreaks::driven_velocity(world, t).unwrap_or([0.0; 3]);
        Ok(Value::Vec3(std::array::from_fn(|i| at[i] + v[i] * frames * 0.05)))
    });
    // A player's eye height over his feet (where bots aim: torso = feet +
    // height / 1.6).
    m!("getplayerviewheight", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(Value::Float(
            frame(world)
                .player(ClientId(n))
                .map_or(60.0, |ps| ps.view_height_current),
        ))
    });
    // An entity's middle: a player's chest, else 32 over its origin.
    m!("getcentroid", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        if let Some(ps) = frame(world).player(ClientId(n)) {
            let o = ps.origin;
            return Ok(Value::Vec3([o[0], o[1], o[2] + ps.view_height_current * 0.6]));
        }
        let h = centroid_height(world, n);
        Ok(origin_of(vm, world, s).map_or(Value::Undefined, |o| Value::Vec3([o[0], o[1], o[2] + h])))
    });
    f!("weaponisdualwield", |vm, _, _, a| {
        let w = text(vm, a, 0);
        Ok(Value::bool(w.contains("+dw") || w.ends_with("dw_mp")))
    });
    // A scoped weapon (BO2: it has an overlay reticle): its bots hold
    // full sights a second before the shot.
    f!("isweaponscopeoverlay", |vm, world, _, a| {
        let w = super::weapon(world, &text(vm, a, 0)).ok();
        let facts = w.and_then(|w| frame(world).combat_facts_for(w));
        Ok(Value::bool(facts.is_some_and(|f| f.overlay_reticle != 0)))
    });
    f!("target_istarget", |_, _, _, _| Ok(Value::Int(0)));
    m!("getplayercamerapos", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(eye(world, n).map_or(Value::Undefined, Value::Vec3))
    });
    // A bot's own classes (its scripts build five from his item table).
    m!("botsetdefaultclass", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(a, 0).as_int().unwrap_or(0);
        let name = text(vm, a, 1);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().classes.entry(class).or_default().default = Some(name);
        }
        Ok(Value::Undefined)
    });
    m!("botclassadditem", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(a, 0).as_int().unwrap_or(0);
        let item = text(vm, a, 1);
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots.get_mut(&n).unwrap().classes.entry(class).or_default().items.push(item);
        }
        Ok(Value::Undefined)
    });
    m!("botclassaddattachment", |vm, world, s, a| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        let class = arg(a, 0).as_int().unwrap_or(0);
        let (weapon, attachment, slot) = (text(vm, a, 1), text(vm, a, 2), text(vm, a, 3));
        if let Some(mut zm) = bot_mut(world, n) {
            zm.bots
                .get_mut(&n)
                .unwrap()
                .classes
                .entry(class)
                .or_default()
                .attachments
                .push((weapon, attachment, slot));
        }
        Ok(Value::Undefined)
    });
    m!("botclasssetweaponoption", |_, _, _, _| Ok(Value::Undefined));
    // The player the bots count teams around: the local player (client 0).
    f!("gethostplayerforbots", |_, world, _, _| Ok(world
        .resource::<Zm>()
        .players
        .get(&0)
        .map_or(Value::Undefined, |p| Value::Object(p.obj))));
    m!("ishostforbots", |vm, _, s, _| Ok(Value::bool(entnum(vm, s) == Some(0))));
    // The map's path nodes as the bot scripts ask about them.
    fn node_obj(world: &World, i: u32) -> Value {
        world
            .resource::<Zm>()
            .node_objs
            .get(i as usize)
            .map_or(Value::Undefined, |o| Value::Object(*o))
    }
    fn node_index(vm: &mut Vm<World>, world: &World, v: &Value) -> Option<u32> {
        let Value::Object(o) = v else { return None };
        world
            .resource::<Zm>()
            .node_of
            .get(o)
            .copied()
            .or_else(|| {
                let f = vm.intern("origin");
                let p = vm.raw_field(*o, f).as_vec3()?;
                world.resource::<Zm>().nav.nearest(p)
            })
    }
    fn node_eye(world: &World, i: u32) -> [f32; 3] {
        let o = world.resource::<Zm>().nav.nodes[i as usize].origin;
        [o[0], o[1], o[2] + 32.0]
    }
    f!("getnearestnode", |vm, world, _, a| {
        let Some(p) = place(vm, world, arg(a, 0)) else {
            return Ok(Value::Undefined);
        };
        let i = world.resource::<Zm>().nav.nearest(p);
        Ok(i.map_or(Value::Undefined, |i| node_obj(world, i)))
    });
    // Two nodes see each other: BO2 reads the map compiler's sight table
    // (`Path_NodesVisible`); a node past it (or a map without one) is traced.
    fn nodes_visible(world: &mut World, i: u32, j: u32) -> bool {
        if let Some(seen) = world.resource::<Zm>().nav.baked.visible(i, j) {
            return seen;
        }
        let (x, y) = (node_eye(world, i), node_eye(world, j));
        sight(world, x, y)
    }
    // BO2's `Scr_GetVisibleNode`: the route from `from` to `to`, and on it,
    // from `from` on, the first node that sees the route's last; none
    // without a route. No distance limit (the bot's turret spot wants one
    // 512 or more from it, toward the enemy).
    f!("getvisiblenode", |vm, world, _, a| {
        let (Some(from), Some(to)) = (place(vm, world, arg(a, 0)), place(vm, world, arg(a, 1)))
        else {
            return Ok(Value::Undefined);
        };
        let route = match (nearest_reachable(world, from), nearest_reachable(world, to)) {
            (Some(x), Some(y)) => world.resource::<Zm>().nav.path(x, y),
            _ => None,
        };
        let Some(&last) = route.as_ref().and_then(|r| r.last()) else {
            return Ok(Value::Undefined);
        };
        let route = route.unwrap_or_default();
        Ok(route
            .into_iter()
            .find(|&i| i == last || nodes_visible(world, i, last))
            .map_or(Value::Undefined, |i| node_obj(world, i)))
    });
    // The nodes a node sees: its linked neighbours that it can see.
    f!("getvisiblenodes", |vm, world, _, a| {
        let Some(i) = node_index(vm, world, arg(a, 0)) else {
            return Ok(list(Vec::new()));
        };
        let links: Vec<u32> = world.resource::<Zm>().nav.nodes[i as usize]
            .links
            .iter()
            .map(|l| u32::from(l.0))
            .collect();
        let from = node_eye(world, i);
        let mut out = Vec::new();
        for j in links {
            if sight(world, from, node_eye(world, j)) {
                out.push(node_obj(world, j));
            }
        }
        Ok(list(out))
    });
    f!("nodesvisible", |vm, world, _, a| {
        let (Some(i), Some(j)) = (node_index(vm, world, arg(a, 0)), node_index(vm, world, arg(a, 1)))
        else {
            return Ok(Value::Int(0));
        };
        Ok(Value::bool(nodes_visible(world, i, j)))
    });
    // BO2's `Scr_NodesCanPath`: a straight walk joins them (the map
    // compiler's smooth table) or the first links to the second; not a
    // search for a route.
    f!("nodescanpath", |vm, world, _, a| {
        let (Some(i), Some(j)) = (node_index(vm, world, arg(a, 0)), node_index(vm, world, arg(a, 1)))
        else {
            return Ok(Value::Int(0));
        };
        let nav = &world.resource::<Zm>().nav;
        let linked = nav.nodes[i as usize].links.iter().any(|l| u32::from(l.0) == j);
        Ok(Value::bool(nav.baked.smooth(i, j) || linked))
    });
    // BO2's `Scr_FindPath`: whether a route joins two points, true or
    // false (the route itself is not handed back; every BO2 script only
    // asks yes or no).
    f!("findpath", |vm, world, _, a| {
        let (Some(from), Some(to)) = (place(vm, world, arg(a, 0)), place(vm, world, arg(a, 1)))
        else {
            return Ok(Value::Int(0));
        };
        let route = match (nearest_reachable(world, from), nearest_reachable(world, to)) {
            (Some(x), Some(y)) => world.resource::<Zm>().nav.path(x, y),
            _ => None,
        };
        Ok(Value::bool(route.is_some()))
    });
    f!("canclaimnode", |_, _, _, _| Ok(Value::Int(1)));
    // BO2's danger marks: a count per team on a node (0..15), raised and
    // lowered by `setdangerous(team, on)`, dangerous while above 0 (bots
    // keep clear of a killstreak's landing spot).
    m!("setdangerous", |vm, world, s, a| {
        let (Some(i), Some(t)) = (node_index(vm, world, s), danger_slot(&text(vm, a, 0))) else {
            return Ok(Value::Undefined);
        };
        let on = arg(a, 1).as_int().unwrap_or(0) != 0;
        let mut zm = world.resource_mut::<Zm>();
        let c = &mut zm.node_danger.entry(i).or_default()[t];
        *c = if on { (*c + 1).min(15) } else { c.saturating_sub(1) };
        Ok(Value::Undefined)
    });
    m!("isdangerous", |vm, world, s, a| {
        let (Some(i), Some(t)) = (node_index(vm, world, s), danger_slot(&text(vm, a, 0))) else {
            return Ok(Value::Int(0));
        };
        Ok(Value::bool(world.resource::<Zm>().node_danger.get(&i).is_some_and(|c| c[t] > 0)))
    });
    f!("claimnode", |_, _, _, _| Ok(Value::Undefined));
    f!("releaseclaimednode", |_, _, _, _| Ok(Value::Undefined));
    // Bots have every item (their scripts ask before building a class).
    m!("isitemlocked", |_, _, _, _| Ok(Value::Int(0)));
    // What a player is doing, from his state.
    m!("isreloading", |vm, world, s, _| {
        let n = entnum(vm, s).unwrap_or(u32::MAX);
        Ok(Value::bool(frame(world).player(ClientId(n)).is_some_and(|ps| {
            reloading(ps.weapon_state_t6)
        })))
    });
    // As BO2 reads them: switching = the weapon raising or dropping,
    // throwing = an offhand (grenade) state, mantling and on a ladder =
    // the move flags.
    m!("isswitchingweapons", |vm, world, s, _| {
        Ok(player_is(vm, world, s, |ps| {
            // TODO(fact): the swim in and out count, as our old MW2 mirror read them.
            let s = ps.weapon_state_t6;
            weapon_t6::state::is_raising_or_swim_out(s) || weapon_t6::state::is_dropping_or_swim_in(s)
        }))
    });
    m!("isthrowinggrenade", |vm, world, s, _| {
        Ok(player_is(vm, world, s, |ps| {
            weapon_t6::state::is_offhand(ps.weapon_state_t6)
                && ps.weapon_state_t6 != weapon_t6::state::OFFHAND_DETONATE
        }))
    });
    m!("ismantling", |vm, world, s, _| {
        Ok(player_is(vm, world, s, |ps| ps.pm_flags & playerstate_t6::game::pm_flags::MANTLE != 0))
    });
    m!("isonladder", |vm, world, s, _| {
        Ok(player_is(vm, world, s, |ps| ps.pm_flags & playerstate_t6::game::pm_flags::LADDER != 0))
    });
    for name in ["isfiring", "isremotecontrolling", "iscarryingturret"] {
        vm.bind(name, true, |_, _, _, _| Ok(Value::Int(0)));
    }
}

/// Each tick: the bots' commands (route, view, buttons) replace their
/// clients' own; a bot whose client never came is dropped.
pub(super) fn drive(world: &mut World, now: i64) {
    let ids: Vec<u32> = world.resource::<Zm>().bots.keys().copied().collect();
    if ids.is_empty() {
        return;
    }
    log_negotiations(world);
    let dv = BotDvars::read(world);
    let crowd = crowd(world);
    let held = super::console_bots::frozen(world);
    for n in ids {
        if !alive(world, n) {
            // Dead, its steering starts over (BO2 clears its bot state).
            if let Some(b) = world.resource_mut::<Zm>().bots.get_mut(&n) {
                b.throw = None;
                // A new life picks from what it spawns holding.
                b.select = 0;
                b.select_held = 0;
                // Its route goes too: a new life plans from where it spawns
                // (one kept from the last life is trimmed as walked past and
                // aims it at the far goal through walls).
                b.path.clear();
                b.negs.clear();
                b.neg = None;
                b.neg_began = false;
                b.path_to = None;
                b.replan_from = None;
                b.replan_at = 0;
                b.prev = None;
                b.la_amount = 0.0;
                b.la_pos = None;
                b.la_dir = None;
                b.la_clear = false;
                if b.aim_avg.is_some() {
                    b.aim_avg = None;
                    b.turn_speed = [0.0; 2];
                    b.sprint_since = None;
                    b.fire_delay_until = 0;
                }
            }
            continue;
        }
        let Some((origin, view, height, ladder, mantling, low_climb, sprinting, ads_frac, wstate, standing, weapon, on_ground, ledge)) = frame(world)
            .player(ClientId(n))
            .map(|ps| {
                use playerstate_t6::game::pm_flags;
                (
                    ps.origin,
                    ps.viewangles,
                    ps.view_height_current,
                    ps.pm_flags & pm_flags::LADDER != 0,
                    ps.pm_flags & pm_flags::MANTLE != 0,
                    // A climb with no room to stand at the ledge (BO2's mantle flag 4).
                    ps.pm_flags & pm_flags::MANTLE != 0 && ps.mantle_state.flags & movement_t6::MANTLE_FORCE_CROUCH != 0,
                    ps.pm_flags & pm_flags::SPRINTING != 0,
                    ps.f_weapon_pos_frac.clamp(0.0, 1.0),
                    ps.weapon_state_t6,
                    ps.pm_flags & (pm_flags::CROUCH | pm_flags::PRONE) == 0,
                    ps.weapon,
                    ps.ground_entity_num != playerstate_t6::game::ENTITYNUM_NONE,
                    // The mantle hint: a ledge in reach (BO2's mantle flag 0x10).
                    ps.mantle_state.flags & playerstate_t6::game::mantle_flags::ACTIVE != 0,
                )
            })
        else {
            continue;
        };
        let select = world.resource_mut::<Zm>().bots.get_mut(&n).map_or(0, |b| {
            follow_select(&mut b.select, &mut b.select_held, weapon);
            b.select
        });
        // Full-automatic weapons (miniguns too) hold the trigger down.
        let full_auto = frame(world)
            .combat_facts_for(weapon)
            .is_none_or(|f| f.fire_type == 0);
        // Taking a negotiation link (BO2's Bot_AtNegotiationEnd): once it
        // has been at the begin, done once its mantle finishes, once it
        // stands within 15 units of the link's end, or (a drop) once it
        // leaves the ground; then it plans on at once, from the link's end
        // node (a route from the node nearest it, on top of a thin wall it
        // climbed, led back over the same link). Ours also gives up
        // after 4 s (a ladder 8 s) or when the route is gone. A ladder link
        // is done once it stands at the top, off the ladder: a route planned
        // afresh from partway up starts at the ladder's foot again, so
        // leaving the ground there would start the climb over and over.
        {
            let mut zm = world.resource_mut::<Zm>();
            let b = zm.bots.get_mut(&n).unwrap();
            let was_mantling = std::mem::replace(&mut b.was_mantling, mantling);
            if let Some((l, since)) = b.neg {
                let near_end = on_ground && !mantling && dist3(origin, l.end) < 15.0;
                let up_top = l.flags & NEG_LADDER != 0 && on_ground && !ladder && origin[2] > l.end[2] - 16.0;
                let landed = b.neg_began
                    && ((was_mantling && !mantling) || near_end || up_top || (l.flags & NEG_DROP != 0 && !on_ground));
                let limit = if l.flags & NEG_LADDER != 0 { LADDER_GIVE_UP_MS } else { 4000 };
                let gave_up = now - since > limit || b.path.is_empty();
                if landed || gave_up {
                    let began = std::mem::take(&mut b.neg_began);
                    b.neg = None;
                    b.path.clear();
                    b.path_to = None;
                    b.replan_at = now;
                    b.replan_from = landed.then_some(l.end_node);
                    if botlog() {
                        diag::info!(
                            Sim,
                            "bo2mp bot {n} negotiation {} flags {:#x} at {:.0} {:.0} {:.0} begin {:.0} {:.0} {:.0} end {:.0} {:.0} {:.0} after {} ms{}",
                            if landed { "done" } else { "gave up" },
                            l.flags,
                            origin[0],
                            origin[1],
                            origin[2],
                            l.begin[0],
                            l.begin[1],
                            l.begin[2],
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
            let zm = world.resource::<Zm>();
            let b = &zm.bots[&n];
            (b.best_goal().cloned(), b.path_to, b.replan_at)
        };
        // On a ladder link's ladder the climb finishes first (a route
        // planned from partway up starts at its foot): its route stays.
        let climbing = ladder
            && world.resource::<Zm>().bots[&n]
                .neg
                .is_some_and(|(l, _)| l.flags & NEG_LADDER != 0);
        if climbing {
        } else if let Some(g) = &goal {
            let moved = path_to.is_none_or(|p| {
                let d = [p[0] - g.pos[0], p[1] - g.pos[1], p[2] - g.pos[2]];
                g.radius * g.radius <= d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
            });
            let far = dist2d(origin, g.pos) > g.radius;
            let empty = world.resource::<Zm>().bots[&n].path.is_empty();
            if (moved || (empty && far)) && now >= replan_at {
                let start = world.resource_mut::<Zm>().bots.get_mut(&n).unwrap().replan_from.take();
                let (route, negs) = plan(world, origin, start, g.pos);
                let mut zm = world.resource_mut::<Zm>();
                let b = zm.bots.get_mut(&n).unwrap();
                b.path = route;
                b.negs = negs;
                b.neg = None;
                b.neg_began = false;
                b.path_to = Some(g.pos);
                b.replan_at = now + 500;
                b.prev = Some(origin);
                b.path_gen = b.path_gen.wrapping_add(1);
                b.path_new = true;
                b.la_amount = 0.0;
                b.la_pos = None;
                b.la_dir = None;
                b.la_clear = false;
                // BO2MP_BOTNEGLOG (test aid): each new route's first points.
                if botneglog() {
                    let first: Vec<[i32; 3]> = b.path.iter().take(4).map(|p| [p[0] as i32, p[1] as i32, p[2] as i32]).collect();
                    diag::info!(
                        Sim,
                        "bo2mp bot {n} route {} at {:.0} {:.0} {:.0} from node {start:?}: {} points, first {first:?}",
                        b.path_gen, origin[0], origin[1], origin[2], b.path.len()
                    );
                }
            }
        } else {
            let mut zm = world.resource_mut::<Zm>();
            let b = zm.bots.get_mut(&n).unwrap();
            b.path.clear();
            b.path_to = None;
            b.replan_from = None;
        }
        // Something at step height between it and the next route point
        // (a ledge, a railing, a low wall), as opposed to stairs or a slope
        // the walk steps up by itself: only then does a jump (and the
        // mantle it starts) help; jumping on stairs under a ceiling holds
        // a bot in place.
        let climb_blocked: Option<([f32; 3], bool)> = {
            let peek = {
                let zm = world.resource::<Zm>();
                let path = &zm.bots[&n].path;
                path.iter()
                    .find(|q| dist2d(origin, **q) >= 24.0)
                    .or(path.last())
                    .copied()
            };
            peek.map(|p| (p, dist2d(origin, p) < 160.0 && blocked_ahead(world, origin, p)))
        };
        // Blocked at step height toward the next point: going up (a ledge)
        // it jumps at once; level or down (a railing it drops over, a low
        // fence) it jumps once it makes no headway.
        let ahead_blocked = climb_blocked.is_some_and(|(_, b)| b);
        let climb_blocked = climb_blocked
            .is_some_and(|(p, b)| b && p[2] - origin[2] > 18.0 && dist2d(origin, p) < 96.0);
        // BO2MP_BOTLOG (test aid): is it standing inside the map or a prop,
        // or against another player?
        let (in_solid, touching) = if botlog() {
            use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
            let t = frame(world).trace_clip(origin, origin, PLAYER_MINS, PLAYER_MAXS, MASK_PLAYER_SOLID);
            let ids: Vec<u32> = world.resource::<Zm>().players.keys().copied().filter(|&id| id != n).collect();
            let touching = ids.into_iter().find(|&id| {
                frame(world).player(ClientId(id)).is_some_and(|ps| {
                    dist2d(ps.origin, origin) < 32.0 && (ps.origin[2] - origin[2]).abs() < 72.0
                })
            });
            (t.startsolid, touching)
        } else {
            (false, None)
        };
        let at_goal = goal
            .as_ref()
            .is_none_or(|g| dist2d(origin, g.pos) <= g.radius.max(16.0));
        // Past a route point once beyond the line across it (the walk cuts
        // corners toward its lookahead, so it seldom touches one), or (BO2's
        // Path_TrimCompletedPath) once it lies behind the bot along its last
        // lookahead, when that walk was clear or the amount has shrunk to
        // half: a nearest node behind it is no walk back.
        let (route, seen_key, seen) = {
            let mut zm = world.resource_mut::<Zm>();
            let b = zm.bots.get_mut(&n).unwrap();
            let trim = b
                .la_dir
                .filter(|_| b.la_clear || (b.la_amount > 0.0 && b.la_amount <= 16384.0))
                .map(|(d, _)| d);
            // A link's begin stays until the link is done (BO2's trim never
            // passes the negotiation start).
            let had = b.path.len();
            let mut why = Vec::new();
            while b.path.len() > 1 {
                let p = b.path[0];
                if b.negs.iter().any(|l| l.begin == p) {
                    break;
                }
                let from = b.prev.unwrap_or(origin);
                let d = [p[0] - from[0], p[1] - from[1]];
                let beyond = d[0] * d[0] + d[1] * d[1] > 1.0
                    && (origin[0] - p[0]) * d[0] + (origin[1] - p[1]) * d[1] >= 0.0;
                let behind = trim.is_some_and(|t| t[0] * (p[0] - origin[0]) + t[1] * (p[1] - origin[1]) <= 0.0);
                if !(beyond || behind || dist2d(origin, p) < 24.0) {
                    break;
                }
                why.push(if beyond { 'b' } else if behind { 'h' } else { 'n' });
                b.prev = Some(p);
                b.path.remove(0);
            }
            // BO2MP_BOTNEGLOG (test aid): a route trimmed down to its goal.
            if had > 1 && b.path.len() == 1 && botneglog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} route down to the goal at {:.0} {:.0} {:.0}: dropped {} ({}) la {:?} clear {} amount {:.0}",
                    origin[0], origin[1], origin[2], had - 1, why.iter().collect::<String>(),
                    b.la_dir.map(|(d, l)| [d[0], d[1], l]), b.la_clear, b.la_amount
                );
            }
            // BO2MP_BOTNEGLOG (test aid): two or more points trimmed at once.
            if why.len() >= 2 && botneglog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} trimmed {} ({}) at {:.0} {:.0} {:.0} route {} next {:?}",
                    why.len(), why.iter().collect::<String>(), origin[0], origin[1], origin[2], b.path_gen,
                    b.path.first().map(|p| [p[0] as i32, p[1] as i32, p[2] as i32])
                );
            }
            let near = b.path.get(1).is_some_and(|p| dist2d(origin, *p) < 192.0);
            let key = (b.path_gen, b.path.len(), near);
            let seen = b.seen.filter(|s| (s.0, s.1, s.2) == key).map(|s| s.3);
            ((!at_goal && !b.path.is_empty()).then(|| b.path.clone()), key, seen)
        };
        // The link its route crosses next, when it heads for the link's
        // begin now (BO2: the lookahead's next node is the begin), and the
        // one it is taking: the walk goes no farther than the begin until
        // the link is done. It is at the begin (BO2's Bot_AtNegotiationBegin)
        // while no farther from the end than the begin is; it starts taking
        // the link there (ours: or within 20 units of the begin).
        let (neg_ahead, link) = {
            let zm = world.resource::<Zm>();
            let b = &zm.bots[&n];
            match (&route, b.neg) {
                (Some(r), None) => {
                    let l = b.negs.iter().find(|l| l.begin == r[0]).copied();
                    (l, l)
                }
                (Some(r), Some((l, _))) if l.begin == r[0] => (None, Some(l)),
                _ => (None, None),
            }
        };
        let bo2_at_begin = link.is_some_and(|l| dist3(origin, l.end) < dist3(l.begin, l.end) + 0.5);
        let at_begin = bo2_at_begin || neg_ahead.is_some_and(|l| dist3(origin, l.begin) < 20.0);
        let route = match (route, link) {
            (Some(r), Some(_)) => Some(r[..2.min(r.len())].to_vec()),
            (r, _) => r,
        };
        let far_seen = match (&route, seen) {
            (Some(_), Some(p)) => Some(p),
            (Some(r), None) => {
                let p = farthest_seen(world, origin, r);
                if let Some(p) = p {
                    let mut zm = world.resource_mut::<Zm>();
                    zm.bots.get_mut(&n).unwrap().seen = Some((seen_key.0, seen_key.1, seen_key.2, p));
                }
                p
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
        let amount = world.resource::<Zm>().bots[&n].la_amount;
        let la_route = route.as_deref().map(|r| if link.is_some() { &r[..1] } else { r });
        let la = match la_route.and_then(|r| lookahead(origin, r, amount)) {
            Some((p, reach)) => {
                let clear = walk_clear(world, origin, p);
                let mut zm = world.resource_mut::<Zm>();
                let b = zm.bots.get_mut(&n).unwrap();
                let amount = if b.la_amount > 0.0 { b.la_amount } else { 32768.0 };
                let was_clear = std::mem::replace(&mut b.la_clear, clear);
                if clear {
                    b.la_amount = (amount * 1.176_470_5 + 6.4).min(65536.0);
                    b.la_pos = Some(p);
                    Some((p, reach))
                } else {
                    b.la_amount = (amount * 0.8).max(0.001);
                    match b.la_pos.filter(|q| was_clear && dist2d(*q, origin) > 1.0) {
                        Some(q) => Some((q, dist2d(origin, q))),
                        None => Some((p, reach)),
                    }
                }
            }
            None => None,
        };
        let next = la.map(|(p, _)| p);
        {
            let mut zm = world.resource_mut::<Zm>();
            zm.bots.get_mut(&n).unwrap().la_dir = la.and_then(|(p, _)| {
                let d = [p[0] - origin[0], p[1] - origin[1]];
                let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
                (len > 1e-3).then(|| ([d[0] / len, d[1] / len, (p[2] - origin[2]) / len], len))
            });
        }
        // Avoidance (Bot_UpdateAvoidance): off ladders and ledges, a bot on
        // a team bends its walk around teammates close by, keeping the
        // lookahead's distance; a bend into a clip brush right in front of
        // it holds it still for the tick.
        let mut walk_to = next;
        let mut hold = false;
        if let Some((lp, _)) = la
            && !ladder
            && !mantling
            && let Some(me) = crowd.iter().find(|a| a.id == n && !a.team.is_empty() && a.team != "free")
        {
            let d = [lp[0] - origin[0], lp[1] - origin[1]];
            let dist = dot2(d, d).sqrt();
            let speed = me.vel[0].hypot(me.vel[1]);
            let range = (speed + BOT_AVOID_SPEED) * 0.5 * AVOID_HORIZON;
            let others: Vec<&Avoider> = crowd
                .iter()
                .filter(|a| {
                    a.id != n
                        && a.team == me.team
                        && dist2d(a.pos, origin) < range
                        && (a.pos[2] - origin[2]).abs() < 90.0
                })
                .take(32)
                .collect();
            if dist > 1.0 && !others.is_empty() {
                let pref = [d[0] / dist * speed, d[1] / dist * speed];
                let v = avoid_velocity(me, pref, BOT_AVOID_SPEED, &others);
                let vl = v[0].hypot(v[1]);
                if vl > 0.01 {
                    let bent = [origin[0] + v[0] / vl * dist, origin[1] + v[1] / vl * dist, lp[2]];
                    hold = dist2d(bent, lp) > 15.0 && !avoid_clear(world, origin, bent);
                    walk_to = Some(bent);
                }
            }
        }
        let mut zm = world.resource_mut::<Zm>();
        let b = zm.bots.get_mut(&n).unwrap();
        b.tick = b.tick.wrapping_add(1);
        // The view (BO2's Bot_Think / Bot_UpdateLookahead): the angles it
        // chases move toward what it looks at (an eighth of the way in
        // pitch, a quarter in yaw each tick), else toward the farthest route
        // point it can see, slower the farther its lookahead reaches; the
        // view then steers toward them. A grenade it is about to throw (or
        // throwing) comes before what it looks at.
        let eye = [origin[0], origin[1], origin[2] + height];
        let look = b.throw.and_then(|t| t.at).or(b.look);
        // A ladder link: from its begin it faces the ladder (toward the
        // link's end) and looks up it, so walking on grabs the ladder and
        // the climb goes up; while on it nothing else turns it.
        let ladder_link = link
            .filter(|l| l.flags & NEG_LADDER != 0 && (b.neg.is_some() || at_begin))
            .filter(|_| ladder || look.is_none());
        let desired: Option<[f32; 2]> = if let Some(l) = ladder_link {
            let (yaw, _) = angles_to(eye, l.end);
            let pitch = LADDER_LOOK_PITCH;
            let avg = b.aim_avg.unwrap_or([view[0], view[1]]);
            b.aim_avg = Some([avg[0] + angle_delta(avg[0], pitch) / 2.0, avg[1] + angle_delta(avg[1], yaw)]);
            Some([pitch, yaw])
        } else if let Some(l) = look {
            let (yaw, pitch) = angles_to(eye, l);
            let avg = b.aim_avg.unwrap_or([view[0], view[1]]);
            b.aim_avg = Some([avg[0] + angle_delta(avg[0], pitch) / 8.0, avg[1] + angle_delta(avg[1], yaw) / 4.0]);
            Some([pitch, yaw])
        } else if let Some((lp, reach)) = la {
            // Ours: at a link's begin, and while taking it, it faces the
            // link's end. Its route stops at the begin until the link is
            // done, so it faced the begin it stood on (any way at all) or,
            // a step past it, back the way it came; BO2's mantle check finds
            // only a ledge within 60 degrees of the view.
            // BO2 faces a jump link's end from the eye, any other from the
            // origin.
            let across = link.filter(|_| at_begin || b.neg.is_some());
            let t = across.map(|l| l.end).or(far_seen.filter(|p| dist2d(origin, *p) >= 1.0)).unwrap_or(lp);
            let from = if across.is_some_and(|l| l.flags & NEG_JUMP != 0) { eye } else { origin };
            if dist2d(origin, t) >= 1.0 {
                let (yaw, pitch) = angles_to(from, t);
                let h = if b.path_new || reach < 64.0 {
                    1.0
                } else if reach < 128.0 {
                    2.0
                } else {
                    4.0
                };
                let avg = b.aim_avg.unwrap_or([view[0], view[1]]);
                b.aim_avg = Some([avg[0] + angle_delta(avg[0], pitch) / (2.0 * h), avg[1] + angle_delta(avg[1], yaw) / h]);
                Some([pitch, yaw])
            } else {
                None
            }
        } else {
            b.aim_avg = None;
            None
        };
        b.path_new = false;
        let mut want = view;
        match (desired, b.aim_avg) {
            (Some(_), Some(avg)) => {
                let out = steer([view[0], view[1]], avg, &mut b.turn_speed, ads_frac, &dv);
                want[0] = out[0];
                want[1] = out[1];
            }
            _ => b.turn_speed = [0.0; 2],
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
        let taking = b.neg.map(|(l, _)| l).or(neg_ahead.filter(|_| at_begin));
        // BO2's Bot_UpdateMovement on a link: at the begin, toward its end
        // at half speed (from the eye, in the view's yaw axes); a drop, run
        // straight off, once it has been at the begin, full speed and
        // flat toward the end. Away from the begin (fallen short of a
        // ledge) it walks back to the begin, at half speed within 64 units.
        let began = b.neg_began || bo2_at_begin;
        let toward_end = link.filter(|l| if l.flags & NEG_DROP != 0 { began } else { bo2_at_begin });
        let speed = match (link, la) {
            (Some(l), Some((_, reach))) if l.flags & NEG_DROP == 0 && reach < 64.0 => 64.0,
            _ => speed,
        };
        if ladder {
            forward = 127;
        } else if let Some(l) = toward_end {
            if !mantling {
                if l.flags & NEG_DROP != 0 {
                    let wish = [l.end[0] - origin[0], l.end[1] - origin[1]];
                    let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
                    if len > 1e-3 {
                        let (s, c) = want[1].to_radians().sin_cos();
                        forward = (((wish[0] * c + wish[1] * s) / len).clamp(-1.0, 1.0) * 127.0) as i8;
                        right = (((wish[0] * s - wish[1] * c) / len).clamp(-1.0, 1.0) * 127.0) as i8;
                    }
                } else {
                    let e = [l.end[0] - eye[0], l.end[1] - eye[1], l.end[2] - eye[2]];
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
        } else if !mantling && !hold && let Some(p) = walk_to {
            let wish = [p[0] - origin[0], p[1] - origin[1]];
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
            b.sprint_since = None;
        } else if b.sprint_since.is_none() {
            b.sprint_since = Some(now);
        }
        let reach = la.map_or(0.0, |(_, r)| r);
        let sprint = if speed == 0.0 || yaw_err >= 5.0 || reach < dv.sprint_distance {
            sprinting && b.sprint_since.is_some_and(|t| now - t < 3000)
        } else {
            !reloading(wstate) && wstate != weapon_t6::state::MELEE_INIT && standing
        };
        if sprint && !b.no_sprint && b.stance == 0 && !ladder && !mantling {
            press |= buttons::SPRINT;
        }
        match b.stance {
            1 => press |= buttons::CROUCH,
            2 => press |= buttons::PRONE,
            _ => {}
        }
        // BO2: through a climb with no room to stand, the bot holds crouch
        // and lets go of prone, so it ends the climb ducked under the lintel.
        if low_climb {
            press = (press & !buttons::PRONE) | buttons::CROUCH;
        }
        // Holding use, it presses nothing else (a plant, a defuse).
        if b.use_until > now {
            press = (press & (buttons::CROUCH | buttons::PRONE)) | buttons::USE_RELOAD;
        }
        // The trigger: one press when allowed, then a rest of
        // bot_minfiretime..bot_maxfiretime ms; a full-automatic holds it.
        // Firing lets go of sprint.
        if b.attack_ticks > 0 {
            press = (press & !buttons::SPRINT) | buttons::ATTACK;
            b.attack_ticks -= 1;
        } else if b.allow_attack && now >= b.fire_delay_until {
            press = (press & !buttons::SPRINT) | buttons::ATTACK;
            b.fire_delay_until = if full_auto { 0 } else { now + irand(n, now, dv.min_fire, dv.max_fire) };
        }
        if b.ads {
            press |= buttons::ADS;
        }
        // A melee only while not raising, lowering, meleeing or throwing (BO2's
        // states: the knife's swing and end, the offhands, a detonator; the swim
        // in and out as a raise and drop, TODO(fact)).
        if std::mem::take(&mut b.melee)
            && !weapon_t6::state::is_raising_or_swim_out(wstate)
            && !weapon_t6::state::is_dropping_or_swim_in(wstate)
            && !(20..=29).contains(&wstate)
            && wstate != weapon_t6::state::DETONATING
        {
            press |= buttons::MELEE_CHARGE;
        }
        if std::mem::take(&mut b.dive) {
            press |= buttons::PRONE;
        }
        // The throw: its button once the view and the angles it chases both
        // face the point (so the view does not swing off it while the arm
        // comes up), or after THROW_AIM_MS of turning, and not while the
        // failsafe holds its buttons; then it faces there until the grenade
        // has left its hand (into the offhand states and out again). A press
        // the gun refused (still ready) is pressed again; the gun going down
        // first is part of the throw.
        let chasing = b.aim_avg;
        let free = b.failsafe.is_none();
        if let Some(t) = b.throw.as_mut() {
            use weapon_t6::state as S;
            let offhand = S::is_offhand(wstate) && wstate != S::OFFHAND_DETONATE;
            if offhand {
                t.started = true;
            } else if t.started || now - t.asked >= THROW_GIVE_UP_MS {
                b.throw = None;
            } else if t.pressed == 0 {
                let off = t.at.map_or(0.0, |at| {
                    let (yaw, pitch) = angles_to(eye, at);
                    let off = |a: [f32; 2]| angle_delta(a[1], yaw).abs().max(angle_delta(a[0], pitch).abs());
                    off([want[0], want[1]]).max(chasing.map_or(0.0, off))
                });
                if free && (off <= THROW_AIM_DEG || now - t.asked >= THROW_AIM_MS) {
                    press |= t.button;
                    t.pressed = now;
                }
            } else if S::is_ready_as_mirrored(wstate) && now - t.pressed >= 200 {
                t.pressed = 0;
            }
        }
        // Jumps and vaults: BO2's engine takes a bot over the negotiation
        // links its path nodes mark (ledges, railings, low walls); ours
        // jumps where the route climbs close ahead (the movement code
        // mantles from a jump), and when walking makes no headway.
        // On a ladder it climbs (looking up its route, walking forward):
        // a jump would let go of it.
        // It takes a link from its begin (ours: or within 20 units of it).
        // At a jump link's begin, not mantling (BO2's Bot_Seek), it jumps
        // the first time; then while a ledge is in reach (the mantle hint,
        // so a jump climbs it on the way up), else again on the whole second
        // if it let go of jump the tick before. At a mantle link (none of
        // the jump, ladder or drop marks) it presses the climb once the
        // ledge is in reach.
        if let Some(l) = neg_ahead.filter(|_| at_begin) {
            b.neg = Some((l, now));
            b.neg_began = false;
            if botlog() {
                diag::info!(
                    Sim,
                    "bo2mp bot {n} negotiation begin flags {:#x} at {:.0} {:.0} {:.0} begin {:.0} {:.0} {:.0} end {:.0} {:.0} {:.0} by {}",
                    l.flags,
                    origin[0],
                    origin[1],
                    origin[2],
                    l.begin[0],
                    l.begin[1],
                    l.begin[2],
                    l.end[0],
                    l.end[1],
                    l.end[2],
                    if bo2_at_begin { "the end distance" } else { "20 units" }
                );
            }
        }
        // At a jump link's begin, there the tick before too, it looks for a
        // window pane in its way on the whole second (after this tick's
        // moves are set, below).
        let mut glass_check = false;
        if let Some((l, _)) = b.neg
            && bo2_at_begin
        {
            let jump = l.flags & NEG_JUMP != 0
                && (ledge || !b.neg_began || (b.last_press & buttons::JUMP == 0 && now % 1000 == 0));
            let mantle = l.flags & (NEG_JUMP | NEG_LADDER | NEG_DROP) == 0 && ledge;
            if !mantling && (jump || mantle) {
                press |= buttons::JUMP;
            }
            glass_check = l.flags & NEG_JUMP != 0 && b.neg_began && now % 1000 == 0;
            b.neg_began = true;
        }
        if next.is_some() && !ladder && climb_blocked && taking.is_none() {
            press |= buttons::JUMP;
        }
        let mut stuck_for = None;
        if next.is_some() && b.failsafe.is_none() {
            match b.progress {
                Some((at, since)) if dist2d(at, origin) < 12.0 => {
                    let stuck = now - since;
                    stuck_for = Some(stuck);
                    if stuck > 600 && (stuck / 300) % 2 == 0 && !ladder && ahead_blocked && taking.is_none() {
                        press |= buttons::JUMP;
                    }
                    // No headway: back off and step aside for a moment
                    // (a doorway corner, another player, a prop), then on.
                    if (1200..2400).contains(&stuck) && !ladder {
                        forward = -127;
                        right = if (n + (since / 3000) as u32) % 2 == 0 { 127 } else { -127 };
                    }
                    if stuck % 1000 == 0 && stuck > 0 && botlog() {
                        diag::info!(
                            Sim,
                            "bo2mp bot {n} stuck {stuck} ms at {:.0} {:.0} {:.0} next {:?} ahead_blocked {ahead_blocked} in_solid {in_solid} touching {touching:?} goal {:?} routes {} left {} hold {hold} look {} stick {forward} {right} stance {} lookahead {:?}",
                            origin[0],
                            origin[1],
                            origin[2],
                            next.map(|p| [p[0] as i32, p[1] as i32, p[2] as i32]),
                            goal.as_ref().map(|g| (g.name.as_str(), [g.pos[0] as i32, g.pos[1] as i32, g.pos[2] as i32], g.radius as i32)),
                            b.path_gen,
                            b.path.len(),
                            b.look.is_some(),
                            b.stance,
                            b.la_dir.map(|(_, l)| l as i32)
                        );
                        // BO2MP_BOTNEGLOG (test aid): the route it is stuck on.
                        if botneglog() {
                            let first: Vec<[i32; 3]> = b.path.iter().take(4).map(|p| [p[0] as i32, p[1] as i32, p[2] as i32]).collect();
                            diag::info!(
                                Sim,
                                "bo2mp bot {n} stuck route {} first {first:?} prev {:?}",
                                b.path_gen,
                                b.prev.map(|p| [p[0] as i32, p[1] as i32, p[2] as i32])
                            );
                        }
                    }
                    // Still stuck: drop the route; the next tick plans again
                    // (not on a ladder: climbing makes no flat headway).
                    if stuck > 3000 && !ladder {
                        b.path.clear();
                        b.path_to = None;
                        b.replan_at = now + 250;
                        b.progress = Some((origin, now));
                    }
                }
                _ => b.progress = Some((origin, now)),
            }
        } else {
            b.progress = None;
        }
        // The failsafe (BO2's Bot_UpdateFailsafe), set by the stuck script:
        // it overrides the rest of the think. No sprint and no jump of its
        // own (G_BotThink still applies the script's buttons: stance, use,
        // fire, aim, melee), the view left alone; on a ladder straight on,
        // else straight at the node from the eye in the view's yaw axes;
        // within 15 units it stands and notifies "goal" with the node each
        // think until the script clears it.
        let mut failsafe_goal = None;
        if let Some((p, node)) = b.failsafe {
            press &= !(buttons::SPRINT | buttons::JUMP);
            want = view;
            (forward, right) = (0, 0);
            if ladder {
                forward = 127;
            } else if (0..3).map(|i| (p[i] - origin[i]).powi(2)).sum::<f32>() >= 225.0 {
                let e = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
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
            b.progress = None;
        }
        b.last_press = press;
        let fire_dbg = botfirelog().then(|| (b.allow_attack, b.fire_delay_until, b.look.is_some()));
        // BO2MP_BOTLOG=1: each bot every 2 s (test aid).
        if now % 2000 == 0 && botlog() {
            diag::info!(
                Sim,
                "bo2mp botlog {n}: at {:.0} {:.0} {:.0} view {:.0} {:.0} goal {:?} path {} walk {:?} face {:?} look {:?} attack {} ads {} press {press:#x} move {forward} {right} sprint {sprint} dodge {} hold {hold}",
                origin[0],
                origin[1],
                origin[2],
                view[0],
                view[1],
                goal.as_ref().map(|g| g.name.as_str()),
                b.path.len(),
                walk_to.map(|p| [p[0] as i32, p[1] as i32]),
                far_seen.map(|p| [p[0] as i32, p[1] as i32]),
                b.look.map(|l| [l[0] as i32, l[1] as i32, l[2] as i32]),
                b.allow_attack,
                b.ads,
                walk_to != next,
            );
        }
        let neg_dbg = b
            .neg
            .filter(|_| now % 100 == 0 && botneglog())
            .map(|(l, since)| (l.begin, now - since, press, forward, right, want[0], bo2_at_begin));
        let stuck_dbg = stuck_for
            .filter(|&s| s >= 200 && now % 100 == 0 && botneglog())
            .map(|s| (s, forward, right, view[1], walk_to, b.look, b.neg.map(|(l, _)| l.begin)));
        drop(zm);
        // BO2MP_BOTNEGLOG (test aid): a bot making no headway, every 100 ms:
        // how it moves, and what a player's box meets 24 units toward its walk.
        if let Some((stuck, forward, right, yaw, to, look, on_link)) = stuck_dbg
            && let Some((o, v, ground)) = frame(world)
                .player(ClientId(n))
                .map(|ps| (ps.origin, ps.velocity, ps.ground_entity_num != playerstate_t6::game::ENTITYNUM_NONE))
        {
            use crate::bullet_collision::{MASK_PLAYER_SOLID, PLAYER_MAXS, PLAYER_MINS};
            let to = to.unwrap_or(o);
            let d = [to[0] - o[0], to[1] - o[1]];
            let len = d[0].hypot(d[1]).max(1e-3);
            let end = [o[0] + d[0] / len * 24.0, o[1] + d[1] / len * 24.0, o[2]];
            let t = frame(world).trace_clip(o, end, PLAYER_MINS, PLAYER_MAXS, MASK_PLAYER_SOLID);
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
                t.fraction,
                t.normal[0],
                t.normal[1],
                t.normal[2],
                t.cflags,
                t.startsolid,
                look.map(|p| p.map(|c| c as i32)),
                on_link.map(|p| p.map(|c| c as i32))
            );
        }
        if let Some((begin, took, press, forward, right, pitch, at)) = neg_dbg
            && let Some(ps) = frame(world).player(ClientId(n))
        {
            diag::info!(
                Sim,
                "bo2mp bot {n} on link {:?} {took} ms: at {:.0} {:.0} {:.0} vel {:.0} {:.0} {:.0} ground {} ledge {} mantling {} jump {} move {forward} {right} pitch {pitch:.0} at begin {at}",
                begin.map(|c| c as i32),
                ps.origin[0],
                ps.origin[1],
                ps.origin[2],
                ps.velocity[0],
                ps.velocity[1],
                ps.velocity[2],
                ps.ground_entity_num != playerstate_t6::game::ENTITYNUM_NONE,
                ps.mantle_state.flags & playerstate_t6::game::mantle_flags::ACTIVE != 0,
                ps.pm_flags & playerstate_t6::game::pm_flags::MANTLE != 0,
                press & buttons::JUMP != 0,
            );
        }
        if glass_check {
            glass_ahead(world, n, eye, view);
        }
        if let Some(node) = failsafe_goal {
            let obj = world.resource::<Zm>().players.get(&n).map(|p| p.obj);
            if let Some(obj) = obj {
                let args: Vec<Value> = node.map(Value::Object).into_iter().collect();
                super::with_vm(world, |vm, world| vm.notify_str(world, obj, "goal", &args));
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
                view_yaw: want[1],
                steer_err: yaw_err,
                weapon,
                wstate,
                sprinting,
                sprint_wanted: sprint,
                allow,
                delay_until,
                looking,
                press,
            };
            log_botfire(world, n, &line);
        }
        // Its command for the tick, the view turned as a script's
        // setplayerangles turns it (the delta under the command angles).
        let mut cmd_angles = None;
        {
            let mut req = world.resource_mut::<crate::step::StepRequest>();
            for (id, cmd) in &mut req.input.cmds {
                if id.0 != n {
                    continue;
                }
                cmd.buttons = press;
                cmd.forwardmove = forward;
                cmd.rightmove = right;
                // Its gun, not the hand: an empty hand mid-throw would
                // otherwise ask for no gun, and the throw never ends.
                if select != 0 {
                    cmd.weapon = select as u16;
                    cmd.weapon_mapped = select as u16;
                }
                cmd_angles = Some(cmd.angles);
            }
        }
        if let Some(angles) = cmd_angles {
            let mut f = frame(world);
            if let Some(p) = f.player_mut(ClientId(n)) {
                p.delta_angles = std::array::from_fn(|i| {
                    let cmd_deg = (angles[i] as u16) as f32 * (360.0 / 65536.0);
                    want[i] - cmd_deg
                });
                p.viewangles = want;
            }
        }
    }
}

/// The gun a bot's command asks for, by a player's client rule
/// (`follow_held_weapon_select`): an empty hand changes nothing, so the gun an
/// offhand throw puts away is asked back when the throw ends (BO2 brings it
/// back by the usercmd's weapon); a gun the game puts in hand is followed
/// while the bot was asking for the one it held.
fn follow_select(select: &mut u32, held: &mut u32, weapon: u32) {
    if weapon == 0 {
        return;
    }
    if *select == 0 || (*select == *held && weapon != *held) {
        *select = weapon;
    }
    *held = weapon;
}

/// A route from a place to a goal over the path nodes (the goal last),
/// and the negotiation links it crosses; straight there when the map has no
/// route between them. It starts at `start` when given (a link's end), else
/// at the node nearest the place.
fn plan(world: &mut World, from: [f32; 3], start: Option<u32>, to: [f32; 3]) -> (Vec<[f32; 3]>, Vec<NegLink>) {
    let a = start.or_else(|| nearest_reachable(world, from));
    let (Some(a), Some(b)) = (a, nearest_reachable(world, to)) else {
        return (vec![to], Vec::new());
    };
    let nav = &world.resource::<Zm>().nav;
    let Some(nodes) = nav.path(a, b) else {
        if botneglog() {
            diag::info!(Sim, "bo2mp plan no route: from {from:?} node {a} to {to:?} node {b}");
        }
        return (vec![to], Vec::new());
    };
    let mut out: Vec<[f32; 3]> = nodes
        .iter()
        .map(|&i| nav.nodes[i as usize].origin)
        .collect();
    // BO2MP_BOTNEGLOG (test aid): a leg between linked nodes over 512 units.
    if botneglog() {
        for (w, p) in nodes.windows(2).zip(out.windows(2)) {
            if dist2d(p[0], p[1]) > 512.0 {
                diag::info!(Sim, "bo2mp plan long leg: node {} {:?} to node {} {:?}", w[0], p[0], w[1], p[1]);
            }
        }
    }
    let negs: Vec<NegLink> = nodes
        .windows(2)
        .filter(|w| nav.nodes[w[0] as usize].ty == super::nav::NODE_NEGOTIATION_BEGIN && nav.negotiation(w[0], w[1]))
        .map(|w| NegLink {
            begin: nav.nodes[w[0] as usize].origin,
            end: nav.nodes[w[1] as usize].origin,
            flags: nav.nodes[w[0] as usize].spawnflags,
            end_node: w[1],
        })
        .collect();
    out.push(to);
    // BO2 smooths each path it plans (Path_SmoothPath: from a node straight
    // on to the farthest later one it can walk to). Ours does it from where
    // the bot stands, up to three nodes on (a trace each), so a nearest
    // node behind it is no U-turn it then rocks on.
    let mut skipped = 0;
    while out.len() > 1 && skipped < 3 && walk_clear(world, from, out[1]) {
        out.remove(0);
        skipped += 1;
    }
    (out, negs)
}

/// The map's negotiation links by kind, once a run (test aid).
fn log_negotiations(world: &World) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let nav = &world.resource::<Zm>().nav;
    if nav.nodes.is_empty() {
        return;
    }
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut kinds: BTreeMap<String, u32> = BTreeMap::new();
    for node in nav.nodes.iter().filter(|n| n.ty == super::nav::NODE_NEGOTIATION_BEGIN) {
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
}
