//! One tick in a missile's life: where it got to, what it touched on the way, and what that set off.

use entity_t6::flight::path_kind::{STRAIGHT, AT_REST};
use trace_t6::surface_kind_of_flags;

use super::blast::{Blast, Debts, set_off_blasts, settle_debts};
use super::contact::{Moment, Touch, settle_touch};
use super::events::push_bounce_event;
use super::launch::fuse_waits_for_floor;
use super::rider::rider_origin;
use super::roll::roll_grenade;
use super::scene::{
    SURF_WATER, Sweep, entity_glass, glass_standing, in_water, missile_bodies, required_facts, script_models_without,
    startsolid_normal,
};
use super::flying::FlyingProjectile;
use super::stick::{park_missile_at, unstick_missile};
use super::vec3::{gap, norm};
use crate::step_view::StepWorldView;
use crate::match_ids::HarmOrigin;
use crate::glass_and_props::Strike;
use crate::{ShotRayRequest, HitTargetId, SHOT_WORLD_FILTER, SIM_STEP_MILLIS, StepNo, RayResult, level_time_ms};

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
fn fuse_due(clock_millis: i32, detonate_at_ms: Option<i32>, live: bool) -> bool {
    detonate_at_ms.is_some_and(|deadline| live && deadline <= clock_millis)
}

/// Writes `missile` back into the world's row for `entnum`.
fn store(world: &mut StepWorldView, slot_no: i32, missile: FlyingProjectile) {
    if let Some(row) = world.projectile_in_slot_mut(slot_no) {
        *row = missile;
    }
}

/// Runs the missile at entity `entnum` for one tick: fuse and cleanup, its move and whatever it touches, rolling, and
/// then the damage, events and blasts that came of it.
pub(crate) fn think_projectile(world: &mut StepWorldView, tick: StepNo, slot_no: i32) {
    let Some(mut shell) = world.projectile_in_slot(slot_no) else {
        return;
    };
    let clock = level_time_ms(tick);
    let prev = clock.saturating_sub(SIM_STEP_MILLIS as i32);
    if clock >= shell.cleanup_at_ms
        && shell.detonate_at_ms.is_none_or(|deadline| deadline > shell.cleanup_at_ms || !shell.in_flight)
    {
        let _ = world.drop_projectile_slot(slot_no);
        return;
    }
    let facts = required_facts(world, shell.gun);
    crate::homing::steer(world, &mut shell, clock, facts);
    let models = script_models_without(world, shell.id);
    let content = world.content();
    let masses = missile_bodies(world);
    let sweep = Sweep { content: &content, bodies: &masses, models: &models };

    // A missile stuck to a player rides his hit box while he lives. When he dies the corpse that would carry it has no
    // pose to ride, so it drops from where it hit to the floor below instead of hanging where he stood.
    if let Some(rider) = shell.rider {
        match rider_origin(&masses, rider) {
            Some(at) => {
                shell.origin = at;
                shell.at.base_point = at;
            }
            None => {
                shell.rider = None;
                let living = world
                    .alive_collision_poses()
                    .iter()
                    .any(|pose| pose.client == rider.client && pose.life_sequence == rider.life);
                if !living {
                    unstick_missile(tick, &mut shell);
                }
            }
        }
        store(world, slot_no, shell);
    }

    let start = shell.origin;
    // The move ends at the tick's time, or earlier when the fuse or the cleanup falls inside this tick.
    let mut eval_time = clock;
    if let Some(deadline) = shell.detonate_at_ms.filter(|&ms| shell.in_flight && ms <= clock) {
        eval_time = eval_time.min(deadline);
    }
    if shell.cleanup_at_ms > prev && shell.cleanup_at_ms <= eval_time {
        eval_time = eval_time.min(shell.cleanup_at_ms);
    }
    let stop_at = shell.spot_at(eval_time);
    let motion_vec = shell.speed_vector_at(eval_time);
    let speed = norm(motion_vec);

    if let Some(pane) = shell.stuck_pane {
        let standing = world.world_objects().pane_blocks_shots(pane);
        if standing && shell.detonate_at_ms.is_none_or(|deadline| deadline > clock) {
            store(world, slot_no, shell);
            return;
        }
        if !standing {
            unstick_missile(tick, &mut shell);
            shell.stuck_pane = None;
        }
    }
    if shell.at.tr_type == AT_REST && (facts.is_retrievable_knife() || shell.detonate_at_ms.is_none()) {
        return;
    }

    let mut debts = Debts::default();
    // Test log (BO2MP_SHOT_LOG): the bodies this stretch could strike, and the
    // nearest hit box to it (a bolt that flies through a zombie).
    if crate::gunfire::shot_log() {
        let along = |p: [f32; 3]| {
            let d: [f32; 3] = core::array::from_fn(|i| stop_at[i] - start[i]);
            let len2 = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).max(1e-6);
            let t = (((p[0] - start[0]) * d[0] + (p[1] - start[1]) * d[1] + (p[2] - start[2]) * d[2]) / len2)
                .clamp(0.0, 1.0);
            norm(core::array::from_fn(|i| p[i] - (start[i] + d[i] * t)))
        };
        let near = models
            .iter()
            .filter_map(|g| Some((g.owner, g.collision.as_ref()?)))
            .flat_map(|(o, c)| c.bones.iter().map(move |b| (o, b.bone, b.center)))
            .map(|(o, bone, c)| (o, bone, c.map(f32::round), along(c).round()))
            .min_by(|p, q| p.3.total_cmp(&q.3));
        diag::info!(
            Sim,
            "nade tick {clock}: {} of {} models posed, nearest box {near:?}",
            models.iter().filter(|g| g.collision.is_some()).count(),
            models.len()
        );
        // The zombie nearest the path, whatever its hit boxes: why it has
        // none (not solid, never posed, or posed then dropped).
        let nearest = world
            .entity_hit_support()
            .iter()
            .filter_map(|entry| {
                let d = entry.dobj.as_ref().filter(|d| d.active_mesh.contains("zom"))?;
                let at = d.placement_matrix.w_axis.truncate().to_array();
                Some((along(at), entry, d))
            })
            .min_by(|p, q| p.0.total_cmp(&q.0));
        if let Some((dist, entry, d)) = nearest {
            diag::info!(
                Sim,
                "nade tick {clock}: nearest model {:?} {} {dist:.0} away: solid {} hidden {} boxes {} model rev {}/{:?} pose rev {}/{:?}",
                entry.owner,
                d.active_mesh,
                entry.solid,
                entry.unseen,
                d.live_hitbox.is_some(),
                d.model_version_stamp,
                d.built_model_stamp,
                d.pose_rev,
                d.built_pose_stamp
            );
        }
    }
    let mut trace_start = start;
    let mut hops = 0u32;
    let mut hop_capped = false;
    // The scripted thing whose glass it went through this tick (once a tick; the second trace passes it by).
    let mut through = None;
    // A missile moving more than 20 this tick from out of the water also traces the water; striking it, it splashes
    // there (a bounce on the water) and traces again, through the water.
    let mut wet = !world.gun_facts().is_empty() && gap(stop_at, start) > WATER_TRACE_MIN_MOVE && !in_water(world, start);
    let ending = loop {
        let damage = world.world_objects().pane_harm_entries();
        let query = ShotRayRequest {
            from: trace_start,
            to: stop_at,
            bits: if wet { SHOT_WORLD_FILTER | CONTENTS_WATER } else { SHOT_WORLD_FILTER },
            ignore: Some(shell.owner),
            skip_client_hit: None,
            ignore_model: through,
        };
        let ending = sweep.missile(&query, &damage);
        if std::mem::take(&mut wet)
            && let RayResult::Hit { end: at, normal, struck: HitTargetId::World { surface_flags: face_bits, .. }, .. } = ending
            && surface_kind_of_flags(face_bits) == SURF_WATER
        {
            push_bounce_event(world, tick, &shell, at, normal, SURF_WATER);
            continue;
        }
        let went_through = match ending {
            RayResult::Hit { end: hit_end, struck: HitTargetId::World { glass_hit_code, .. }, .. }
            | RayResult::BeganInside { end: hit_end, struck: Some(HitTargetId::World { glass_hit_code, .. }) }
                if glass_hit_code != 0
                    && world.world_objects().is_t6_glass(u32::from(glass_hit_code).saturating_sub(1)) =>
            {
                let pane = u32::from(glass_hit_code).saturating_sub(1);
                if !world.world_objects().pane_blocks_shots(pane) {
                    false
                } else {
                    let _ = world.world_objects_mut().shatter_glass(pane, clock, Strike { at: hit_end, way: motion_vec });
                    hops = hops.saturating_add(1);
                    if hops >= PANE_HOPS_PER_TICK {
                        hop_capped = true;
                        park_missile_at(tick, &mut shell, hit_end, motion_vec);
                        false
                    } else {
                        let heading = [stop_at[0] - trace_start[0], stop_at[1] - trace_start[1], stop_at[2] - trace_start[2]];
                        let len = norm(heading).max(1.0);
                        trace_start = core::array::from_fn(|axis| hit_end[axis] + heading[axis] / len * PAST_PANE);
                        true
                    }
                }
            }
            // A car's window: the car takes the missile's damage as an impact (a breakable at least 1000) and the
            // missile flies on through it.
            RayResult::Hit { struck, .. } | RayResult::BeganInside { struck: Some(struck), .. }
                if through.is_none() && speed >= 1.0 && facts.collision_harm != 0 =>
            {
                match entity_glass(world, struck) {
                    Some((target, bone, breakable)) => {
                        let amount = if breakable { facts.collision_harm.max(1000) } else { facts.collision_harm };
                        debts.things.push((
                            HarmOrigin::Projectile(shell.id),
                            crate::gsc_host::EntityHit {
                                target,
                                amount,
                                attacker: Some(shell.owner),
                                means: "MOD_IMPACT",
                                gun: shell.gun,
                                point: start,
                                way: motion_vec,
                                bone,
                                bits: 0,
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
        break ending;
    };
    if hop_capped {
        shell.motion = motion_vec;
        store(world, slot_no, shell);
        return;
    }

    let mut hit = match ending {
        RayResult::Hit { fraction, end: stop_at, normal, struck } => Some(Touch { to: stop_at, face_dir: normal, collider: Some(struck), share: fraction }),
        // A trace that starts inside something faces back along the move.
        RayResult::BeganInside { struck, end: solid_end } => {
            Some(Touch { to: solid_end, face_dir: startsolid_normal(trace_start, stop_at), collider: struck, share: 0.0 })
        }
        RayResult::Miss { .. } | RayResult::Invalid { .. } => None,
    };
    // Riding on him, it touches nothing more.
    if shell.rider.is_some() {
        hit = None;
    }
    // A Hellstorm starts 18000 out past the map and only ever noses down, so the one he rides flies in through the
    // sky, and out of the nothing it starts in, instead of going off there.
    let ridden = world.ordered_player_ids().into_iter().any(|client| {
        world
            .client_meta(client)
            .and_then(|meta| meta.remote_missile)
            .is_some_and(|link| link.projectile == shell.id && link.unlink_at_ms.is_none())
    });
    if ridden && let Some(touch) = hit {
        let outside = match ending {
            RayResult::BeganInside { struck, .. } => struck.is_none_or(crate::shot_trace::is_sky),
            _ => touch.collider.is_some_and(crate::shot_trace::is_sky),
        };
        diag::info!(
            Sim,
            "bo2mp streaks: ridden missile {} touched {:?} at {:?} (from {:?}){}",
            shell.id.0,
            touch.collider,
            touch.to,
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
        && !matches!(touch.collider, Some(HitTargetId::Player { .. }))
    {
        let probe_start = core::array::from_fn(|axis| touch.to[axis] + touch.face_dir[axis] * 0.135);
        let probe_end = core::array::from_fn(|axis| touch.to[axis] - touch.face_dir[axis] * 1.5);
        let query = ShotRayRequest {
            from: probe_start,
            to: probe_end,
            bits: SHOT_WORLD_FILTER,
            ignore: Some(shell.owner),
            skip_client_hit: None,
            ignore_model: None,
        };
        let solid = |piece: u16| world.world_objects().pane_blocks_shots(u32::from(piece));
        if let RayResult::Hit { end: stop_at, normal, struck, fraction } = sweep.bullet(&masses, &query, &solid) {
            // Surface clearance comes before the knife's resting and embedded pose offsets.
            contact_origin = Some(core::array::from_fn(|axis| stop_at[axis] + (stop_at[axis] - probe_end[axis])));
            hit = Some(Touch { to: stop_at, face_dir: normal, collider: Some(struck), share: fraction });
        }
    }
    let contact_time = match hit {
        Some(touch) => prev.saturating_add(((eval_time - prev) as f32 * touch.share) as i32),
        None => eval_time,
    };
    // How fast it was going when it touched (not at the tick's end): a soft or hard hit is judged on that.
    let contact_vel = shell.speed_vector_at(contact_time);
    let mut fuse_ready = fuse_due(clock, shell.detonate_at_ms, shell.in_flight);
    if fuse_ready && !shell.grounded && fuse_waits_for_floor(world, &facts, shell.gun) {
        fuse_ready = false;
        shell.detonate_at_ms = Some(clock.saturating_add(SIM_STEP_MILLIS as i32));
    }
    let cleanup_ready = clock >= shell.cleanup_at_ms;
    let touched_first = hit.is_some()
        && !(shell.at.tr_type == AT_REST && fuse_ready)
        && shell.detonate_at_ms.is_none_or(|deadline| contact_time <= deadline);
    let ruling = if touched_first {
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
    let step_damage = world.world_objects().pane_harm_entries();
    let thrower = shell.owner;
    let step_off = |from: [f32; 3], till: [f32; 3]| -> f32 {
        let query = ShotRayRequest {
            from,
            to: till,
            bits: SHOT_WORLD_FILTER,
            ignore: Some(thrower),
            skip_client_hit: None,
            ignore_model: None,
        };
        match sweep.missile(&query, &step_damage) {
            RayResult::Hit { fraction, .. } => fraction,
            RayResult::Miss { .. } => 1.0,
            RayResult::BeganInside { .. } | RayResult::Invalid { .. } => 0.0,
        }
    };

    let mut blasts: Vec<Blast> = Vec::new();
    // The surface a rolling grenade touched this tick.
    let mut contact_normal: Option<[f32; 3]> = None;
    match ruling {
        Verdict::Cleanup => {
            let _ = world.drop_projectile_slot(slot_no);
            return;
        }
        Verdict::Fuse => {
            let detonation_delay = shell.detonate_at_ms.unwrap_or(eval_time);
            shell.origin = shell.spot_at(detonation_delay);
            shell.motion = shell.speed_vector_at(detonation_delay);
            blasts.push(Blast {
                missile: shell,
                spot: shell.origin,
                face_dir: [0.0, 0.0, 1.0],
                surface: 0,
                splash: shell.in_flight,
            });
        }
        Verdict::Fly => {
            if crate::gunfire::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {clock}: fly to {:?} vel {:?}",
                    stop_at.map(|v| (v * 10.0).round() / 10.0),
                    motion_vec.map(f32::round)
                );
            }
            // A frame in free flight leaves the ground: a grenade that hopped off a floor waits for its next floor
            // before a ground-only fuse goes off.
            if speed != 0.0 {
                shell.grounded = false;
            }
            shell.origin = stop_at;
            shell.travel_distance += gap(stop_at, start);
            shell.motion = shell.speed_vector_at(clock);
            crate::rocket_wobble::destabilize(world, clock, &mut shell);
            // A rolling grenade on the ground goes on to its roll below (ground, slope, resistance) every tick.
            let rolling = facts.rolling_radius.is_some() && shell.at.tr_type == STRAIGHT;
            if !rolling {
                store(world, slot_no, shell);
                return;
            }
        }
        Verdict::Contact => {
            let Some(touch) = hit else {
                unreachable!("a contact verdict needs a hit")
            };
            contact_normal = Some(touch.face_dir);
            if crate::gunfire::shot_log() {
                diag::info!(
                    Sim,
                    "nade tick {clock}: contact at {:?} normal {:?} frac {:.2} vel {:?} collider {:?}",
                    touch.to.map(|v| (v * 10.0).round() / 10.0),
                    touch.face_dir.map(|v| (v * 100.0).round() / 100.0),
                    touch.share,
                    motion_vec.map(f32::round),
                    touch.collider
                );
            }
            shell.origin = contact_origin.unwrap_or(touch.to);
            shell.travel_distance += gap(touch.to, start);
            let moment = Moment { time: clock, motion: motion_vec, bodies: &masses, step_off: &step_off };
            blasts.extend(settle_touch(world, tick, &mut shell, facts, touch, &moment, &mut debts));
        }
    }

    // A Black Ops II rolling grenade (the frag) rests and rolls on its body, not on its centre.
    if blasts.is_empty()
        && let Some(radius) = facts.rolling_radius
        && shell.at.tr_type != AT_REST
    {
        // Black Ops II rolls its bump chance each hit.
        let bump_roll = world.combat_dice_mut().draw_word() as f32 * (1.0 / 4_294_967_296.0);
        let roll_damage = world.world_objects().pane_harm_entries();
        let query_from = |from: [f32; 3], till: [f32; 3]| ShotRayRequest {
            from,
            to: till,
            bits: SHOT_WORLD_FILTER,
            ignore: Some(thrower),
            skip_client_hit: None,
            ignore_model: None,
        };
        let probe = |from: [f32; 3], till: [f32; 3]| -> Option<([f32; 3], [f32; 3])> {
            let solid = |piece: u16| glass_standing(&roll_damage, piece);
            match sweep.bullet(&masses, &query_from(from, till), &solid) {
                RayResult::Hit { end: stop_at, normal, struck, .. } if !matches!(struck, HitTargetId::Player { .. }) => {
                    Some((stop_at, normal))
                }
                _ => None,
            }
        };
        roll_grenade(tick, &mut shell, radius, contact_normal, contact_vel, bump_roll, &probe);
    }

    // A coasting missile that hits anything, or whose fuse runs out, is freed with no blast and no damage; the scripts
    // hear only its "death".
    if shell.coasting && !(blasts.is_empty() && debts.is_empty()) {
        let _ = world.drop_projectile_slot(slot_no);
        return;
    }
    if blasts.is_empty() {
        shell.motion = shell.speed_vector_at(clock);
        store(world, slot_no, shell);
    } else {
        let _ = world.drop_projectile_slot(slot_no);
    }
    for blast in blasts.iter().filter(|blast| blast.splash) {
        world.mark_shell_dying(tick, blast.missile);
    }
    let applies_harm = world.emits_state_snapshot();
    // A thrown grenade's move this tick damages the damage triggers the scripts set to feel it.
    if applies_harm && blasts.is_empty() && facts.gun_family_code == i32::from(weapon_t6::facts::gun_family_code::GRENADE) {
        crate::zombie_rt::damage_triggers::grenade_path(
            world.ecs(),
            shell.id.0,
            start,
            shell.origin,
            facts.blast_core_damage,
        );
    }
    if applies_harm {
        settle_debts(world, tick, debts);
    }
    set_off_blasts(world, tick, blasts, applies_harm);
}
