//! One tick in a missile's life: where it got to, what it touched on the way, and what that set off.

use entity_t6::trajectory::tr_type::{LINEAR, STATIONARY};
use trace_t6::surface_type_from_flags;

use super::blast::{self, Blast, Debts};
use super::contact::{self, Moment, Touch};
use super::events::push_bounce_event;
use super::launch::fuse_waits_for_floor;
use super::rider::rider_origin;
use super::roll::roll_grenade;
use super::scene::{
    SURF_WATER, Sweep, entity_glass, glass_standing, in_water, missile_bodies, required_facts, script_models_without,
    startsolid_normal,
};
use super::state::ProjectileState;
use super::stick::{park_missile_at, unstick_missile};
use super::vec3::{gap, norm};
use crate::frame::FrameWorld;
use crate::identities::DamageSource;
use crate::{BulletTraceQuery, ColliderId, MASK_BULLET_WORLD, MATCH_TICK_MS, Tick, TraceOutcome, level_time_ms};

/// How many panes one missile may break through in a tick before it is parked where it is (so it can never loop on
/// glass).
const PANE_HOPS_PER_TICK: u32 = 8;
const CONTENTS_WATER: u32 = movement_t6::water::CONTENTS_WATER;
/// A move longer than this from out of the water also traces the water surface.
const WATER_TRACE_MIN_MOVE: f32 = 20.0;
/// How far past a broken pane the trace resumes.
const PAST_PANE: f32 = 0.25;

/// What the tick comes to, in the order the rules give them priority.
enum Verdict {
    Contact,
    Fuse,
    Cleanup,
    Fly,
}

/// Whether the fuse has run out for a missile that is still live.
fn fuse_due(now_ms: i32, detonate_at_ms: Option<i32>, live: bool) -> bool {
    detonate_at_ms.is_some_and(|deadline| live && deadline <= now_ms)
}

/// Writes `missile` back into the world's row for `entnum`.
fn store(world: &mut FrameWorld, entnum: i32, missile: ProjectileState) {
    if let Some(row) = world.projectile_mut_by_number(entnum) {
        *row = missile;
    }
}

/// Runs the missile at entity `entnum` for one tick: fuse and cleanup, its move and whatever it touches, rolling, and
/// then the damage, events and blasts that came of it.
pub(crate) fn think_projectile(world: &mut FrameWorld, tick: Tick, entnum: i32) {
    let Some(mut projectile) = world.projectile_by_number(entnum) else {
        return;
    };
    let time = level_time_ms(tick);
    let prev = time.saturating_sub(MATCH_TICK_MS as i32);
    if time >= projectile.cleanup_at_ms
        && projectile.detonate_at_ms.is_none_or(|deadline| deadline > projectile.cleanup_at_ms || !projectile.live)
    {
        let _ = world.remove_projectile_by_number(entnum);
        return;
    }
    let facts = required_facts(world, projectile.weapon);
    crate::missile_guidance::steer(world, &mut projectile, time, facts);
    let models = script_models_without(world, projectile.id);
    let content = world.content();
    let bodies = missile_bodies(world);
    let sweep = Sweep { content: &content, bodies: &bodies, models: &models };

    // A missile stuck to a player rides his hit box while he lives. When he dies the corpse that would carry it has no
    // pose to ride, so it drops from where it hit to the floor below instead of hanging where he stood.
    if let Some(rider) = projectile.rider {
        match rider_origin(&bodies, rider) {
            Some(at) => {
                projectile.origin = at;
                projectile.pos.tr_base = at;
            }
            None => {
                projectile.rider = None;
                let alive = world
                    .alive_collision_poses()
                    .iter()
                    .any(|pose| pose.client == rider.client && pose.life_sequence == rider.life);
                if !alive {
                    unstick_missile(tick, &mut projectile);
                }
            }
        }
        store(world, entnum, projectile);
    }

    let start = projectile.origin;
    // The move ends at the tick's time, or earlier when the fuse or the cleanup falls inside this tick.
    let mut eval_time = time;
    if let Some(deadline) = projectile.detonate_at_ms.filter(|&ms| projectile.live && ms <= time) {
        eval_time = eval_time.min(deadline);
    }
    if projectile.cleanup_at_ms > prev && projectile.cleanup_at_ms <= eval_time {
        eval_time = eval_time.min(projectile.cleanup_at_ms);
    }
    let end = projectile.origin_at(eval_time);
    let vel = projectile.velocity_at(eval_time);
    let speed = norm(vel);

    if let Some(pane) = projectile.stuck_pane {
        let standing = world.world_objects().glass_is_solid(pane);
        if standing && projectile.detonate_at_ms.is_none_or(|deadline| deadline > time) {
            store(world, entnum, projectile);
            return;
        }
        if !standing {
            unstick_missile(tick, &mut projectile);
            projectile.stuck_pane = None;
        }
    }
    if projectile.pos.tr_type == STATIONARY && (facts.is_retrievable_knife() || projectile.detonate_at_ms.is_none()) {
        return;
    }

    let mut debts = Debts::default();
    let mut trace_start = start;
    let mut hops = 0u32;
    let mut hop_capped = false;
    // The scripted thing whose glass it went through this tick (once a tick; the second trace passes it by).
    let mut through = None;
    // A missile moving more than 20 this tick from out of the water also traces the water; striking it, it splashes
    // there (a bounce on the water) and traces again, through the water.
    let mut wet = !world.gun_facts().is_empty() && gap(end, start) > WATER_TRACE_MIN_MOVE && !in_water(world, start);
    let outcome = loop {
        let damage = world.world_objects().glass_damage_pairs();
        let query = BulletTraceQuery {
            start: trace_start,
            end,
            mask: if wet { MASK_BULLET_WORLD | CONTENTS_WATER } else { MASK_BULLET_WORLD },
            ignore: Some(projectile.owner),
            ignore_hit: None,
            ignore_model: through,
        };
        let outcome = sweep.missile(&query, &damage);
        if std::mem::take(&mut wet)
            && let TraceOutcome::Hit { end: at, normal, collider: ColliderId::World { surface_flags, .. }, .. } = outcome
            && surface_type_from_flags(surface_flags) == SURF_WATER
        {
            push_bounce_event(world, tick, &projectile, at, normal, SURF_WATER);
            continue;
        }
        let went_through = match outcome {
            TraceOutcome::Hit { end: hit_end, collider: ColliderId::World { glass_encoded, .. }, .. }
            | TraceOutcome::StartSolid { end: hit_end, collider: Some(ColliderId::World { glass_encoded, .. }) }
                if glass_encoded != 0
                    && world.world_objects().is_t6_glass(u32::from(glass_encoded).saturating_sub(1)) =>
            {
                let pane = u32::from(glass_encoded).saturating_sub(1);
                if !world.world_objects().glass_is_solid(pane) {
                    false
                } else {
                    let _ = world.world_objects_mut().force_shatter_glass(pane, time, hit_end, vel);
                    hops = hops.saturating_add(1);
                    if hops >= PANE_HOPS_PER_TICK {
                        hop_capped = true;
                        park_missile_at(tick, &mut projectile, hit_end, vel);
                        false
                    } else {
                        let heading = [end[0] - trace_start[0], end[1] - trace_start[1], end[2] - trace_start[2]];
                        let len = norm(heading).max(1.0);
                        trace_start = core::array::from_fn(|axis| hit_end[axis] + heading[axis] / len * PAST_PANE);
                        true
                    }
                }
            }
            // A car's window: the car takes the missile's damage as an impact (a breakable at least 1000) and the
            // missile flies on through it.
            TraceOutcome::Hit { collider, .. } | TraceOutcome::StartSolid { collider: Some(collider), .. }
                if through.is_none() && speed >= 1.0 && facts.impact_damage != 0 =>
            {
                match entity_glass(world, collider) {
                    Some((target, bone, breakable)) => {
                        let amount = if breakable { facts.impact_damage.max(1000) } else { facts.impact_damage };
                        debts.things.push((
                            DamageSource::Projectile(projectile.id),
                            crate::script::EntityHit {
                                target,
                                amount,
                                attacker: Some(projectile.owner),
                                means: "MOD_IMPACT",
                                weapon: projectile.weapon,
                                point: start,
                                dir: vel,
                                bone,
                                flags: 0,
                            },
                        ));
                        through = Some(target);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        };
        if went_through && !hop_capped {
            continue;
        }
        break outcome;
    };
    if hop_capped {
        projectile.velocity = vel;
        store(world, entnum, projectile);
        return;
    }

    let mut hit = match outcome {
        TraceOutcome::Hit { fraction, end, normal, collider } => Some(Touch { end, normal, collider: Some(collider), fraction }),
        // A trace that starts inside something faces back along the move.
        TraceOutcome::StartSolid { collider, end: solid_end } => {
            Some(Touch { end: solid_end, normal: startsolid_normal(trace_start, end), collider, fraction: 0.0 })
        }
        TraceOutcome::Miss { .. } | TraceOutcome::Invalid { .. } => None,
    };
    // Riding on him, it touches nothing more.
    if projectile.rider.is_some() {
        hit = None;
    }
    // A Hellstorm starts 18000 out past the map and only ever noses down, so the one he rides flies in through the
    // sky, and out of the nothing it starts in, instead of going off there.
    let ridden = world.client_ids_sorted().into_iter().any(|client| {
        world
            .client_meta(client)
            .and_then(|meta| meta.remote_missile)
            .is_some_and(|link| link.projectile == projectile.id && link.unlink_at_ms.is_none())
    });
    if ridden && let Some(touch) = hit {
        let outside = match outcome {
            TraceOutcome::StartSolid { collider, .. } => collider.is_none_or(crate::bullet_collision::is_sky),
            _ => touch.collider.is_some_and(crate::bullet_collision::is_sky),
        };
        diag::info!(
            Sim,
            "bo2mp streaks: ridden missile {} touched {:?} at {:?} (from {:?}){}",
            projectile.id.0,
            touch.collider,
            touch.end,
            trace_start,
            if outside { ", flies on" } else { "" }
        );
        if outside {
            hit = None;
        }
    }
    // A knife that hit anything but a player is measured again from the surface: it rests a hair off it.
    let mut contact_origin = None;
    if facts.is_retrievable_knife()
        && let Some(touch) = hit
        && !matches!(touch.collider, Some(ColliderId::Player { .. }))
    {
        let probe_start = core::array::from_fn(|axis| touch.end[axis] + touch.normal[axis] * 0.135);
        let probe_end = core::array::from_fn(|axis| touch.end[axis] - touch.normal[axis] * 1.5);
        let query = BulletTraceQuery {
            start: probe_start,
            end: probe_end,
            mask: MASK_BULLET_WORLD,
            ignore: Some(projectile.owner),
            ignore_hit: None,
            ignore_model: None,
        };
        let solid = |piece: u16| world.world_objects().glass_is_solid(u32::from(piece));
        if let TraceOutcome::Hit { end, normal, collider, fraction } = sweep.bullet(&bodies, &query, &solid) {
            // Surface clearance comes before the knife's resting and embedded pose offsets.
            contact_origin = Some(core::array::from_fn(|axis| end[axis] + (end[axis] - probe_end[axis])));
            hit = Some(Touch { end, normal, collider: Some(collider), fraction });
        }
    }
    let contact_time = match hit {
        Some(touch) => prev.saturating_add(((eval_time - prev) as f32 * touch.fraction) as i32),
        None => eval_time,
    };
    // How fast it was going when it touched (not at the tick's end): a soft or hard hit is judged on that.
    let contact_vel = projectile.velocity_at(contact_time);
    let mut fuse_ready = fuse_due(time, projectile.detonate_at_ms, projectile.live);
    if fuse_ready && !projectile.grounded && fuse_waits_for_floor(world, &facts, projectile.weapon) {
        fuse_ready = false;
        projectile.detonate_at_ms = Some(time.saturating_add(MATCH_TICK_MS as i32));
    }
    let cleanup_ready = time >= projectile.cleanup_at_ms;
    let touched_first = hit.is_some()
        && !(projectile.pos.tr_type == STATIONARY && fuse_ready)
        && projectile.detonate_at_ms.is_none_or(|deadline| contact_time <= deadline);
    let verdict = if touched_first {
        Verdict::Contact
    } else if fuse_ready {
        Verdict::Fuse
    } else if cleanup_ready {
        Verdict::Cleanup
    } else {
        Verdict::Fly
    };

    // How far a bounced grenade gets on a short move off the surface it hit (the same missile trace, the thrower
    // ignored).
    let step_damage = world.world_objects().glass_damage_pairs();
    let thrower = projectile.owner;
    let step_off = |from: [f32; 3], to: [f32; 3]| -> f32 {
        let query = BulletTraceQuery {
            start: from,
            end: to,
            mask: MASK_BULLET_WORLD,
            ignore: Some(thrower),
            ignore_hit: None,
            ignore_model: None,
        };
        match sweep.missile(&query, &step_damage) {
            TraceOutcome::Hit { fraction, .. } => fraction,
            TraceOutcome::Miss { .. } => 1.0,
            TraceOutcome::StartSolid { .. } | TraceOutcome::Invalid { .. } => 0.0,
        }
    };

    let mut blasts: Vec<Blast> = Vec::new();
    // The surface a rolling grenade touched this tick.
    let mut contact_normal: Option<[f32; 3]> = None;
    match verdict {
        Verdict::Cleanup => {
            let _ = world.remove_projectile_by_number(entnum);
            return;
        }
        Verdict::Fuse => {
            let fuse_time = projectile.detonate_at_ms.unwrap_or(eval_time);
            projectile.origin = projectile.origin_at(fuse_time);
            projectile.velocity = projectile.velocity_at(fuse_time);
            blasts.push(Blast {
                missile: projectile,
                origin: projectile.origin,
                normal: [0.0, 0.0, 1.0],
                surface: 0,
                splash: projectile.live,
            });
        }
        Verdict::Fly => {
            if crate::combat::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {time}: fly to {:?} vel {:?}",
                    end.map(|v| (v * 10.0).round() / 10.0),
                    vel.map(f32::round)
                );
            }
            // A frame in free flight leaves the ground: a grenade that hopped off a floor waits for its next floor
            // before a ground-only fuse goes off.
            if speed != 0.0 {
                projectile.grounded = false;
            }
            projectile.origin = end;
            projectile.travel_distance += gap(end, start);
            projectile.velocity = projectile.velocity_at(time);
            crate::missile_decoy::destabilize(world, time, &mut projectile);
            // A rolling grenade on the ground goes on to its roll below (ground, slope, resistance) every tick.
            let rolling = facts.rolling_radius.is_some() && projectile.pos.tr_type == LINEAR;
            if !rolling {
                store(world, entnum, projectile);
                return;
            }
        }
        Verdict::Contact => {
            let Some(touch) = hit else {
                unreachable!("a contact verdict needs a hit")
            };
            contact_normal = Some(touch.normal);
            if crate::combat::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {time}: contact at {:?} normal {:?} frac {:.2} vel {:?} collider {:?}",
                    touch.end.map(|v| (v * 10.0).round() / 10.0),
                    touch.normal.map(|v| (v * 100.0).round() / 100.0),
                    touch.fraction,
                    vel.map(f32::round),
                    touch.collider
                );
            }
            projectile.origin = contact_origin.unwrap_or(touch.end);
            projectile.travel_distance += gap(touch.end, start);
            let moment = Moment { time, velocity: vel, bodies: &bodies, step_off: &step_off };
            blasts.extend(contact::resolve(world, tick, &mut projectile, facts, touch, &moment, &mut debts));
        }
    }

    // A Black Ops II rolling grenade (the frag) rests and rolls on its body, not on its centre.
    if blasts.is_empty()
        && let Some(radius) = facts.rolling_radius
        && projectile.pos.tr_type != STATIONARY
    {
        // Black Ops II rolls its bump chance each hit.
        let bump_roll = world.combat_rng_mut().next_u32() as f32 * (1.0 / 4_294_967_296.0);
        let roll_damage = world.world_objects().glass_damage_pairs();
        let query_from = |from: [f32; 3], to: [f32; 3]| BulletTraceQuery {
            start: from,
            end: to,
            mask: MASK_BULLET_WORLD,
            ignore: Some(thrower),
            ignore_hit: None,
            ignore_model: None,
        };
        let probe = |from: [f32; 3], to: [f32; 3]| -> Option<([f32; 3], [f32; 3])> {
            let solid = |piece: u16| glass_standing(&roll_damage, piece);
            match sweep.bullet(&bodies, &query_from(from, to), &solid) {
                TraceOutcome::Hit { end, normal, collider, .. } if !matches!(collider, ColliderId::Player { .. }) => {
                    Some((end, normal))
                }
                _ => None,
            }
        };
        roll_grenade(tick, &mut projectile, radius, contact_normal, contact_vel, bump_roll, &probe);
    }

    // A coasting missile that hits anything, or whose fuse runs out, is freed with no blast and no damage; the scripts
    // hear only its "death".
    if projectile.coasting && !(blasts.is_empty() && debts.is_empty()) {
        let _ = world.remove_projectile_by_number(entnum);
        return;
    }
    if blasts.is_empty() {
        projectile.velocity = projectile.velocity_at(time);
        store(world, entnum, projectile);
    } else {
        let _ = world.remove_projectile_by_number(entnum);
    }
    for blast in blasts.iter().filter(|blast| blast.splash) {
        world.note_dying_missile(tick, blast.missile);
    }
    let resolves_damage = world.publishes_snapshot();
    // A thrown grenade's move this tick damages the damage triggers the scripts set to feel it.
    if resolves_damage && blasts.is_empty() && facts.weap_type == i32::from(weapon_t6::facts::weap_type::GRENADE) {
        crate::t6::damage_triggers::grenade_path(
            world.ecs(),
            projectile.id.0,
            start,
            projectile.origin,
            facts.explosion_inner_damage,
        );
    }
    if resolves_damage {
        blast::pay(world, tick, debts);
    }
    blast::detonate(world, tick, blasts, resolves_damage);
}
