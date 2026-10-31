//! bo2mp: Black Ops II's knife lunge around the weapon tick: the target
//! picked with the melee press (a player's client picks its own; the server picks a bot's),
//! and what the move needs of a running lunge (`movement_t6::bridge::bo2_lunge_update`).
//!
//! The weapon tick starts the lunge (`weapon_t6::pm::melee::charge_start`) when the command
//! names a target; `t6_gun` mirrors it into the move's player state, and the next move
//! launches it.

use crate::step_view::StepWorldView;
use crate::player_ledger::PlayerSessionStage;
use crate::sim_parts::SeatNo;
use crate::sim_parts::StepNo;
use playerstate_t6::game::{AvatarState, movement_state_bits};
use weapon_t6::facts::{FactsTable, gun_family_code};
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

/// The player picking a lunge target.
#[derive(Clone, Copy, Debug)]
pub struct LungePicker {
    pub spot: [f32; 3],
    pub eye: [f32; 3],
    pub look: [f32; 3],
    pub motion: [f32; 3],
    /// Base move speed (`ps.speed`), for the move limit.
    pub base_speed: f32,
    pub lying: bool,
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
    pub turn_deg: f32,
    /// The distance sent (30 short of the real one).
    pub dist: u8,
}

/// Auto-melee reach of the held weapon: a melee weapon's own lunge range, a bayonet's, or
/// the mode's default.
pub fn reach(facts: &FactsTable, gun: u32, zombies: bool) -> f32 {
    let f = facts.get(gun);
    let melee_weapon = f.core.gun_family_code == gun_family_code::CLOSE_COMBAT || f.melee.use_as_melee;
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
    if me.lying {
        return None;
    }
    let (ahead, side, rise) = game_math::euler_basis(me.look);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut closest: Option<(f32, u16, [f32; 3])> = None;
    for c in candidates {
        let gap = [
            c.origin[0] - me.spot[0],
            c.origin[1] - me.spot[1],
            c.origin[2] - me.spot[2],
        ];
        if dot(gap, gap) > me.range * me.range {
            continue;
        }
        let center = [c.origin[0], c.origin[1], c.origin[2] + BODY_CENTER];
        let to = [center[0] - me.eye[0], center[1] - me.eye[1], center[2] - me.eye[2]];
        let depth = dot(to, ahead);
        if depth <= 1.0 {
            continue;
        }
        let x = dot(to, side) / depth;
        let y = dot(to, rise) / depth;
        if x.abs() - BODY_HALF_WIDTH / depth > REGION_TAN_X
            || y.abs() - BODY_CENTER / depth > REGION_TAN_Y
        {
            continue;
        }
        let crosshair = x * x + y * y;
        if closest.is_some_and(|(b, _, _)| b <= crosshair) || !clear(me.eye, center) {
            continue;
        }
        closest = Some((crosshair, c.ent, gap));
    }
    let (_, ent, gap) = closest?;
    if !lunge::move_limit_ok(gap, me.motion, me.base_speed) {
        return None;
    }
    Some(LungePick {
        ent,
        turn_deg: game_math::heading_of(gap[0], gap[1]),
        dist: lunge::charge_cmd_dist(dot(gap, gap).sqrt()),
    })
}

/// The picker for a player state.
pub fn picker(ps: &AvatarState, range: f32) -> LungePicker {
    let [x, y, z] = ps.origin;
    let (angles, motion_vec) = (ps.look_angles, ps.velocity);
    LungePicker {
        spot: [x, y, z],
        eye: [x, y, z + ps.eye_height_now],
        look: angles,
        motion: motion_vec,
        base_speed: ps.speed as f32,
        lying: ps.movement_state_bits & movement_state_bits::LYING_FLAT != 0,
        range,
    }
}

/// A bot's lunge target (bots send no command of their own): the same pick over the live
/// enemies the server holds.
pub(crate) fn bot_pick(
    world: &StepWorldView,
    who: SeatNo,
    ps: &AvatarState,
    facts: &FactsTable,
    zombies: bool,
) -> Option<LungePick> {
    let team = world.client_meta(who).map_or(0, |m| m.seat_side);
    let mut foes: Vec<LungeCandidate> = Vec::new();
    world.walk_avatars(|other, ops| {
        let enemy = world.client_meta(other).is_some_and(|m| {
            matches!(m.lifecycle, PlayerSessionStage::Alive)
                && m.seat_side != entity_t6::team::SPECTATOR
                && (team == 0 || m.seat_side != team)
        });
        if other != who
            && ops.movement_mode <= 1
            && enemy
            && let Ok(ent) = u16::try_from(other.0)
        {
            foes.push(LungeCandidate { ent, origin: ops.origin });
        }
    });
    pick(picker(ps, reach(facts, ps.weapon, zombies)), foes, |eye, center| {
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
    world: &StepWorldView,
    who: SeatNo,
    step: StepNo,
    zombies: bool,
) -> movement_t6::bridge::Bo2Lunge {
    let (ent, hit_delay_ms) = crate::gun_brain::lunge_state(world, who);
    let target = ent.map(|e| SeatNo(u32::from(e))).and_then(|foe| {
        // Client prediction: its world has no other players, only where it draws them.
        if let Some((spot, drift)) = world.lunge_seen(foe) {
            log_aim(world, who, true, spot, spot);
            return Some(movement_t6::bridge::LungeTarget { origin: spot, velocity: drift });
        }
        if !world.client_meta(foe).is_some_and(|m| m.lifecycle == PlayerSessionStage::Alive) {
            return None;
        }
        let p = world.avatar(foe)?;
        let origin = seen_by(world, who, foe, step).unwrap_or(p.origin);
        log_aim(world, who, false, origin, p.origin);
        Some(movement_t6::bridge::LungeTarget { origin, velocity: p.velocity })
    });
    movement_t6::bridge::Bo2Lunge { target, hit_delay_ms, zombies }
}

/// Test aid (BO2MP_LUNGE_LOG=1): each lunge move, where it aimed and where the target was.
fn log_aim(world: &StepWorldView, who: SeatNo, client: bool, aim: [f32; 3], clock: [f32; 3]) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ENABLED.get_or_init(|| std::env::var_os("BO2MP_LUNGE_LOG").is_some()) {
        return;
    }
    let Some(me) = world.avatar(who) else {
        return;
    };
    diag::info!(
        Sim,
        "lunge move by c{} ({} side) at {}: attacker ({:.1} {:.1}), swing at ({:.1} {:.1}), target now ({:.1} {:.1}), lunge clock {}",
        who.0,
        if client { "client" } else { "server" },
        me.last_cmd_ms,
        me.origin[0],
        me.origin[1],
        aim[0],
        aim[1],
        clock[0],
        clock[1],
        me.lunge_clock
    );
}

/// The target's body at the moment the attacker's command saw, between the two snapshots he
/// was drawing (none: no claim, or no history of this life to go back to).
fn seen_by(world: &StepWorldView, who: SeatNo, foe: SeatNo, step: StepNo) -> Option<[f32; 3]> {
    let sample = world.lagcomp_sample(who).filter(|s| s.still_fresh_at(step))?;
    let life = world.client_meta(foe)?.life_sequence;
    let oldest = step.0.saturating_sub(crate::MAX_COMPENSATION_STEPS);
    let at = |k: u32| {
        world
            .collision_history()
            .sample_for(crate::StepNo(k.max(oldest)))?
            .poses
            .iter()
            .find(|pose| pose.client == foe && pose.life_sequence == life)
            .map(|pose| pose.origin)
    };
    let a = at(sample.left.0)?;
    let b = at(sample.right.0).unwrap_or(a);
    let f = sample.opacity.clamp(0.0, 1.0);
    Some([0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> LungePicker {
        let prone = false;
        LungePicker {
            spot: <[f32; 3]>::default(),
            eye: [0.0, 0.0, 60.0],
            look: <[f32; 3]>::default(),
            // Walking at the enemy: beyond 20 units BO2 only lunges while moving toward it.
            motion: [190.0, 0.0, 0.0],
            base_speed: 190.0,
            lying: prone,
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
        assert!(pick(LungePicker { lying: true, ..me() }, [at(1, 90.0, 0.0)], |_, _| true).is_none());
        assert!(pick(me(), [at(1, 90.0, 0.0)], |_, _| false).is_none());
        assert!(pick(me(), [at(1, -90.0, 0.0)], |_, _| true).is_none());
        assert!(pick(me(), [at(1, 101.0, 0.0)], |_, _| true).is_none());
    }

    #[test]
    fn backing_away_drops_the_lunge() {
        let away = LungePicker { motion: [-190.0, 0.0, 0.0], ..me() };
        assert!(pick(away, [at(1, 90.0, 0.0)], |_, _| true).is_none());
        let close = LungePicker { motion: [-190.0, 0.0, 0.0], ..me() };
        assert!(pick(close, [at(1, 19.0, 0.0)], |_, _| true).is_some());
    }
}
