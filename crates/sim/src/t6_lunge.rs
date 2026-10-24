//! bo2mp: Black Ops II's knife lunge around the weapon tick: the target
//! picked with the melee press (a player's client picks its own; the server picks a bot's),
//! and what the move needs of a running lunge (`movement_iw4::bo2_lunge_update`).
//!
//! The weapon tick starts the lunge (`weapon_t6::pm::melee::charge_start`) when the command
//! names a target; `t6_gun` mirrors it into the move's player state, and the next move
//! launches it.

use crate::frame::FrameWorld;
use crate::match_state::ClientLifecycle;
use crate::world::ClientId;
use playerstate_iw4::{PlayerState, pm_flags};
use weapon_t6::facts::{FactsTable, weap_type};
use weapon_t6::pm::melee::lunge;

/// The T6 brain runs the gun (and so the lunge) on a map with BO2's weapon facts.
pub fn brain_on(facts: &FactsTable) -> bool {
    !facts.is_empty()
}

/// The auto-melee screen region (`aim_automelee_region` 320x240 of the 640x480 screen) as
/// half-tangents of the default view.
const REGION_TAN_X: f32 = 0.849 * 0.5;
const REGION_TAN_Y: f32 = 0.478 * 0.5;
/// A target's body, for the region test and the line-of-sight trace.
const BODY_HALF_WIDTH: f32 = 15.0;
const BODY_CENTER: f32 = 36.0;
const TEAM_SPECTATOR: i32 = 3;

/// The player picking a lunge target.
#[derive(Clone, Copy, Debug)]
pub struct LungePicker {
    pub origin: [f32; 3],
    pub eye: [f32; 3],
    pub viewangles: [f32; 3],
    pub velocity: [f32; 3],
    /// Base move speed (`ps.speed`), for the move limit.
    pub base_speed: f32,
    pub prone: bool,
    /// Auto-melee reach ([`reach`]).
    pub range: f32,
}

/// One enemy that could be lunged at.
#[derive(Clone, Copy, Debug)]
pub struct LungeCandidate {
    pub ent: u16,
    pub origin: [f32; 3],
}

/// What goes into the command with the melee press.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LungePick {
    pub ent: u16,
    pub yaw: f32,
    /// The distance sent (30 short of the real one).
    pub dist: u8,
}

/// Auto-melee reach of the held weapon: a melee weapon's own lunge range, a bayonet's, or
/// the mode's default.
pub fn reach(facts: &FactsTable, weapon: u32, zombies: bool) -> f32 {
    let f = facts.get(weapon);
    let melee_weapon = f.core.weap_type == weap_type::MELEE || f.melee.use_as_melee;
    lunge::auto_melee_range(&f.melee, melee_weapon, zombies)
}

/// The lunge target: the enemy in reach under the crosshair, or the one nearest it inside
/// the auto-melee region, with a clear line from the eye (`clear(eye, body centre)`). None
/// when prone, or when moving away from it.
pub fn pick(
    me: LungePicker,
    candidates: impl IntoIterator<Item = LungeCandidate>,
    clear: impl Fn([f32; 3], [f32; 3]) -> bool,
) -> Option<LungePick> {
    if me.prone {
        return None;
    }
    let (forward, right, up) = math_iw4::angle_vectors(me.viewangles);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut best: Option<(f32, u16, [f32; 3])> = None;
    for c in candidates {
        let delta = [
            c.origin[0] - me.origin[0],
            c.origin[1] - me.origin[1],
            c.origin[2] - me.origin[2],
        ];
        if dot(delta, delta) > me.range * me.range {
            continue;
        }
        let center = [c.origin[0], c.origin[1], c.origin[2] + BODY_CENTER];
        let to = [center[0] - me.eye[0], center[1] - me.eye[1], center[2] - me.eye[2]];
        let depth = dot(to, forward);
        if depth <= 1.0 {
            continue;
        }
        let x = dot(to, right) / depth;
        let y = dot(to, up) / depth;
        if x.abs() - BODY_HALF_WIDTH / depth > REGION_TAN_X
            || y.abs() - BODY_CENTER / depth > REGION_TAN_Y
        {
            continue;
        }
        let crosshair = x * x + y * y;
        if best.is_some_and(|(b, _, _)| b <= crosshair) || !clear(me.eye, center) {
            continue;
        }
        best = Some((crosshair, c.ent, delta));
    }
    let (_, ent, delta) = best?;
    if !lunge::move_limit_ok(delta, me.velocity, me.base_speed) {
        return None;
    }
    Some(LungePick {
        ent,
        yaw: math_iw4::vec_to_yaw(delta[0], delta[1]),
        dist: lunge::charge_cmd_dist(dot(delta, delta).sqrt()),
    })
}

/// The picker for a player state.
pub fn picker(ps: &PlayerState, range: f32) -> LungePicker {
    LungePicker {
        origin: ps.origin,
        eye: [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current],
        viewangles: ps.viewangles,
        velocity: ps.velocity,
        base_speed: ps.speed as f32,
        prone: ps.pm_flags & pm_flags::PRONE != 0,
        range,
    }
}

/// A bot's lunge target (bots send no command of their own): the same pick over the live
/// enemies the server holds.
pub(crate) fn bot_pick(
    world: &FrameWorld,
    id: ClientId,
    ps: &PlayerState,
    facts: &FactsTable,
    zombies: bool,
) -> Option<LungePick> {
    let team = world.client_meta(id).map_or(0, |m| m.client_state_team);
    let mut candidates = Vec::new();
    world.visit_players(|other, ops| {
        let enemy = world.client_meta(other).is_some_and(|m| {
            m.lifecycle == ClientLifecycle::Alive
                && m.client_state_team != TEAM_SPECTATOR
                && (team == 0 || m.client_state_team != team)
        });
        if other != id
            && ops.pm_type <= 1
            && enemy
            && let Ok(ent) = u16::try_from(other.0)
        {
            candidates.push(LungeCandidate { ent, origin: ops.origin });
        }
    });
    pick(picker(ps, reach(facts, ps.weapon, zombies)), candidates, |eye, center| {
        world
            .trace_static_world(eye, center, [0.0; 3], [0.0; 3], crate::MASK_SHOT)
            .fraction
            >= 1.0
    })
}

/// The move's view of this player's lunge: the target's body where the attacker saw it, and
/// the swing's time to its hit. BO2 runs the lunge against the target as the attacker's
/// screen showed it: the server moves the target back to the attacker's view (as for his
/// bullets), the client takes the target where it draws it.
pub(crate) fn move_lunge(
    world: &FrameWorld,
    id: ClientId,
    tick: crate::Tick,
    zombies: bool,
) -> movement_iw4::Bo2Lunge {
    let (ent, hit_delay_ms) = crate::t6_gun::lunge_state(world, id);
    let target = ent.map(|e| ClientId(u32::from(e))).and_then(|t| {
        // Client prediction: its world has no other players, only where it draws them.
        if let Some((origin, velocity)) = world.lunge_seen(t) {
            log_aim(world, id, true, origin, origin);
            return Some(movement_iw4::LungeTarget { origin, velocity });
        }
        if !world.client_meta(t).is_some_and(|m| m.lifecycle == ClientLifecycle::Alive) {
            return None;
        }
        let p = world.player(t)?;
        let origin = seen_by(world, id, t, tick).unwrap_or(p.origin);
        log_aim(world, id, false, origin, p.origin);
        Some(movement_iw4::LungeTarget { origin, velocity: p.velocity })
    })
    .or_else(|| {
        // bo2zm: a zombie (entity numbers past the players).
        let feet = crate::t6::zombie_feet(world.ecs_ref(), u32::from(ent?))?;
        Some(movement_iw4::LungeTarget { origin: feet, velocity: [0.0; 3] })
    });
    movement_iw4::Bo2Lunge { target, hit_delay_ms, zombies }
}

/// Test aid (BO2MP_LUNGE_LOG=1): each lunge move, where it aimed and where the target was.
fn log_aim(world: &FrameWorld, id: ClientId, client: bool, aim: [f32; 3], now: [f32; 3]) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("BO2MP_LUNGE_LOG").is_some()) {
        return;
    }
    let Some(me) = world.player(id) else {
        return;
    };
    diag::info!(
        Sim,
        "lunge aim: c{} {} at {} me ({:.1} {:.1}) aim ({:.1} {:.1}) now ({:.1} {:.1}) t{}",
        id.0,
        if client { "client" } else { "server" },
        me.command_time,
        me.origin[0],
        me.origin[1],
        aim[0],
        aim[1],
        now[0],
        now[1],
        me.melee_charge_time
    );
}

/// The target's body at the moment the attacker's command saw, between the two snapshots he
/// was drawing (none: no claim, or no history of this life to go back to).
fn seen_by(world: &FrameWorld, id: ClientId, t: ClientId, tick: crate::Tick) -> Option<[f32; 3]> {
    let sample = world.lagcomp_sample(id).filter(|s| s.is_valid_for(tick))?;
    let life = world.client_meta(t)?.life_sequence;
    let oldest = tick.0.saturating_sub(crate::LAGCOMP_MAX_REWIND_TICKS);
    let at = |k: u32| {
        world
            .collision_history()
            .frame_at(crate::Tick(k.max(oldest)))?
            .poses
            .iter()
            .find(|pose| pose.client == t && pose.life_sequence == life)
            .map(|pose| pose.origin)
    };
    let a = at(sample.left.0)?;
    let b = at(sample.right.0).unwrap_or(a);
    let f = sample.alpha.clamp(0.0, 1.0);
    Some([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> LungePicker {
        LungePicker {
            origin: [0.0; 3],
            eye: [0.0, 0.0, 60.0],
            viewangles: [0.0; 3],
            // Walking at the enemy: beyond 20 units BO2 only lunges while moving toward it.
            velocity: [190.0, 0.0, 0.0],
            base_speed: 190.0,
            prone: false,
            range: lunge::AUTO_MELEE_RANGE_MP,
        }
    }

    fn at(ent: u16, x: f32, y: f32) -> LungeCandidate {
        LungeCandidate { ent, origin: [x, y, 0.0] }
    }

    #[test]
    fn picks_the_enemy_nearest_the_crosshair_in_reach() {
        let p = pick(me(), [at(1, 90.0, 20.0), at(2, 95.0, 0.0), at(3, 150.0, 0.0)], |_, _| true);
        assert_eq!(p.map(|p| (p.ent, p.dist)), Some((2, 65)));
    }

    #[test]
    fn no_target_when_prone_blocked_behind_or_out_of_reach() {
        assert!(pick(LungePicker { prone: true, ..me() }, [at(1, 90.0, 0.0)], |_, _| true).is_none());
        assert!(pick(me(), [at(1, 90.0, 0.0)], |_, _| false).is_none());
        assert!(pick(me(), [at(1, -90.0, 0.0)], |_, _| true).is_none());
        assert!(pick(me(), [at(1, 101.0, 0.0)], |_, _| true).is_none());
    }

    #[test]
    fn backing_away_drops_the_lunge() {
        let away = LungePicker { velocity: [-190.0, 0.0, 0.0], ..me() };
        assert!(pick(away, [at(1, 90.0, 0.0)], |_, _| true).is_none());
        let close = LungePicker { velocity: [-190.0, 0.0, 0.0], ..me() };
        assert!(pick(close, [at(1, 19.0, 0.0)], |_, _| true).is_some());
    }
}
