//! bo2mp: Black Ops II's rocket wobble and its missile decoys.
//!
//! A rocket that wobbles (a SMAW, an RPG, a grenade launcher's shell, the
//! FHJ, the strafe run's rockets: `destabilizationRateTime` set in its def)
//! flies straight for its first `destabilizeDistance` units, then turns a
//! little every frame at three rates, pitch, yaw and roll, each up to
//! `destabilizationCurvatureMax` degrees a second, rolling new ones every
//! rate time. Each new pitch turns against the last yaw and each new yaw
//! against the last pitch, so it weaves instead of curling away. A locked
//! sidewinder rocket wobbles on top of its steering. While it wobbles, the
//! level's attractors draw it toward an entity or a point and its repulsors
//! push it up and away (the Escort Drone's `missile_createattractorent`
//! draws rockets onto itself). A player's `setstablemissile( 1 )` makes the
//! rockets he fires from then on fly straight.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::FRAC_2_PI;

use bevy_ecs::resource::Resource;
use bevy_ecs::world::{Mut, World};
use entity_t6::Trajectory;
use entity_t6::flight::path_kind;
use glam::Vec3;
use weapon_t6::facts::MissileFacts;

use crate::FlyingProjectile;
use crate::step_view::StepWorldView;
use crate::match_ids::MatchRandom;
use crate::zombie_rt::Zm;
use crate::sim_parts::SeatNo;

/// How many attractors and repulsors a level holds at once.
pub(crate) const SLOTS: usize = 32;
/// A slot's entity number when it holds a point instead of an entity.
pub(crate) const AT_POINT: u32 = 0x3ff;
/// `guidedMissileType_t`: none, sidewinder.
const GUIDED_NONE: i32 = 0;
const GUIDED_SIDEWINDER: i32 = 1;
/// A server frame, in seconds.
const FRAME_SECONDS: f32 = 0.05;
/// A missile not seen in flight this long is gone.
const FORGET_MS: i32 = 1000;
/// Each missile's wobble rolls from its own stream.
const WOBBLE_SEED: u64 = 0x6D69_7373_696C_6557;

/// An attractor or repulsor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pull {
    /// It draws missiles in (else it pushes them away).
    pub attractor: bool,
    /// The entity it rides, or [`AT_POINT`].
    pub ent_no: u32,
    /// Where it stands when it is a point.
    pub origin: [f32; 3],
    pub strength: f32,
    /// How near a missile must be to feel it.
    pub max_dist: f32,
}

#[derive(Clone, Debug)]
struct Wobble {
    /// When it rolls new rates.
    next_roll_ms: i32,
    /// Its turn rates (pitch, yaw, roll), degrees a second, once it has rolled.
    rates: Option<[f32; 3]>,
    /// It flies straight: its def does not wobble, or its player fired it stable.
    stable: bool,
    rng: MatchRandom,
    seen_ms: i32,
}

impl Wobble {
    fn launched(id: u32, launched_ms: i32, m: &MissileFacts, stable: bool) -> Self {
        let straight_ms =
            (m.destabilize_distance as f32 / m.missile_velocity as f32) * 1000.0;
        Self {
            next_roll_ms: launched_ms + straight_ms as i32,
            rates: None,
            stable,
            rng: MatchRandom::new(WOBBLE_SEED ^ u64::from(id)),
            seen_ms: launched_ms,
        }
    }

    /// This frame's heading from `heading`, or None while it flies straight.
    fn frame(&mut self, m: &MissileFacts, time: i32, heading: [f32; 3]) -> Option<[f32; 3]> {
        if self.stable {
            return None;
        }
        if self.next_roll_ms < time {
            let most = m.destabilize_curvature_max;
            let rng = &mut self.rng;
            self.rates = Some(match self.rates {
                None => [(); 3].map(|_| flrand(rng, -1.0, 1.0) * most),
                Some(old_rates) => {
                    let mut rates = [(); 3].map(|_| flrand(rng, 0.0, 1.0) * most);
                    if old_rates[1] < 0.0 {
                        rates[0] = -rates[0];
                    }
                    if old_rates[0] > 0.0 {
                        rates[1] = -rates[1];
                    }
                    rates
                }
            });
            self.next_roll_ms = time + (m.destabilize_rate_time * 1000.0) as i32;
        }
        let rates = self.rates?;
        Some(core::array::from_fn(|i| heading[i] + rates[i] * FRAME_SECONDS))
    }
}

/// A random float in `[lo, hi)` from the match stream.
fn flrand(rng: &mut MatchRandom, lo: f32, hi: f32) -> f32 {
    let unit = rng.draw_word() as f32 * (1.0 / 4_294_967_296.0);
    lo + (hi - lo) * unit
}

/// The level's attractors and repulsors, the players who fire stable, and
/// each rocket's wobble.
#[derive(Resource, Default)]
pub(crate) struct Decoys {
    pulls: [Option<Pull>; SLOTS],
    /// Players whose rockets fly straight (`setstablemissile`).
    stable: BTreeSet<u32>,
    /// Each missile seen in flight, by projectile id.
    wobbles: BTreeMap<u32, Wobble>,
}

fn decoys(world: &mut World) -> Mut<'_, Decoys> {
    if !world.contains_resource::<Decoys>() {
        world.insert_resource(Decoys::default());
    }
    world.resource_mut::<Decoys>()
}

/// The first free slot, for `missile_create*`.
pub(crate) fn free_slot(world: &mut World) -> Result<usize, String> {
    decoys(world)
        .pulls
        .iter()
        .position(|slot| slot.is_none())
        .ok_or_else(|| format!("Ran out of attractor/repulsors.  Max allowed: {SLOTS}"))
}

/// `missile_create*` puts `pull` in slot `at`.
pub(crate) fn place(world: &mut World, at: usize, pull: Pull) -> Result<(), String> {
    if pull.max_dist <= 0.0 {
        return Err("maxDist must be greater than zero".to_owned());
    }
    decoys(world).pulls[at] = Some(pull);
    Ok(())
}

/// `missile_deleteattractor(slot)`.
pub(crate) fn delete(world: &mut World, slot: i32) -> Result<(), String> {
    let slot = usize::try_from(slot)
        .ok()
        .filter(|s| *s < SLOTS)
        .ok_or("Invalid attractor or repulsor")?;
    decoys(world).pulls[slot] = None;
    Ok(())
}

/// The slots an entity held go when it does.
pub(crate) fn free_refs(world: &mut World, gone: u32) {
    let Some(mut d) = world.get_resource_mut::<Decoys>() else {
        return;
    };
    for slot in &mut d.pulls {
        if slot.is_some_and(|p| p.ent_no == gone) {
            slot.take();
        }
    }
}

/// Every frame: the slots of entities the level freed go.
pub(crate) fn sweep(world: &mut World) {
    let Some(d) = world.get_resource::<Decoys>() else {
        return;
    };
    let Some(zm) = world.get_resource::<Zm>() else {
        return;
    };
    let gone: Vec<u32> = d
        .pulls
        .iter()
        .flatten()
        .map(|p| p.ent_no)
        .filter(|n| *n != AT_POINT && !zm.players.contains_key(n) && !zm.ents.contains_key(n))
        .collect();
    for n in gone {
        free_refs(world, n);
    }
}

/// `setstablemissile(on)`: the rockets `shooter` fires from now fly straight.
pub(crate) fn set_stable(world: &mut World, shooter: u32, on: bool) {
    let mut d = decoys(world);
    if on {
        d.stable.insert(shooter);
    } else {
        d.stable.remove(&shooter);
    }
}

/// The level starts again: every slot and wobble goes.
pub(crate) fn restart(world: &mut World) {
    world.insert_resource(Decoys::default());
}

/// A missile's frame after its move in free flight (BO2's
/// `RunMissile_Destabilize`, then `Missile_ApplyAttractorsRepulsors`): a
/// wobbling rocket turns and the level's attractors and repulsors bend it.
/// It flies on from where it is at its new velocity.
pub(crate) fn destabilize(world: &mut StepWorldView, time: i32, shot: &mut FlyingProjectile) {
    if world.gun_facts().is_empty() {
        return;
    }
    let facts = world.gun_facts().get(shot.gun);
    let (core, m) = (facts.core, facts.missile);
    let guidance = match core.guided_missile_type {
        GUIDED_NONE => true,
        GUIDED_SIDEWINDER => m.destabilize_distance > 0,
        _ => false,
    };
    if core.gun_family_code != weapon_t6::facts::gun_family_code::PROJECTILE
        || !guidance
        || m.missile_velocity <= 0
        || shot.motion == [0.0; 3]
    {
        return;
    }
    let anchors = anchors(world);
    let turned = {
        let mut d = decoys(world.ecs());
        let stable = m.destabilize_rate_time == 0.0 || d.stable.contains(&shot.owner.0);
        let w = d
            .wobbles
            .entry(shot.id.0)
            .or_insert_with(|| Wobble::launched(shot.id.0, shot.spawn_time_ms, &m, stable));
        w.seen_ms = time;
        let turned = w.frame(&m, time, shot.turn_path.base_point);
        d.wobbles.retain(|_, w| w.seen_ms >= time - FORGET_MS);
        turned
    };
    let Some(heading) = turned else {
        return;
    };
    let speed = m.missile_velocity as f32;
    let forward = game_math::euler_basis(heading).0;
    let (velocity, heading) = bend(&anchors, shot.origin, forward.map(|v| v * speed), speed)
        .unwrap_or((forward.map(|v| v * speed), heading));
    shot.motion = velocity;
    shot.turn_path = path(path_kind::BLEND, time, heading, [0.0; 3]);
    shot.at = path(path_kind::STRAIGHT, time, shot.origin, velocity);
}

/// An open-ended trajectory of `kind` that starts at `time` from `from`, changing by `rate`.
fn path(kind: u8, time: i32, from: [f32; 3], rate: [f32; 3]) -> Trajectory {
    Trajectory {
        tr_type: kind,
        began_at: time,
        span_ms: Default::default(),
        base_point: from,
        rate,
    }
}

/// Each attractor and repulsor in use, with where it stands now.
fn anchors(world: &mut StepWorldView) -> Vec<(Pull, [f32; 3])> {
    let pulls: Vec<Pull> = world
        .ecs()
        .get_resource::<Decoys>()
        .map(|d| d.pulls.iter().flatten().copied().collect())
        .unwrap_or_default();
    let mut out = Vec::with_capacity(pulls.len());
    for p in pulls {
        let at = if p.ent_no == AT_POINT {
            Some(p.origin)
        } else {
            entity_origin(world, p.ent_no)
        };
        if let Some(pos) = at {
            out.push((p, pos));
        }
    }
    out
}

fn entity_origin(world: &mut StepWorldView, n: u32) -> Option<[f32; 3]> {
    let (player, ent) = {
        let zm = world.ecs().get_resource::<Zm>()?;
        (zm.players.contains_key(&n), zm.ents.get(&n).map(|e| e.origin))
    };
    if player {
        return world.avatar(SeatNo(n)).map(|ps| ps.origin);
    }
    ent
}

/// The attractors and repulsors bend a missile at `at` flying at `velocity`:
/// its new velocity (`speed` long) and heading, or None when none reaches
/// it. Each one ahead of it and within its distance pulls across its line,
/// harder the nearer it is and the further off the line; an attractor never
/// turns it past the point, and a repulsor above its line pushes it up.
fn bend(
    anchors: &[(Pull, [f32; 3])],
    at: [f32; 3],
    motion_vec: [f32; 3],
    speed: f32,
) -> Option<([f32; 3], [f32; 3])> {
    let v = Vec3::from_array(motion_vec);
    let len = v.length();
    if len < 1e-5 {
        return None;
    }
    let dir = v / len;
    let mut force = Vec3::ZERO;
    for (p, anchor) in anchors {
        let delta = Vec3::from_array(*anchor) - Vec3::from_array(at);
        let ahead = delta.dot(dir);
        if ahead <= 0.0 {
            continue;
        }
        let perp = delta - dir * ahead;
        let mut side = perp.length();
        let mut across = if side > 0.0 { perp / side } else { perp };
        if side < 1e-5 {
            if p.attractor {
                continue;
            }
            across = Vec3::NEG_Z;
        }
        if !p.attractor && across.z > 0.0 {
            across = Vec3::NEG_Z;
            side = 0.0;
        }
        let range = delta.length();
        if range > p.max_dist {
            continue;
        }
        let mut share = (side.abs() / ahead).atan() * FRAC_2_PI;
        if !p.attractor {
            share -= 1.0;
        }
        let mut push = share * (1.0 - range / p.max_dist) * p.strength;
        if p.attractor {
            let most = speed * side / ahead * 20.0;
            if push >= most {
                push = most;
            }
        }
        force += across * push;
    }
    if force == Vec3::ZERO {
        return None;
    }
    let bent = v + force * FRAME_SECONDS;
    let n = bent.length();
    let dir = if n > 0.0 { bent / n } else { bent };
    Some(((dir * speed).to_array(), gsc_t6::math::vector_to_angles(dir.to_array())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// smaw_mp's def (common_mp.ff).
    const SMAW: MissileFacts = MissileFacts {
        missile_velocity: 2500,
        destabilize_rate_time: 0.4,
        destabilize_curvature_max: 10.0,
        destabilize_distance: 300,
    };

    #[test]
    fn a_rocket_flies_straight_until_its_distance_then_turns_each_frame() {
        // 300 units at 2500 a second: 120 ms straight, rolled after that.
        let mut w = Wobble::launched(7, 1000, &SMAW, false);
        assert_eq!(w.next_roll_ms, 1120);
        assert_eq!(w.frame(&SMAW, 1100, [0.0; 3]), None);
        assert_eq!(w.frame(&SMAW, 1120, [0.0; 3]), None);
        let turned = w.frame(&SMAW, 1150, [0.0; 3]).expect("it wobbles past its distance");
        let rates = w.rates.unwrap();
        for i in 0..3 {
            assert!(rates[i].abs() <= 10.0);
            assert!((turned[i] - rates[i] * 0.05).abs() < 1e-6);
        }
        // The next roll comes a rate time (400 ms) later; until then it keeps turning the same.
        assert_eq!(w.next_roll_ms, 1550);
        let again = w.frame(&SMAW, 1200, turned).unwrap();
        assert!((again[1] - turned[1] - rates[1] * 0.05).abs() < 1e-6);
        assert_eq!(w.rates, Some(rates));
    }

    #[test]
    fn each_new_roll_turns_against_the_last() {
        let mut w = Wobble::launched(3, 0, &SMAW, false);
        w.rates = Some([2.0, -3.0, 1.0]);
        w.next_roll_ms = 0;
        w.frame(&SMAW, 50, [0.0; 3]).unwrap();
        let r = w.rates.unwrap();
        // Last yaw < 0: pitch goes negative; last pitch > 0: yaw goes negative; roll stays up.
        assert!(r[0] <= 0.0 && r[1] <= 0.0 && r[2] >= 0.0);
        w.rates = Some([-2.0, 3.0, 1.0]);
        w.next_roll_ms = 0;
        w.frame(&SMAW, 100, [0.0; 3]).unwrap();
        let r = w.rates.unwrap();
        assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[2] >= 0.0);
    }

    #[test]
    fn a_stable_rocket_never_turns() {
        let mut w = Wobble::launched(1, 0, &SMAW, true);
        assert_eq!(w.frame(&SMAW, 5000, [0.0; 3]), None);
        assert_eq!(w.rates, None);
    }

    fn pull(draws: bool, at: [f32; 3]) -> (Pull, [f32; 3]) {
        let p = Pull { attractor: draws, ent_no: AT_POINT, origin: at, strength: 1000.0, max_dist: 2000.0 };
        (p, at)
    }

    #[test]
    fn an_attractor_ahead_draws_the_rocket_toward_it() {
        let a = [pull(true, [1000.0, 200.0, 0.0])];
        let (v, angles) = bend(&a, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0).unwrap();
        assert!(v[1] > 0.0, "it turns toward the attractor: {v:?}");
        assert!((Vec3::from_array(v).length() - 2500.0).abs() < 0.5);
        assert!(angles[1] > 0.0 && angles[1] < 90.0);
    }

    #[test]
    fn a_repulsor_pushes_it_away_and_one_above_pushes_it_up() {
        let below = [pull(false, [1000.0, 0.0, -200.0])];
        let (v, _) = bend(&below, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0).unwrap();
        assert!(v[2] > 0.0, "away from the repulsor below: {v:?}");
        let above = [pull(false, [1000.0, 0.0, 200.0])];
        let (v, _) = bend(&above, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0).unwrap();
        assert!(v[2] > 0.0, "a repulsor above still pushes it up: {v:?}");
    }

    #[test]
    fn behind_or_beyond_reach_does_nothing() {
        let behind = [pull(true, [-500.0, 100.0, 0.0])];
        assert_eq!(bend(&behind, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0), None);
        let far = [pull(true, [3000.0, 100.0, 0.0])];
        assert_eq!(bend(&far, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0), None);
        // An attractor dead on its line has nothing to pull across.
        let on_line = [pull(true, [1000.0, 0.0, 0.0])];
        assert_eq!(bend(&on_line, [0.0; 3], [2500.0, 0.0, 0.0], 2500.0), None);
    }

    #[test]
    fn slots_fill_free_and_run_out() {
        let mut world = World::new();
        let p = Pull { attractor: true, ent_no: 40, origin: <[f32; 3]>::default(), strength: 1.0, max_dist: 10.0 };
        for i in 0..SLOTS {
            let slot = free_slot(&mut world).unwrap();
            assert_eq!(slot, i);
            place(&mut world, slot, p).unwrap();
        }
        assert_eq!(
            free_slot(&mut world),
            Err("Ran out of attractor/repulsors.  Max allowed: 32".to_owned())
        );
        delete(&mut world, 5).unwrap();
        assert_eq!(free_slot(&mut world), Ok(5));
        assert!(delete(&mut world, 32).is_err() && delete(&mut world, -1).is_err());
        assert!(place(&mut world, 5, Pull { max_dist: 0.0, ..p }).is_err());
        free_refs(&mut world, 40);
        assert_eq!(free_slot(&mut world), Ok(0));
        restart(&mut world);
        assert!(world.resource::<Decoys>().pulls.iter().all(Option::is_none));
    }
}
