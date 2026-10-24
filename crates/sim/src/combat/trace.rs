//! Flying each bullet through the map and the players, and what it does to what it meets.

use super::{Emission, damage_log, hit_event, on_shield, ricochet, shows_impact};
use crate::bullet::bullet_damage_at_distance;
use crate::bullet_collision::{
    AuthorityModelOwner, BulletTraceQuery, BulletTraceSegment, ColliderId, MASK_BULLET_WORLD, PlayerCollisionPose,
    bullet_trace_segments_filtered, glass_piece_from_hit,
};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence};
use crate::match_state::{ClientLifecycle, EventAudience};
use crate::world::{ClientId, Tick};
use crate::world_objects::{T6_GLASS_DAMAGE_MULTIPLIER, glass_add_damage, glass_piece_is_solid};
use playerstate_t6::game::ENTITYNUM_NONE;
use std::cell::RefCell;
use std::collections::HashMap;
use weapon_t6::combat_facts::WeaponCombatFacts;
use weapon_t6::penetration::BulletPenFacts;

/// Traces every bullet in order. A bullet a riot shield turns aside is queued after this tick's own shots, carrying
/// how many times it has bounced.
pub(crate) fn phase_trace(world: &mut FrameWorld, tick: Tick, emissions: &[Emission]) {
    let bo2 = !world.gun_facts().is_empty();
    let mut queue: Vec<(Emission, u8)> = emissions.iter().map(|&em| (em, 0)).collect();
    let mut next = 0;
    while let Some(&(bullet, bounces)) = queue.get(next) {
        next += 1;
        if let Some(turned) = fly_bullet(world, tick, &bullet, bounces, bo2) {
            queue.push((turned, bounces + 1));
        }
    }
}

/// The glass state one bullet sees while it flies: panes shot so far hold their damage only for this trace.
struct GlassBook {
    /// Damage per legacy pane.
    damage: RefCell<HashMap<u32, u16>>,
    /// Remaining health per Black Ops II pane, which stands until its health is gone.
    t6_health: RefCell<HashMap<u32, f32>>,
    /// Panes the bullet has already counted.
    seen: RefCell<Vec<u32>>,
}

impl GlassBook {
    fn from_world(world: &FrameWorld) -> Self {
        let objects = world.world_objects();
        Self {
            damage: RefCell::new(objects.glass_damage_pairs().into_iter().collect()),
            t6_health: RefCell::new(objects.t6_glass_healths().into_iter().collect()),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn stands(&self, piece: u16) -> bool {
        let pane = u32::from(piece);
        let intact = glass_piece_is_solid(self.damage.borrow().get(&pane).copied().unwrap_or(0));
        intact && self.t6_health.borrow().get(&pane).is_none_or(|health| *health > 0.0)
    }

    /// The bullet passed through `piece` ending at `at`; each pane takes the hit once.
    fn take_hit(&self, piece: u16, at: [f32; 3], shot: &Emission, facts: &WeaponCombatFacts) {
        let pane = u32::from(piece);
        if self.seen.borrow().contains(&pane) {
            return;
        }
        self.seen.borrow_mut().push(pane);
        let dist = distance(shot.origin, at);
        let scaled = bullet_damage_at_distance(facts, dist).max(0) as u32;
        if let Some(health) = self.t6_health.borrow_mut().get_mut(&pane) {
            // BO2 panes only take a hit of 5 or more (before the pane multiplier).
            if *health > 0.0 && scaled as f32 >= 5.0 * T6_GLASS_DAMAGE_MULTIPLIER {
                *health -= scaled as f32;
            }
            return;
        }
        let mut damage = self.damage.borrow_mut();
        let total = damage.entry(pane).or_insert(0);
        *total = glass_add_damage(*total, scaled);
    }
}

fn distance(from: [f32; 3], to: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// What stays the same while the segments of one bullet are applied.
struct Flight<'a> {
    tick: Tick,
    shot: &'a Emission,
    facts: &'a WeaponCombatFacts,
    pen: BulletPenFacts,
    poses: &'a [PlayerCollisionPose],
    bo2: bool,
    /// The bullet's kind of death ("MOD_RIFLE_BULLET" and the like).
    means: &'static str,
}

/// Flies one bullet. Returns the bullet that carries on if a riot shield turned this one aside.
fn fly_bullet(world: &mut FrameWorld, tick: Tick, shot: &Emission, bounces: u8, bo2: bool) -> Option<Emission> {
    let facts = world.combat_facts_for(shot.weapon)?;
    let query = world.lagcomp_query_for(shot.attacker, tick);
    let end = [
        shot.origin[0] + shot.direction[0] * shot.max_range,
        shot.origin[1] + shot.direction[1] * shot.max_range,
        shot.origin[2] + shot.direction[2] * shot.max_range,
    ];
    let mut pen = world.bullet_pen_facts_for(shot.weapon);
    if let Some(scale) = shot.pen_scale {
        pen.penetrate_multiplier = if scale > 0.0 { scale } else { 1.0 };
    }

    let glass = GlassBook::from_world(world);
    let note_glass = |piece: u16, at: [f32; 3]| glass.take_hit(piece, at, shot, &facts);
    let (segments, _) = bullet_trace_segments_filtered(
        world.clip_brushes(),
        world.clip_bsp(),
        world.clip_cmodels(),
        world.clip_mesh(),
        &query.players.poses,
        &query.entities.rows,
        &BulletTraceQuery {
            start: shot.origin,
            end,
            mask: MASK_BULLET_WORLD,
            ignore: Some(shot.attacker),
            ignore_hit: shot.ignore_hit,
            ignore_model: None,
        },
        pen,
        world.penetration_table(),
        &|piece| glass.stands(piece),
        Some(&note_glass),
    );

    // A shot that hits nothing (the sky, or past a shotgun's reach) still flies its tracer to the end of its range:
    // without a normal there is no impact to draw.
    let started_inside = segments.first().is_some_and(|segment| segment.startsolid);
    let impacts = segments.iter().filter(|segment| shows_impact(segment.collider)).count();
    if impacts == 0 && !started_inside {
        world.push_pellet_fx(crate::PelletFxRecord {
            attacker: shot.attacker.0 as i32,
            weapon: shot.weapon,
            correlation: shot.shot_id.0,
            pellet: shot.pellet.0,
            hand: shot.hand,
            start: shot.origin,
            end: segments.last().map_or(end, |segment| segment.end),
            normal: [0.0; 3],
            surf_type: 0,
            surface_flags: 0,
            flesh_flags: 0,
        });
    }

    let turned = ricochet::follow_up(world, shot, bounces, &segments, &query.players.poses);
    if segments.is_empty() {
        return turned;
    }

    let flight = Flight {
        tick,
        shot,
        facts: &facts,
        pen,
        poses: &query.players.poses,
        bo2,
        means: crate::script_player::means(world, DamageSource::Shot(shot.shot_id), shot.weapon, 0, false),
    };
    let mut panes_broken: Vec<u32> = Vec::new();
    for segment in &segments {
        apply_segment(world, &flight, segment, &mut panes_broken);
    }
    turned
}

/// Everything one stretch of a bullet's path does: wounds, effects, triggers, and damage to what it struck.
fn apply_segment(world: &mut FrameWorld, flight: &Flight, segment: &BulletTraceSegment, panes_broken: &mut Vec<u32>) {
    let shot = flight.shot;
    let exit = segment.surface_flags & fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG != 0;
    let dist = distance(shot.origin, segment.end);
    let scaled = ((bullet_damage_at_distance(flight.facts, dist) as f32) * segment.damage_mult) as i32;
    if damage_log() {
        diag::info!(
            Sim,
            "bo2mp damage: shot {:?} segment surface {} at ({:.1} {:.1} {:.1}) exit {} thickness {:.2} into {:?} multiplier {:.3}",
            shot.shot_id,
            segment.surf_type,
            segment.end[0],
            segment.end[1],
            segment.end[2],
            exit,
            segment.thickness,
            segment.collider,
            segment.damage_mult
        );
    }

    let mut flesh_flags = 0u8;
    if !exit && let Some(ColliderId::Player { client, life, hitloc }) = segment.collider {
        flesh_flags = wound_player(world, flight, segment, (client, life, hitloc), dist, scaled);
    }

    let payload = event_payload(world, shot, segment, flesh_flags);
    if shows_impact(segment.collider) {
        show_impact(world, flight, segment, payload, flesh_flags);
    }

    if exit || !world.publishes_snapshot() || scaled <= 0 {
        return;
    }
    // Bullet_Process: the damage triggers along this stretch of the bullet's path.
    if segment.collider.is_some() {
        crate::t6::damage_triggers::bullet_path(
            world.ecs(),
            shot.attacker,
            segment.start,
            segment.end,
            scaled,
            flight.means,
        );
    }
    if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
        diag::info!(
            Sim,
            "bo2mp bullet struck {:?} at {:?} surface {} for {scaled}",
            segment.collider,
            segment.end,
            segment.surf_type
        );
    }
    match segment.collider {
        Some(ColliderId::EntityDObjBone { owner, bone, .. }) => {
            hurt_model(world, flight, segment, owner, Some(bone), scaled);
        }
        Some(ColliderId::EntityLinkedBrush { owner, .. }) => {
            hurt_model(world, flight, segment, owner, None, scaled);
        }
        Some(ColliderId::World { .. }) => break_glass(world, flight, segment, scaled, panes_broken),
        Some(ColliderId::Player { .. }) | None => {}
    }
}

/// A bullet reaches a player: the damage it deals (while the match runs and they live), and the flesh flags the
/// impact effect shows.
fn wound_player(
    world: &mut FrameWorld,
    flight: &Flight,
    segment: &BulletTraceSegment,
    (victim, victim_life, hitloc): (ClientId, LifeSequence, u8),
    dist: f32,
    scaled: i32,
) -> u8 {
    let shot = flight.shot;
    if damage_log() {
        diag::info!(
            Sim,
            "bo2mp damage: shot {:?} weapon {} pen type {} x{} hit player {:?} hitloc {:?} at {:.0} base {} pen multiplier {:.3} damage {}",
            shot.shot_id,
            shot.weapon,
            flight.pen.penetrate_type,
            flight.pen.penetrate_multiplier,
            victim,
            hitloc,
            dist,
            bullet_damage_at_distance(flight.facts, dist),
            segment.damage_mult,
            scaled
        );
    }
    let alive = world
        .client_meta(victim)
        .is_some_and(|meta| meta.lifecycle == ClientLifecycle::Alive);
    let mut fatal = false;
    if world.publishes_snapshot() && scaled > 0 && alive {
        let attempt = crate::DamageAttempt {
            source: DamageSource::Shot(shot.shot_id),
            pellet: shot.pellet,
            attacker: shot.attacker,
            attacker_life: shot.attacker_life,
            target: victim,
            target_life: victim_life,
            weapon: shot.weapon,
            amount: scaled,
            killcam_entity_start_time: 0,
            inflictor_origin: None,
            hitloc,
            blast: false,
            penetrated: segment.penetrated,
            impact: false,
        };
        let outcome = crate::damage::apply_damage_attempt(world, flight.tick, &attempt);
        fatal = matches!(outcome, crate::DamageOutcome::Died(_));
    }
    let head = hud_t6::gamemsg::obituary_is_headshot(hitloc);
    fx_t6::runtime::flesh_hit_flags(head, fatal) as u8
}

/// The entity event describing where this stretch of the bullet landed.
fn event_payload(
    world: &FrameWorld,
    shot: &Emission,
    segment: &BulletTraceSegment,
    flesh_flags: u8,
) -> crate::EntityEventPayload {
    let other_entity_num = match segment.collider {
        Some(ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. }) => owner
            .script_model()
            .and_then(|model| world.gentity_number(model))
            .unwrap_or(ENTITYNUM_NONE),
        Some(ColliderId::Player { client, .. }) => client.0 as i32,
        _ => ENTITYNUM_NONE,
    };
    crate::EntityEventPayload {
        number: shot.attacker.0 as i32,
        attacker_entity_num: shot.attacker.0 as i32,
        other_entity_num,
        event_parm: i32::from(flesh_flags),
        weapon: shot.weapon,
        correlation: shot.shot_id.0,
        pellet: shot.pellet.0,
        hand: shot.hand,
        origin: segment.end,
        origin2: segment.start,
        direction: segment.normal,
        surf_type: segment.surf_type,
        surface_flags: segment.surface_flags,
        simulation_flags: u8::from(segment.penetrated),
    }
}

/// Sends the impact to the clients: the shield events for a riot shield, the weapon's hit event otherwise, or a plain
/// effect record when the weapon has an impact table row but no hit event.
fn show_impact(
    world: &mut FrameWorld,
    flight: &Flight,
    segment: &BulletTraceSegment,
    payload: crate::EntityEventPayload,
    flesh_flags: u8,
) {
    let (tick, shot, impact_type, bo2) = (flight.tick, flight.shot, flight.facts.impact_type, flight.bo2);
    let victim = match segment.collider {
        Some(ColliderId::Player { client, .. }) => Some(client),
        _ => None,
    };
    if let Some(victim) = victim
        && on_shield(segment.collider, bo2)
    {
        // Bullet_ImpactEffect: a hit on a riot shield tells the others where it struck, and tells its bearer where on
        // their shield.
        let tag = flight
            .poses
            .iter()
            .find(|pose| pose.client == victim)
            .and_then(|pose| pose.shield_tag);
        crate::t6::riotshield::broadcast_hit(world, tick, victim, tag.as_ref(), payload);
    } else if let Some(world_event) = hit_event(impact_type, false, bo2) {
        let audience = victim.map_or(EventAudience::All, EventAudience::AllExcept);
        world.push_entity_event(tick, audience, world_event, payload);
        if let Some(victim) = victim
            && let Some(local_event) = hit_event(impact_type, true, bo2)
        {
            world.push_entity_event(tick, EventAudience::Client(victim), local_event, payload);
        }
    } else if fx_t6::runtime::impact_table_row(impact_type, false).is_some() {
        world.push_pellet_fx(crate::PelletFxRecord {
            attacker: shot.attacker.0 as i32,
            weapon: shot.weapon,
            correlation: shot.shot_id.0,
            pellet: shot.pellet.0,
            hand: shot.hand,
            start: segment.start,
            end: segment.end,
            normal: segment.normal,
            surf_type: segment.surf_type,
            surface_flags: segment.surface_flags,
            flesh_flags,
        });
    }
}

/// A bullet strikes a script entity (a prop, a zombie, a planted shield, a car): a destructible takes the hit first,
/// anything else gets the script damage.
fn hurt_model(
    world: &mut FrameWorld,
    flight: &Flight,
    segment: &BulletTraceSegment,
    owner: AuthorityModelOwner,
    bone: Option<u16>,
    scaled: i32,
) {
    let shot = flight.shot;
    // A Black Ops II map's loose prop takes the gun's full damage (BO2's client reads `damage[0]`); the kind of death
    // picks a breakable piece's scale.
    let amount = if crate::t5_destructible::is_loose_prop(world, owner) {
        bullet_damage_at_distance(flight.facts, 0.0)
    } else {
        scaled
    };
    if let Some(bone) = bone {
        let along = [
            segment.end[0] - shot.origin[0],
            segment.end[1] - shot.origin[1],
            segment.end[2] - shot.origin[2],
        ];
        let absorbed = crate::t5_destructible::apply_hit(
            world,
            flight.tick,
            owner,
            bone,
            amount as u32,
            crate::world_objects::t6_means_from_name(flight.means),
            segment.end,
            along,
            Some(shot.attacker),
            shot.weapon,
        );
        if absorbed {
            return;
        }
    }
    if let Some(target) = owner.script_model() {
        crate::script::damage_entity(
            world.ecs(),
            &crate::script::EntityHit {
                target,
                amount: scaled,
                attacker: Some(shot.attacker),
                means: flight.means,
                weapon: shot.weapon,
                point: segment.end,
                dir: shot.direction,
                bone: bone.map(usize::from),
                flags: 0,
            },
        );
    }
}

/// A bullet strikes world geometry that is a glass pane: Black Ops II panes take the bullet's damage as its kind.
/// Each pane breaks once per bullet however many segments cross it.
fn break_glass(
    world: &mut FrameWorld,
    flight: &Flight,
    segment: &BulletTraceSegment,
    scaled: i32,
    panes_broken: &mut Vec<u32>,
) {
    let Some(piece) = glass_piece_from_hit(segment.hit_type, segment.hit_id) else {
        return;
    };
    if panes_broken.contains(&piece) {
        return;
    }
    panes_broken.push(piece);
    let at_time_ms = i32::try_from(flight.tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
    let mut holdrand = *world.stuck_holdrand_mut();
    if world.world_objects().is_t6_glass(piece) {
        let means = crate::world_objects::t6_means_from_name(flight.means);
        world.world_objects_mut().apply_t6_glass_damage(
            piece,
            scaled,
            means,
            at_time_ms,
            segment.end,
            flight.shot.direction,
            &mut || crate::item::random_unit(&mut holdrand),
        );
    }
    *world.stuck_holdrand_mut() = holdrand;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_stands_until_its_health_or_damage_is_spent() {
        let book = GlassBook {
            damage: RefCell::new(HashMap::new()),
            t6_health: RefCell::new(HashMap::from([(4, 100.0), (5, 0.0)])),
            seen: RefCell::new(Vec::new()),
        };
        assert!(book.stands(4));
        assert!(!book.stands(5));
        // A pane with no health row is judged by its legacy damage alone.
        assert!(book.stands(9));
    }

    #[test]
    fn distance_is_euclidean() {
        assert_eq!(distance([0.0; 3], [3.0, 4.0, 12.0]), 13.0);
    }
}
