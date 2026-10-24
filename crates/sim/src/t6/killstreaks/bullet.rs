//! A scorestreak gun's bullets: a turret's (`Fire_Lead`: the Sentry Gun)
//! and a vehicle's (`VEH_FireGunnerWeapon`, `G_TraceBulletPathForVehTurret`:
//! the Stealth Chopper's and the VTOL's guns, the drones').
//!
//! Each is traced through the world as a player's bullet is, at the players
//! as they stand now (no rewind), with the gun's own spread: its weapon's
//! `aiSpread`, or `playerSpread` when a player sits at it. Whatever it
//! strikes takes the hit where it lands: a player through the scripts'
//! damage callback (the turret or vehicle as the inflictor, and as the
//! attacker unless a player sits at it: `figureoutattacker` gives the kill
//! to its owner), a dog, a breakable, a streak, a damage trigger, glass.
//! A riot shield may turn it, and it flies on as a player's bullet does
//! (`BulletRicochet`, `combat::shield_bounce`).

use std::cell::RefCell;
use std::collections::HashMap;

use bevy_ecs::prelude::World;
use gsc_t6::{Value, Vm};

use super::super::{T6_HITLOC_NAMES, Zm, frame, tick};
use super::streaks;
use crate::bullet::bullet_damage_at_distance;
use crate::bullet_collision::{
    BulletTraceQuery, BulletTraceSegment, ColliderId, EntityCollisionCapabilities, HITLOC_SHIELD,
    MASK_BULLET_WORLD, bullet_trace_segments_filtered, glass_piece_from_hit,
};
use crate::identities::{DamageSource, MatchRng, ShotId};
use crate::match_state::{ClientLifecycle, EventAudience};
use crate::world::ClientId;

/// How far ahead of a gunner's muzzle his own vehicle must be clear for
/// his gun to fire (`VEH_FireGunnerWeapon`).
pub(super) const HULL_CLEAR: f32 = 500.0;

/// A streak gun firing.
pub(super) struct Gun<'a> {
    /// The turret or vehicle.
    pub ent: u32,
    /// The player sitting at it, when one is: he is the attacker, and the
    /// gun spreads as his (`playerSpread`).
    pub shooter: Option<u32>,
    /// The player it fires for: the damage triggers, breakables and streaks
    /// it hits are his hits.
    pub owner: Option<u32>,
    pub weapon: &'a str,
    pub w: u32,
}

/// The gun's spread (degrees): `playerSpread` with a player at it, else
/// `aiSpread`; none on content that has no Black Ops II weapon facts.
fn spread(world: &mut World, gun: &Gun) -> f32 {
    let f = frame(world);
    if f.gun_facts().is_empty() {
        return 0.0;
    }
    cone(&f.gun_facts().get(gun.w).spread, gun.shooter.is_some())
}

fn cone(s: &weapon_t6::pm::spread::SpreadFacts, seated: bool) -> f32 {
    if seated { s.player_spread } else { s.ai_spread }
}

/// One trigger pull: each of its pellets from `start` along `dir` (turned by
/// the gun's spread). The players it struck.
pub(super) fn fire(
    vm: &mut Vm<World>,
    world: &mut World,
    gun: &Gun,
    start: [f32; 3],
    dir: [f32; 3],
) -> Vec<u32> {
    let t = tick(world);
    let (model, number) = world
        .resource::<Zm>()
        .presences
        .by_ent
        .get(&gun.ent)
        .map_or((None, playerstate_t6::game::ENTITYNUM_NONE), |p| {
            (Some(p.id), p.number)
        });
    let correlation = {
        let mut st = streaks(world);
        st.shots = st.shots.wrapping_add(1);
        st.shots
    };
    let Some(facts) = frame(world).combat_facts_for(gun.w) else {
        return Vec::new();
    };
    let spread = spread(world, gun);
    let pellets = facts.pellet_count().clamp(1, 32) as u16;
    let range = facts.bullet_range();
    let angles = super::aim_angles(dir);
    let mut rng = MatchRng::new(
        (u64::from(t.0) << 32) ^ (u64::from(gun.ent) << 16) ^ u64::from(correlation),
    );
    let mut struck = Vec::new();
    for pellet in 0..pellets {
        let d = crate::combat::spread_pellet_direction(angles, spread, &mut rng);
        // Each stretch of its flight: a riot shield that turns it sends it
        // on past the shield's bearer, the gun's full range from there.
        let (mut from, mut way, mut reach, mut past) = (start, d, range, None);
        let mut bounces = 0u8;
        loop {
            let end: [f32; 3] = std::array::from_fn(|i| from[i] + way[i] * reach);
            let (segments, poses) = trace(world, gun, model, from, end, past);
            let shot = Shot {
                gun,
                number,
                correlation,
                pellet,
                start: from,
                end,
                dir: way,
                impact_type: facts.impact_type,
            };
            for v in hits(vm, world, &shot, &segments, &poses) {
                if !struck.contains(&v) {
                    struck.push(v);
                }
            }
            let seed = (u64::from(correlation) << 24) ^ (u64::from(pellet) << 8) ^ u64::from(bounces);
            let Some(bounce) = crate::combat::shield_bounce(
                &frame(world),
                gun.w,
                way,
                seed,
                bounces,
                &segments,
                &poses,
            ) else {
                break;
            };
            (from, way, reach, past) = (
                bounce.origin,
                bounce.direction,
                bounce.max_range,
                Some(bounce.bearer),
            );
            bounces += 1;
        }
    }
    struck
}

/// Whether a gunner's gun has its own vehicle in the way: the `HULL_CLEAR`
/// units ahead of its muzzle cross the vehicle's own model.
pub(super) fn own_hull_blocks(world: &mut World, ent: u32, start: [f32; 3], dir: [f32; 3]) -> bool {
    let Some(id) = world
        .resource::<Zm>()
        .presences
        .by_ent
        .get(&ent)
        .map(|p| p.id)
    else {
        return false;
    };
    let f = frame(world);
    let rows: Vec<_> = f
        .entity_collision_capabilities()
        .iter()
        .filter(|r| r.owner.script_model() == Some(id))
        .map(EntityCollisionCapabilities::trace_geom)
        .collect();
    if rows.is_empty() {
        return false;
    }
    let end: [f32; 3] = std::array::from_fn(|i| start[i] + dir[i] * HULL_CLEAR);
    let (segments, _) = bullet_trace_segments_filtered(
        f.clip_brushes(),
        f.clip_bsp(),
        f.clip_cmodels(),
        f.clip_mesh(),
        &[],
        &rows,
        &BulletTraceQuery {
            start,
            end,
            mask: MASK_BULLET_WORLD,
            ignore: None,
            ignore_hit: None,
            ignore_model: None,
        },
        f.bullet_pen_facts_for(0),
        f.penetration_table(),
        &|_| true,
        None,
    );
    segments.iter().any(|s| struck_model(s.collider, id))
}

/// Whether a bullet's stretch ended on that script model.
fn struck_model(collider: Option<ColliderId>, id: crate::ScriptModelId) -> bool {
    matches!(
        collider,
        Some(
            ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. }
        ) if owner.script_model() == Some(id)
    )
}

/// One pellet's path: what it passes through and stops in, past the gun
/// itself, the player at it and `past` (the bearer of a shield it bounced
/// off); and the players as they stood.
fn trace(
    world: &mut World,
    gun: &Gun,
    model: Option<crate::ScriptModelId>,
    start: [f32; 3],
    end: [f32; 3],
    past: Option<ClientId>,
) -> (
    Vec<BulletTraceSegment>,
    Vec<crate::bullet_collision::PlayerCollisionPose>,
) {
    let f = frame(world);
    let poses = f.alive_collision_poses();
    let rows: Vec<_> = f
        .entity_collision_capabilities()
        .iter()
        .map(EntityCollisionCapabilities::trace_geom)
        .collect();
    let facts = f.combat_facts_for(gun.w);
    // Each pane's own BO2 health, as the gun's trace has it: a hit takes
    // its damage (5+ only) and the next hop meets the pane while any is
    // left (Bullet_Process damages the pane; Bullet_Trace sees only
    // standing panes).
    let glass_health = RefCell::new(
        f.world_objects()
            .t6_glass_healths()
            .into_iter()
            .collect::<HashMap<u32, f32>>(),
    );
    let glass_seen = RefCell::new(Vec::<u32>::new());
    let on_glass_hit = |piece: u16, at: [f32; 3]| {
        let pane = u32::from(piece);
        if glass_seen.borrow().contains(&pane) {
            return;
        }
        glass_seen.borrow_mut().push(pane);
        let Some(facts) = facts else {
            return;
        };
        let scaled = bullet_damage_at_distance(&facts, dist(start, at)).max(0);
        if let Some(health) = glass_health.borrow_mut().get_mut(&pane).filter(|h| **h > 0.0) {
            *health = glass_t6::server::health_after_hit(
                *health,
                scaled,
                crate::world_objects::T6_GLASS_DAMAGE_MULTIPLIER,
            );
        }
    };
    let (segments, _) = bullet_trace_segments_filtered(
        f.clip_brushes(),
        f.clip_bsp(),
        f.clip_cmodels(),
        f.clip_mesh(),
        &poses,
        &rows,
        &BulletTraceQuery {
            start,
            end,
            mask: MASK_BULLET_WORLD,
            ignore: gun.shooter.map(ClientId),
            ignore_hit: past,
            ignore_model: model,
        },
        f.bullet_pen_facts_for(gun.w),
        f.penetration_table(),
        &|piece| {
            glass_health
                .borrow()
                .get(&u32::from(piece))
                .is_none_or(|health| *health > 0.0)
        },
        Some(&on_glass_hit),
    );
    (segments, poses)
}

/// One pellet in flight.
struct Shot<'a, 'b> {
    gun: &'a Gun<'b>,
    /// The gun's entity number in the snapshot.
    number: i32,
    correlation: u32,
    pellet: u16,
    start: [f32; 3],
    end: [f32; 3],
    dir: [f32; 3],
    impact_type: i32,
}

/// What a pellet's path strikes, each in turn: its effects and its damage.
/// The players it hit.
fn hits(
    vm: &mut Vm<World>,
    world: &mut World,
    shot: &Shot,
    segments: &[BulletTraceSegment],
    poses: &[crate::bullet_collision::PlayerCollisionPose],
) -> Vec<u32> {
    let gun = shot.gun;
    let t = tick(world);
    let bo2 = !frame(world).gun_facts().is_empty();
    let mut struck = Vec::new();
    let shown = segments
        .iter()
        .filter(|s| crate::combat::shows_impact(s.collider))
        .count();
    let startsolid = segments.first().is_some_and(|s| s.startsolid);
    // Nothing shown (the sky, out of reach): its tracer still flies on.
    if shown == 0 && !startsolid {
        frame(world).push_pellet_fx(crate::PelletFxRecord {
            attacker: shot.number,
            weapon: gun.w,
            correlation: shot.correlation,
            pellet: shot.pellet,
            hand: 0,
            start: shot.start,
            end: segments.last().map_or(shot.end, |s| s.end),
            normal: [0.0; 3],
            surf_type: 0,
            surface_flags: 0,
            flesh_flags: 0,
        });
    }
    let inflictor = super::craft::world_obj(world, gun.ent);
    let attacker = gun
        .shooter
        .map_or_else(|| inflictor.clone(), |c| super::craft::world_obj(world, c));
    let credit = gun.shooter.or(gun.owner).map(ClientId);
    let armor_piercing = {
        let f = frame(world);
        !f.gun_facts().is_empty() && f.gun_facts().get(gun.w).core.armor_piercing
    };
    let mut glass_hit: Vec<u32> = Vec::new();
    for segment in segments {
        let exit = segment.surface_flags & fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG != 0;
        let scaled = {
            let f = frame(world);
            f.combat_facts_for(gun.w).map_or(0, |facts| {
                ((bullet_damage_at_distance(&facts, dist(shot.start, segment.end)) as f32)
                    * segment.damage_mult) as i32
            })
        };
        let mut flesh_flags = 0u8;
        if !exit && let Some(ColliderId::Player { client, hitloc, .. }) = segment.collider {
            let mut fatal = false;
            let alive = frame(world)
                .client_meta(client)
                .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive);
            if alive && scaled > 0 {
                let (amount, means, flags) = {
                    let f = frame(world);
                    // A hit the riot shield stopped carries the bullet's
                    // whole damage to the scripts.
                    let scale = if hitloc == HITLOC_SHIELD {
                        1.0
                    } else {
                        f.combat_facts_for(gun.w)
                            .map_or(1.0, |facts| facts.location_scale(hitloc))
                    };
                    let means = crate::script_player::means(
                        &f,
                        DamageSource::Shot(ShotId(shot.correlation)),
                        gun.w,
                        hitloc,
                        false,
                    );
                    let flags = if segment.penetrated {
                        crate::script_player::IDFLAGS_PENETRATION
                    } else {
                        0
                    } | if armor_piercing {
                        crate::script_player::IDFLAGS_NO_ARMOR
                    } else {
                        0
                    };
                    (weapon_t6::hitloc::scaled_damage(scaled, scale), means, flags)
                };
                if amount > 0 {
                    let place = if hitloc == HITLOC_SHIELD {
                        "riotshield"
                    } else {
                        T6_HITLOC_NAMES
                            .get(usize::from(hitloc))
                            .copied()
                            .unwrap_or("none")
                    };
                    super::super::natives_ai::player_damage(
                        vm,
                        world,
                        client.0,
                        inflictor.clone(),
                        attacker.clone(),
                        amount,
                        flags,
                        means,
                        gun.weapon,
                        segment.end,
                        shot.dir,
                        place,
                    );
                    if !struck.contains(&client.0) {
                        struck.push(client.0);
                    }
                    fatal = frame(world)
                        .player(client)
                        .is_none_or(|ps| ps.health <= 0);
                }
            }
            flesh_flags = fx_t6::runtime::flesh_hit_flags(weapon_t6::hitloc::is_headshot(hitloc), fatal) as u8;
        }
        let mut f = frame(world);
        let payload = crate::EntityEventPayload {
            number: shot.number,
            attacker_entity_num: gun.shooter.map_or(shot.number, |c| c as i32),
            other_entity_num: match segment.collider {
                Some(
                    ColliderId::EntityDObjBone { owner, .. }
                    | ColliderId::EntityLinkedBrush { owner, .. },
                ) => owner
                    .script_model()
                    .and_then(|id| f.gentity_number(id))
                    .unwrap_or(playerstate_t6::game::ENTITYNUM_NONE),
                Some(ColliderId::Player { client, .. }) => client.0 as i32,
                _ => playerstate_t6::game::ENTITYNUM_NONE,
            },
            event_parm: i32::from(flesh_flags),
            weapon: gun.w,
            correlation: shot.correlation,
            pellet: shot.pellet,
            hand: 0,
            origin: segment.end,
            origin2: segment.start,
            direction: segment.normal,
            surf_type: segment.surf_type,
            surface_flags: segment.surface_flags,
            simulation_flags: u8::from(segment.penetrated),
        };
        let victim = match segment.collider {
            Some(ColliderId::Player { client, .. }) => Some(client),
            _ => None,
        };
        if crate::combat::shows_impact(segment.collider) {
            if let Some(victim) = victim
                && crate::combat::on_shield(segment.collider, bo2)
            {
                let tag = poses
                    .iter()
                    .find(|p| p.client == victim)
                    .and_then(|p| p.shield_tag);
                super::super::riotshield::broadcast_hit(&mut f, t, victim, tag.as_ref(), payload);
            } else if let Some(event) = crate::combat::hit_event(shot.impact_type, false, bo2) {
                let audience = victim.map_or(EventAudience::All, EventAudience::AllExcept);
                f.push_entity_event(t, audience, event, payload);
                if let Some(victim) = victim
                    && let Some(local) = crate::combat::hit_event(shot.impact_type, true, bo2)
                {
                    f.push_entity_event(t, EventAudience::Client(victim), local, payload);
                }
            } else if fx_t6::runtime::impact_table_row(shot.impact_type, false).is_some() {
                f.push_pellet_fx(crate::PelletFxRecord {
                    attacker: shot.number,
                    weapon: gun.w,
                    correlation: shot.correlation,
                    pellet: shot.pellet,
                    hand: 0,
                    start: segment.start,
                    end: segment.end,
                    normal: segment.normal,
                    surf_type: segment.surf_type,
                    surface_flags: segment.surface_flags,
                    flesh_flags,
                });
            }
        }
        if exit || scaled <= 0 {
            continue;
        }
        let means = crate::script_player::means(
            &f,
            DamageSource::Shot(ShotId(shot.correlation)),
            gun.w,
            0,
            false,
        );
        drop(f);
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp streak bullet ({}) struck {:?} at {:?} for {scaled}",
                gun.weapon,
                segment.collider,
                segment.end
            );
        }
        // Bullet_Process: the damage triggers along this stretch of its path.
        if segment.collider.is_some() {
            super::super::damage_triggers::bullet_path_vm(
                vm,
                world,
                attacker.clone(),
                segment.start,
                segment.end,
                scaled,
                means,
            );
        }
        match segment.collider {
            Some(
                ColliderId::EntityDObjBone { owner, .. }
                | ColliderId::EntityLinkedBrush { owner, .. },
            ) => {
                let bone = match segment.collider {
                    Some(ColliderId::EntityDObjBone { bone, .. }) => Some(bone),
                    _ => None,
                };
                let way: [f32; 3] = std::array::from_fn(|i| segment.end[i] - shot.start[i]);
                if let Some(bone) = bone
                    && crate::t5_destructible::apply_hit(
                        &mut frame(world),
                        t,
                        owner,
                        bone,
                        scaled as u32,
                        crate::world_objects::t6_means_from_name(means),
                        segment.end,
                        way,
                        credit,
                        gun.w,
                    )
                {
                    continue;
                }
                if let Some(id) = owner.script_model() {
                    entity_hit(
                        vm,
                        world,
                        &EntHit {
                            model: id,
                            bone,
                            amount: scaled,
                            inflictor: &inflictor,
                            attacker: &attacker,
                            means,
                            weapon: gun.weapon,
                            w: gun.w,
                            point: segment.end,
                            dir: shot.dir,
                        },
                    );
                }
            }
            Some(ColliderId::World { .. }) => {
                if let Some(piece) = glass_piece_from_hit(segment.hit_type, segment.hit_id) {
                    if glass_hit.contains(&piece) {
                        continue;
                    }
                    glass_hit.push(piece);
                    let at_time_ms =
                        i32::try_from(t.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
                    let mut f = frame(world);
                    let mut holdrand = *f.stuck_holdrand_mut();
                    f.world_objects_mut().apply_glass_hit(
                        piece,
                        scaled as u32,
                        at_time_ms,
                        segment.end,
                        shot.dir,
                        &mut || crate::item::random_unit(&mut holdrand),
                    );
                    *f.stuck_holdrand_mut() = holdrand;
                }
            }
            _ => {}
        }
    }
    struck
}

/// A streak bullet's hit on a script entity.
struct EntHit<'a> {
    model: crate::ScriptModelId,
    bone: Option<u16>,
    amount: i32,
    inflictor: &'a Value,
    attacker: &'a Value,
    means: &'a str,
    weapon: &'a str,
    w: u32,
    point: [f32; 3],
    dir: [f32; 3],
}

/// A dog takes it where it lands (its weapon's location multiplier); a
/// streak or another entity that takes damage hears `damage`.
fn entity_hit(vm: &mut Vm<World>, world: &mut World, hit: &EntHit) {
    let Some(n) = world
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == hit.model)
        .map(|(n, _)| *n)
    else {
        return;
    };
    let actor = world
        .resource::<Zm>()
        .actors
        .get(&n)
        .is_some_and(|a| a.alive);
    if actor {
        let part = {
            let f = frame(world);
            hit.bone
                .and_then(|bone| {
                    f.entity_collision_capabilities()
                        .iter()
                        .find(|row| row.owner.script_model() == Some(hit.model))
                        .and_then(|row| row.dobj.as_ref())
                        .and_then(|d| d.current_collision.as_ref())
                        .and_then(|c| c.bones.iter().find(|b| b.bone == bone))
                        .map(|b| b.part_classification)
                })
                .unwrap_or(4)
        };
        let scale = frame(world)
            .combat_facts_for(hit.w)
            .map_or(1.0, |facts| facts.location_scale(part));
        let amount = weapon_t6::hitloc::scaled_damage(hit.amount, scale);
        let place = T6_HITLOC_NAMES
            .get(usize::from(part))
            .copied()
            .unwrap_or("none");
        super::super::actors::damage(
            vm,
            world,
            n,
            hit.inflictor.clone(),
            hit.attacker.clone(),
            amount,
            0,
            hit.means,
            hit.weapon,
            hit.point,
            hit.dir,
            place,
        );
        return;
    }
    if !super::damage_target(world, n) {
        return;
    }
    let Some(obj) = world.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    if !vm.alive(obj) {
        return;
    }
    let (m, w, none) = (vm.string(hit.means), vm.string(hit.weapon), vm.string(""));
    let args = [
        Value::Int(hit.amount),
        hit.attacker.clone(),
        Value::Vec3(gsc_t6::math::normalize(hit.dir)),
        Value::Vec3(hit.point),
        m,
        none.clone(),
        none.clone(),
        none,
        w,
        Value::Int(0),
    ];
    vm.notify_str(world, obj, "damage", &args);
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bullet_collision::AuthorityModelOwner;

    #[test]
    fn a_seated_player_fires_with_player_spread_and_a_turret_alone_with_ai_spread() {
        let s = weapon_t6::pm::spread::SpreadFacts {
            ai_spread: 4.0,
            player_spread: 1.0,
            ..Default::default()
        };
        assert_eq!(cone(&s, true), 1.0);
        assert_eq!(cone(&s, false), 4.0);
    }

    #[test]
    fn only_the_vehicles_own_model_blocks_its_gunner() {
        let own = crate::ScriptModelId(7);
        let bone = |id| {
            Some(ColliderId::EntityDObjBone {
                owner: AuthorityModelOwner::ScriptModel(id),
                bone: 3,
                part_classification: 0,
                surface_flags: 0,
            })
        };
        assert!(struck_model(bone(own), own));
        assert!(!struck_model(bone(crate::ScriptModelId(8)), own));
        assert!(!struck_model(None, own));
    }
}
