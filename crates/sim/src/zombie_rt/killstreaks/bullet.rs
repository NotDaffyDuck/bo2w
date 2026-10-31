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
use gsc_t6::{ScriptValue, Vm};

use super::super::{Zm, frame, hitloc_name, tick};
use super::streaks;
use crate::shot_falloff::shot_damage_at_range;
use crate::shot_trace::{
    ServerModelSource, ShotRayRequest, ShotPathLeg, HitTargetId, HITLOC_SHIELD,
    SHOT_WORLD_FILTER, shot_legs_with_filter, pane_from_hit,
};
use crate::match_ids::{HarmOrigin, MatchRandom, ShotId};
use crate::player_ledger::{PlayerSessionStage, EventRecipients};
use crate::sim_parts::ClientId;

/// Pellet effects come from the entry row of the impact table, not the exit row.
const AS_EXIT: bool = false;
use crate::glass_and_props::{Blow, Strike};

/// How far ahead of a gunner's muzzle his own vehicle must be clear for
/// his gun to fire (`VEH_FireGunnerWeapon`).
pub(super) const HULL_CLEAR: f32 = 500.0;

/// Part classification a model hit counts as when its bone has no row.
const FALLBACK_PART: u8 = 4;

/// A streak gun firing.
pub(super) struct Gun<'a> {
    /// The turret or vehicle.
    pub(super) ent: u32,
    /// The player sitting at it, when one is: he is the attacker, and the
    /// gun spreads as his (`playerSpread`).
    pub(super) shooter: Option<u32>,
    /// The player it fires for: the damage triggers, breakables and streaks
    /// it hits are his hits.
    pub(super) owner: Option<u32>,
    pub(super) weapon: &'a str,
    pub(super) w: u32,
}

/// The gun's spread (degrees): `playerSpread` with a player at it, else
/// `aiSpread`; none on content that has no Black Ops II weapon facts.
fn spread(sim: &mut World, gun: &Gun) -> f32 {
    let f = frame(sim);
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
    sim: &mut World,
    gun: &Gun,
    from_pt: [f32; 3],
    dir: [f32; 3],
) -> Vec<u32> {
    let t = tick(sim);
    let (model, number) = sim
        .resource::<Zm>()
        .presences
        .by_ent
        .get(&gun.ent)
        .map(|pt| (pt.id, pt.num))
        .unzip();
    let number = number.unwrap_or(playerstate_t6::game::NO_ENTITY_SLOT);
    let correlation = {
        let mut st = streaks(sim);
        st.shots = st.shots.wrapping_add(1);
        st.shots
    };
    let Some(facts) = frame(sim).damage_profile_of(gun.w) else {
        return Vec::new();
    };
    let spread = spread(sim, gun);
    let pellets = facts.shot_pellet_total().clamp(1, 32) as u16;
    let range = facts.shot_reach();
    let angles = super::aim_angles(dir);
    let mut rng = MatchRandom::new(
        (u64::from(t.0) << 32) ^ (u64::from(gun.ent) << 16) ^ u64::from(correlation),
    );
    let mut struck = Vec::new();
    for pellet in 0..pellets {
        let d = crate::gunfire::pellet_heading(angles, spread, &mut rng);
        // Each stretch of its flight: a riot shield that turns it sends it
        // on past the shield's bearer, the gun's full range from there.
        let (mut from, mut way, mut reach, mut past) = (from_pt, d, range, None);
        let mut bounces = 0u8;
        loop {
            let end: [f32; 3] = std::array::from_fn(|i| from[i] + way[i] * reach);
            let (legs, poses) = trace(sim, gun, model, from, end, past);
            let shot = Shot {
                gun,
                num: number,
                shot_serial: correlation,
                pellet_idx: pellet,
                from_pt: from,
                end,
                dir: way,
                impact: facts.impact_kind_id,
            };
            for v in hits(vm, sim, &shot, &legs, &poses) {
                if !struck.contains(&v) {
                    struck.push(v);
                }
            }
            let seed = (u64::from(correlation) << 24) ^ (u64::from(pellet) << 8) ^ u64::from(bounces);
            let Some(bounce) = crate::gunfire::shield_bounce(
                &frame(sim),
                gun.w,
                way,
                seed,
                bounces,
                &legs,
                &poses,
            ) else {
                break;
            };
            (from, way, reach, past) = (
                bounce.origin,
                bounce.direction,
                bounce.reach,
                Some(bounce.bearer),
            );
            bounces += 1;
        }
    }
    struck
}

/// The model a collider belongs to, when it is a model's bone or a brush
/// linked to one.
fn owner_of(struck: Option<HitTargetId>) -> Option<ServerModelSource> {
    match struck {
        Some(HitTargetId::PosedBone { owner, .. }) => Some(owner),
        Some(HitTargetId::AttachedBrush { owner, .. }) => Some(owner),
        _ => None,
    }
}

/// The player a collider is, if it is one.
fn player_of(collider: Option<HitTargetId>) -> Option<ClientId> {
    if let Some(HitTargetId::Player { client, .. }) = collider {
        Some(client)
    } else {
        None
    }
}

/// Whether a gunner's gun has its own vehicle in the way: the `HULL_CLEAR`
/// units ahead of its muzzle cross the vehicle's own model.
pub(super) fn own_hull_blocks(sim: &mut World, en: u32, from_pt: [f32; 3], aim: [f32; 3]) -> bool {
    let Some(id) = sim.resource::<Zm>().presences.by_ent.get(&en).map(|p| p.id) else {
        return false;
    };
    let f = frame(sim);
    let caps = f.entity_hit_support();
    let geoms: Vec<_> = caps
        .iter()
        .filter(|row| row.owner.scripted_model() == Some(id))
        .map(|row| row.trace_geom())
        .collect();
    if geoms.is_empty() {
        return false;
    }
    let end: [f32; 3] = std::array::from_fn(|i| from_pt[i] + aim[i] * HULL_CLEAR);
    let nobody: Option<ClientId> = None;
    let mask = SHOT_WORLD_FILTER;
    let query = ShotRayRequest {
        from: from_pt,
        to: end,
        bits: mask,
        ignore: nobody,
        skip_client_hit: nobody,
        ignore_model: None,
    };
    let (legs, _) = shot_legs_with_filter(
        f.collision_solids(),
        f.collision_tree(),
        f.collision_submodel_set(),
        f.collision_triangles(),
        &[],
        &geoms,
        &query,
        f.pierce_facts_for(0),
        f.wall_pierce_depths(),
        &|_| true,
        None,
    );
    legs.iter().any(|leg| struck_model(leg.collider, id))
}

/// Whether a bullet's stretch ended on that script model.
fn struck_model(collider: Option<HitTargetId>, id: crate::ScriptModelId) -> bool {
    owner_of(collider).is_some_and(|owner| owner.scripted_model() == Some(id))
}

/// One pellet's path: what it passes through and stops in, past the gun
/// itself, the player at it and `past` (the bearer of a shield it bounced
/// off); and the players as they stood.
fn trace(
    sim: &mut World,
    gun: &Gun,
    model: Option<crate::ScriptModelId>,
    from_pt: [f32; 3],
    end: [f32; 3],
    past: Option<ClientId>,
) -> (
    Vec<ShotPathLeg>,
    Vec<crate::shot_trace::PlayerHitboxPose>,
) {
    let f = frame(sim);
    let poses = f.alive_collision_poses();
    let caps = f.entity_hit_support();
    let geoms: Vec<_> = caps
        .iter()
        .map(|row| row.trace_geom())
        .collect();
    let facts = f.damage_profile_of(gun.w);
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
        let scaled = shot_damage_at_range(&facts, dist(from_pt, at)).max(0);
        if let Some(health) = glass_health.borrow_mut().get_mut(&pane).filter(|h| **h > 0.0) {
            *health = glass_t6::server::health_after_hit(
                *health,
                scaled,
                crate::glass_and_props::T6_GLASS_DAMAGE_MULTIPLIER,
            );
        }
    };
    let mask = SHOT_WORLD_FILTER;
    let query = ShotRayRequest {
        from: from_pt,
        to: end,
        bits: mask,
        ignore: gun.shooter.map(ClientId),
        skip_client_hit: past,
        ignore_model: model,
    };
    let (legs, _) = shot_legs_with_filter(
        f.collision_solids(),
        f.collision_tree(),
        f.collision_submodel_set(),
        f.collision_triangles(),
        &poses,
        &geoms,
        &query,
        f.pierce_facts_for(gun.w),
        f.wall_pierce_depths(),
        &|piece| {
            glass_health
                .borrow()
                .get(&u32::from(piece))
                .is_none_or(|health| *health > 0.0)
        },
        Some(&on_glass_hit),
    );
    (legs, poses)
}

/// One pellet in flight.
struct Shot<'a, 'b> {
    gun: &'a Gun<'b>,
    /// The gun's entity number in the snapshot.
    num: i32,
    shot_serial: u32,
    pellet_idx: u16,
    from_pt: [f32; 3],
    end: [f32; 3],
    dir: [f32; 3],
    impact: i32,
}

impl Shot<'_, '_> {
    /// The pellet's tracer record over `start..end`, with no impact detail yet.
    fn tracer(&self, from_pt: [f32; 3], finish: [f32; 3]) -> crate::PelletFxRecord {
        crate::PelletFxRecord {
            attacker: self.num,
            weapon: self.gun.w,
            correlation: self.shot_serial,
            pellet: self.pellet_idx,
            start: from_pt,
            end: finish,
            ..Default::default()
        }
    }
}

/// What a pellet's path strikes, each in turn: its effects and its damage.
/// The players it hit.
fn hits(
    vm: &mut Vm<World>,
    sim: &mut World,
    shot: &Shot,
    legs: &[ShotPathLeg],
    poses: &[crate::shot_trace::PlayerHitboxPose],
) -> Vec<u32> {
    let gun = shot.gun;
    let t = tick(sim);
    let bo2 = !frame(sim).gun_facts().is_empty();
    let mut struck = Vec::new();
    let shown = legs
        .iter()
        .filter(|s| crate::gunfire::shows_impact(s.collider))
        .count();
    let began_in_solid = legs.first().is_some_and(|s| s.began_inside);
    // Nothing shown (the sky, out of reach): its tracer still flies on.
    if shown == 0 && !began_in_solid {
        let reach_end = legs.last().map_or(shot.end, |s| s.to);
        frame(sim).queue_pellet_effect(shot.tracer(shot.from_pt, reach_end));
    }
    let inflictor = super::craft::world_obj(sim, gun.ent);
    let culprit = gun
        .shooter
        .map_or_else(|| inflictor.clone(), |c| super::craft::world_obj(sim, c));
    let credit = gun.shooter.or(gun.owner).map(ClientId);
    let armor_piercing = {
        let f = frame(sim);
        !f.gun_facts().is_empty() && f.gun_facts().get(gun.w).core.armor_piercing
    };
    let mut glass_hit: Vec<u32> = Vec::new();
    for leg in legs {
        let exit = leg.face_bits & fx_t6::runtime::HIT_ON_WAY_OUT_BIT != 0;
        let dealt = {
            let f = frame(sim);
            f.damage_profile_of(gun.w).map_or(0, |known| {
                ((shot_damage_at_range(&known, dist(shot.from_pt, leg.to)) as f32)
                    * leg.harm_multiplier) as i32
            })
        };
        let mut flesh = 0u8;
        if !exit && let Some(HitTargetId::Player { client, hitloc, .. }) = leg.collider {
            let mut died = false;
            let alive = matches!(
                frame(sim).client_meta(client),
                Some(m) if m.lifecycle == PlayerSessionStage::Alive
            );
            if alive && dealt > 0 {
                let (qty, means, flags) = {
                    let f = frame(sim);
                    // A hit the riot shield stopped carries the bullet's
                    // whole damage to the scripts.
                    let scale = if hitloc == HITLOC_SHIELD {
                        1.0
                    } else {
                        f.damage_profile_of(gun.w)
                            .map_or(1.0, |known| known.location_scale(hitloc))
                    };
                    let means = crate::client_script::means(
                        &f,
                        HarmOrigin::Shot(ShotId(shot.shot_serial)),
                        gun.w,
                        hitloc,
                        false,
                    );
                    let flags = if leg.passed_through_wall {
                        crate::client_script::IDFLAGS_PENETRATION
                    } else {
                        0
                    } | if armor_piercing {
                        crate::client_script::IDFLAGS_NO_ARMOR
                    } else {
                        0
                    };
                    (weapon_t6::hitloc::scaled_damage(dealt, scale), means, flags)
                };
                if qty > 0 {
                    let place = hitloc_name(hitloc);
                    super::super::natives_ai::player_damage(
                        vm,
                        sim,
                        client.0,
                        inflictor.clone(),
                        culprit.clone(),
                        qty,
                        flags,
                        means,
                        gun.weapon,
                        leg.to,
                        shot.dir,
                        place,
                    );
                    if !struck.contains(&client.0) {
                        struck.push(client.0);
                    }
                    died = frame(sim)
                        .avatar(client)
                        .is_none_or(|ps| ps.health <= 0);
                }
            }
            flesh = fx_t6::runtime::flesh_hit_flags(weapon_t6::hitloc::is_headshot(hitloc), died) as u8;
        }
        let mut f = frame(sim);
        let victim = player_of(leg.collider);
        let other_entity_num = if let Some(owner) = owner_of(leg.collider) {
            let model_id = owner.scripted_model();
            model_id
                .and_then(|id| f.game_actor_number(id))
                .unwrap_or(playerstate_t6::game::NO_ENTITY_SLOT)
        } else if let Some(client) = victim {
            client.0 as i32
        } else {
            playerstate_t6::game::NO_ENTITY_SLOT
        };
        let event_args = crate::EntityEventPayload {
            number: shot.num,
            attacker_entity_num: gun.shooter.map_or(shot.num, |c| c as i32),
            other_entity_num,
            event_parm: i32::from(flesh),
            weapon: gun.w,
            correlation: shot.shot_serial,
            pellet: shot.pellet_idx,
            origin: leg.to,
            origin2: leg.from,
            direction: leg.face_dir,
            surf_type: leg.face_kind,
            surface_flags: leg.face_bits,
            wall_pass_bits: u8::from(leg.passed_through_wall),
            ..Default::default()
        };
        if crate::gunfire::shows_impact(leg.collider) {
            let shield_bearer = victim.filter(|_| crate::gunfire::on_shield(leg.collider, bo2));
            if let Some(bearer) = shield_bearer {
                let tag = poses
                    .iter()
                    .find(|p| p.client == bearer)
                    .and_then(|p| p.shield_tag);
                super::super::riotshield::broadcast_hit(&mut f, t, bearer, tag.as_ref(), event_args);
            } else if let Some(event) = crate::gunfire::hit_event(shot.impact, false, bo2) {
                let audience = victim.map_or(EventRecipients::All, EventRecipients::AllExcept);
                f.post_entity_event(t, audience, event, event_args);
                let own_view = victim.zip(crate::gunfire::hit_event(shot.impact, true, bo2));
                if let Some((who, local)) = own_view {
                    f.post_entity_event(t, EventRecipients::Client(who), local, event_args);
                }
            } else if fx_t6::runtime::impact_table_row(shot.impact, AS_EXIT).is_some() {
                f.queue_pellet_effect(crate::PelletFxRecord {
                    normal: leg.face_dir,
                    surf_type: leg.face_kind,
                    surface_flags: leg.face_bits,
                    body_hit_bits: flesh,
                    ..shot.tracer(leg.from, leg.to)
                });
            }
        }
        if exit || dealt <= 0 {
            continue;
        }
        let means = crate::client_script::means(
            &f,
            HarmOrigin::Shot(ShotId(shot.shot_serial)),
            gun.w,
            0,
            false,
        );
        drop(f);
        if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
            diag::info!(
                Sim,
                "bo2mp streak bullet ({}) struck {:?} at {:?} for {dealt}",
                gun.weapon,
                leg.collider,
                leg.to
            );
        }
        // Bullet_Process: the damage triggers along this stretch of its path.
        if leg.collider.is_some() {
            super::super::damage_triggers::bullet_path_vm(
                vm,
                sim,
                culprit.clone(),
                leg.from,
                leg.to,
                dealt,
                means,
            );
        }
        if let Some(owner) = owner_of(leg.collider) {
            let bone = match leg.collider {
                Some(HitTargetId::PosedBone { bone, .. }) => Some(bone),
                _ => None,
            };
            let way: [f32; 3] = std::array::from_fn(|i| leg.to[i] - shot.from_pt[i]);
            let broke_apart = bone.is_some_and(|bone| {
                crate::staged_props::resolve_destructible_hit(
                    &mut frame(sim),
                    t,
                    owner,
                    bone,
                    dealt as u32,
                    crate::glass_and_props::t6_means_from_name(means),
                    leg.to,
                    way,
                    credit,
                    gun.w,
                )
            });
            if broke_apart {
                continue;
            }
            if let Some(id) = owner.scripted_model() {
                entity_hit(
                    vm,
                    sim,
                    &EntHit {
                        model: id,
                        bone,
                        dmg_amt: dealt,
                        inflictor: &inflictor,
                        attacker: &culprit,
                        means,
                        weapon: gun.weapon,
                        w: gun.w,
                        spot: leg.to,
                        dir: shot.dir,
                    },
                );
            }
        } else if matches!(leg.collider, Some(HitTargetId::World { .. })) {
            if let Some(piece) = pane_from_hit(leg.impact_category, leg.struck_entity_num) {
                if glass_hit.contains(&piece) {
                    continue;
                }
                glass_hit.push(piece);
                let stamp_ms =
                    i32::try_from(t.0.saturating_mul(crate::SIM_STEP_MILLIS)).unwrap_or(i32::MAX);
                let strike = Strike { at: leg.to, way: shot.dir };
                frame(sim).strike_glass(Blow::bullet(piece, dealt as u32, stamp_ms, strike));
            }
        }
    }
    struck
}

/// A streak bullet's hit on a script entity.
struct EntHit<'a> {
    model: crate::ScriptModelId,
    bone: Option<u16>,
    dmg_amt: i32,
    inflictor: &'a ScriptValue,
    attacker: &'a ScriptValue,
    means: &'a str,
    weapon: &'a str,
    w: u32,
    spot: [f32; 3],
    dir: [f32; 3],
}

/// A dog takes it where it lands (its weapon's location multiplier); a
/// streak or another entity that takes damage hears `damage`.
fn entity_hit(vm: &mut Vm<World>, game: &mut World, hit: &EntHit) {
    let Some(n) = game
        .resource::<Zm>()
        .presences
        .by_ent
        .iter()
        .find(|(_, s)| s.id == hit.model)
        .map(|(n, _)| *n)
    else {
        return;
    };
    let actor = game
        .resource::<Zm>()
        .actors
        .get(&n)
        .is_some_and(|a| a.living);
    if actor {
        let part = {
            let f = frame(game);
            hit.bone
                .and_then(|bone| {
                    f.entity_hit_support()
                        .iter()
                        .find(|row| row.owner.scripted_model() == Some(hit.model))
                        .and_then(|row| row.dobj.as_ref())
                        .and_then(|d| d.live_hitbox.as_ref())
                        .and_then(|c| c.bones.iter().find(|b| b.bone == bone))
                        .map(|b| b.body_part_category)
                })
                .unwrap_or(FALLBACK_PART)
        };
        let scale = frame(game)
            .damage_profile_of(hit.w)
            .map_or(1.0, |facts| facts.location_scale(part));
        let amount = weapon_t6::hitloc::scaled_damage(hit.dmg_amt, scale);
        let place = hitloc_name(part);
        super::super::actors::damage(
            vm,
            game,
            n,
            hit.inflictor.clone(),
            hit.attacker.clone(),
            amount,
            0,
            hit.means,
            hit.weapon,
            hit.spot,
            hit.dir,
            place,
        );
        return;
    }
    if !super::damage_target(game, n) {
        return;
    }
    let Some(obj) = game.resource::<Zm>().ents.get(&n).and_then(|e| e.obj) else {
        return;
    };
    if !vm.alive(obj) {
        return;
    }
    let (m, w, none) = (vm.string(hit.means), vm.string(hit.weapon), vm.string(""));
    let call_args = [
        ScriptValue::Int(hit.dmg_amt),
        hit.attacker.clone(),
        ScriptValue::Vec3(gsc_t6::math::normalize(hit.dir)),
        ScriptValue::Vec3(hit.spot),
        m,
        none.clone(),
        none.clone(),
        none,
        w,
        ScriptValue::Int(0),
    ];
    vm.notify_str(game, obj, "damage", &call_args);
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Some(HitTargetId::PosedBone {
                owner: ServerModelSource::ScriptModel(id),
                bone: 3,
                body_part_category: Default::default(),
                surface_flags: Default::default(),
            })
        };
        assert!(struck_model(bone(own), own));
        assert!(!struck_model(bone(crate::ScriptModelId(8)), own));
        assert!(!struck_model(None, own));
    }
}
