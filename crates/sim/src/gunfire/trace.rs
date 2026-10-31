//! Flying each bullet through the map and the players, and what it does to what it meets.

use super::{PelletShot, damage_log, hit_event, on_shield, ricochet, shows_impact};
use crate::shot_falloff::shot_damage_at_range;
use crate::shot_trace::{
    ServerModelSource, ShotRayRequest, ShotPathLeg, HitTargetId, SHOT_WORLD_FILTER, PlayerHitboxPose,
    shot_legs_with_filter, pane_from_hit,
};
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, LifeCount};
use crate::player_ledger::{PlayerSessionStage, EventRecipients};
use crate::sim_parts::{SeatNo, StepNo};
use crate::glass_and_props::{Blow, Strike, T6_GLASS_DAMAGE_MULTIPLIER, pile_harm, shard_blocks_shots};
use playerstate_t6::game::NO_ENTITY_SLOT;
use std::cell::RefCell;
use std::collections::HashMap;
use weapon_t6::combat_facts::GunBattleProfile;
use weapon_t6::penetration::PierceCapability;

/// Traces every bullet in order. A bullet a riot shield turns aside is queued after this tick's own shots, carrying
/// how many times it has bounced.
pub(crate) fn trace_shots(world: &mut StepWorldView, tick: StepNo, emissions: &[PelletShot]) {
    let bo2 = !world.gun_facts().is_empty();
    let mut queue: Vec<(PelletShot, u8)> = emissions.iter().map(|&em| (em, 0)).collect();
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
    fn from_world(world: &StepWorldView) -> Self {
        let objects = world.world_objects();
        Self {
            damage: RefCell::new(objects.pane_harm_entries().into_iter().collect()),
            t6_health: RefCell::new(objects.t6_glass_healths().into_iter().collect()),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn stands(&self, piece: u16) -> bool {
        let pane = u32::from(piece);
        let intact = shard_blocks_shots(self.damage.borrow().get(&pane).copied().unwrap_or(0));
        intact && self.t6_health.borrow().get(&pane).is_none_or(|health| *health > 0.0)
    }

    /// The bullet passed through `piece` ending at `at`; each pane takes the hit once.
    fn take_hit(&self, piece: u16, at: [f32; 3], round: &PelletShot, facts: &GunBattleProfile) {
        let pane = u32::from(piece);
        if self.seen.borrow().contains(&pane) {
            return;
        }
        self.seen.borrow_mut().push(pane);
        let dist = distance(round.spot, at);
        let harm = shot_damage_at_range(facts, dist).max(0) as u32;
        if let Some(health) = self.t6_health.borrow_mut().get_mut(&pane) {
            // BO2 panes only take a hit of 5 or more (before the pane multiplier).
            if *health > 0.0 && harm as f32 >= 5.0 * T6_GLASS_DAMAGE_MULTIPLIER {
                *health -= harm as f32;
            }
            return;
        }
        let mut damage = self.damage.borrow_mut();
        let total = damage.entry(pane).or_insert(0);
        *total = pile_harm(*total, harm);
    }
}

fn distance(from: [f32; 3], till: [f32; 3]) -> f32 {
    let (step_along_x, dy, dz) = (till[0] - from[0], till[1] - from[1], till[2] - from[2]);
    (step_along_x * step_along_x + dy * dy + dz * dz).sqrt()
}

/// What stays the same while the segments of one bullet are applied.
struct Flight<'a> {
    tick: StepNo,
    round: &'a PelletShot,
    facts: &'a GunBattleProfile,
    pen: PierceCapability,
    poses: &'a [PlayerHitboxPose],
    bo2: bool,
    /// The bullet's kind of death ("MOD_RIFLE_BULLET" and the like).
    means: &'static str,
}

/// Flies one bullet. Returns the bullet that carries on if a riot shield turned this one aside.
fn fly_bullet(world: &mut StepWorldView, tick: StepNo, round: &PelletShot, bounces: u8, bo2: bool) -> Option<PelletShot> {
    let facts = world.damage_profile_of(round.gun)?;
    let probe = world.rewind_request_for(round.attacker, tick);
    let end = [
        round.spot[0] + round.way[0] * round.reach_limit,
        round.spot[1] + round.way[1] * round.reach_limit,
        round.spot[2] + round.way[2] * round.reach_limit,
    ];
    let mut pen = world.pierce_facts_for(round.gun);
    if let Some(scale) = round.pen_scale {
        pen.wall_pierce_scale = if scale > 0.0 { scale } else { 1.0 };
    }

    let (poses, entries) = (&probe.players.bodies_then, &probe.entities.rows);
    let glass = GlassBook::from_world(world);
    let note_glass = |piece: u16, at: [f32; 3]| glass.take_hit(piece, at, round, &facts);
    let (legs, _) = shot_legs_with_filter(
        world.collision_solids(),
        world.collision_tree(),
        world.collision_submodel_set(),
        world.collision_triangles(),
        poses,
        entries,
        &ShotRayRequest {
            from: round.spot,
            to: end,
            bits: SHOT_WORLD_FILTER,
            ignore: Some(round.attacker),
            skip_client_hit: round.skip_client_hit,
            ignore_model: None,
        },
        pen,
        world.wall_pierce_depths(),
        &|piece| glass.stands(piece),
        Some(&note_glass),
    );

    // A shot that hits nothing (the sky, or past a shotgun's reach) still flies its tracer to the end of its range:
    // without a normal there is no impact to draw.
    let started_inside = legs.first().is_some_and(|leg| leg.began_inside);
    let impacts = legs.iter().filter(|leg| shows_impact(leg.collider)).count();
    if impacts == 0 && !started_inside {
        world.queue_pellet_effect(crate::ScatterPuff {
            attacker: round.attacker.0 as i32,
            weapon: round.gun,
            correlation: round.shot_serial.0,
            pellet: round.pellet.0,
            hand: round.fist,
            start: round.spot,
            end: legs.last().map_or(end, |leg| leg.to),
            normal: Default::default(),
            surf_type: Default::default(),
            surface_flags: Default::default(),
            body_hit_bits: Default::default(),
        });
    }

    let turned = ricochet::follow_up(world, round, bounces, &legs, poses);
    if legs.is_empty() {
        return turned;
    }

    let flight = Flight {
        tick,
        round,
        facts: &facts,
        pen,
        poses,
        bo2,
        means: crate::client_script::means(world, HarmOrigin::Shot(round.shot_serial), round.gun, 0, false),
    };
    let mut panes_broken: Vec<u32> = Vec::new();
    legs.iter().for_each(|leg| apply_leg(world, &flight, leg, &mut panes_broken));
    // bo2zm: a Black Ops II bullet weapon whose def sets bBulletImpactExplode explodes where its bullet first hits.
    if let Some(eq) = world.equipment_row_raw(round.gun)
        && eq.bullet_impact_explode
        && let Some(struck) = legs
            .iter()
            .find(|s| s.collider.is_some() && s.face_bits & fx_t6::runtime::HIT_ON_WAY_OUT_BIT == 0)
    {
        bullet_impact_blast(world, tick, round, eq, struck.to, struck.face_dir, struck.face_kind);
    }
    turned
}

/// bo2zm: a bBulletImpactExplode bullet's explosion at `origin`: the weapon's explosion effect (its impact type, as a
/// grenade's) and its explosion radius with inner / outer damage, as splash damage.
fn bullet_impact_blast(
    world: &mut StepWorldView,
    tick: StepNo,
    blow: &PelletShot,
    eq: crate::GadgetLiveStats,
    spot: [f32; 3],
    face: [f32; 3],
    surface: u8,
) {
    world.post_entity_event(
        tick,
        EventRecipients::All,
        entity_t6::EventCode::BOMB_BURST,
        crate::HappeningBody {
            number: i32::from(trace_t6::WORLD_ENTITY_SLOT),
            attacker_entity_num: blow.attacker.0 as i32,
            weapon: blow.gun,
            correlation: blow.shot_serial.0,
            origin: spot,
            direction: face,
            surf_type: surface.into(),
            ..Default::default()
        },
    );
    let radius = eq.blast_outer_reach.max(eq.blast_reach_floor).max(0) as f32;
    if radius <= 0.0 || (eq.blast_core_damage <= 0 && eq.blast_rim_damage <= 0) {
        return;
    }
    let blast = crate::harm::ExplosionBlast {
        cone: None,
        origin: spot,
        reach: radius,
        inner_damage: eq.blast_core_damage as f32,
        outer_damage: eq.blast_rim_damage.max(0) as f32,
        gun: blow.gun,
        source: HarmOrigin::Projectile(crate::ThrownId(0)),
        attacker: blow.attacker,
        killer_life: blow.attacker_life,
        cam_ent_since: 0,
    };
    crate::harm::apply_explosion_blast(world, tick, &blast);
}

/// Everything one stretch of a bullet's path does: wounds, effects, triggers, and damage to what it struck.
fn apply_leg(world: &mut StepWorldView, flight: &Flight, leg: &ShotPathLeg, panes_broken: &mut Vec<u32>) {
    let round = flight.round;
    let exit = leg.face_bits & fx_t6::runtime::HIT_ON_WAY_OUT_BIT != 0;
    let dist = distance(round.spot, leg.to);
    let harm = ((shot_damage_at_range(flight.facts, dist) as f32) * leg.harm_multiplier) as i32;
    if damage_log() {
        diag::info!(
            Sim,
            "bo2mp damage: shot {:?} segment surface {} at ({:.1} {:.1} {:.1}) exit {} thickness {:.2} into {:?} multiplier {:.3}",
            round.shot_serial,
            leg.face_kind,
            leg.to[0],
            leg.to[1],
            leg.to[2],
            exit,
            leg.depth,
            leg.collider,
            leg.harm_multiplier
        );
    }

    let flesh = match leg.collider {
        Some(HitTargetId::Player { client, life, hitloc }) if !exit => {
            wound_player(world, flight, leg, (client, life, hitloc), dist, harm)
        }
        _ => 0,
    };

    let payload = event_payload(world, round, leg, flesh);
    if shows_impact(leg.collider) {
        show_impact(world, flight, leg, payload, flesh);
    }

    if exit || !world.emits_state_snapshot() {
        return;
    }
    // Bullet_Process: the damage triggers along this stretch of the bullet's path. A 0-damage bullet (the Paralyzer)
    // still fires them, for at least 1 (bo2zm).
    if leg.collider.is_some() {
        crate::zombie_rt::damage_triggers::bullet_path(
            world.ecs(),
            round.attacker,
            leg.from,
            leg.to,
            harm.max(1),
            flight.means,
        );
    }
    if harm <= 0 {
        return;
    }
    if std::env::var_os("BO2MP_T6_HITLOG").is_some() {
        diag::info!(
            Sim,
            "bo2mp bullet struck {:?} at {:?} surface {} for {scaled}",
            leg.collider,
            leg.to,
            leg.face_kind,
            scaled = harm
        );
    }
    match leg.collider {
        Some(HitTargetId::PosedBone { owner, bone: joint, .. }) => {
            hurt_model(world, flight, leg, owner, Some(joint), harm);
        }
        Some(HitTargetId::AttachedBrush { owner, .. }) => {
            hurt_model(world, flight, leg, owner, None, harm);
        }
        Some(HitTargetId::World { .. }) => break_glass(world, flight, leg, harm, panes_broken),
        Some(HitTargetId::Player { .. }) | None => {}
    }
}

/// A bullet reaches a player: the damage it deals (while the match runs and they live), and the flesh flags the
/// impact effect shows.
fn wound_player(
    world: &mut StepWorldView,
    flight: &Flight,
    leg: &ShotPathLeg,
    (struck, victim_life, hitloc): (SeatNo, LifeCount, u8),
    dist: f32,
    harm: i32,
) -> u8 {
    let round = flight.round;
    if damage_log() {
        diag::info!(
            Sim,
            "bo2mp damage: shot {:?} weapon {} pen type {} x{} hit player {:?} hitloc {:?} at {:.0} base {} pen multiplier {:.3} damage {}",
            round.shot_serial,
            round.gun,
            flight.pen.wall_pierce_class,
            flight.pen.wall_pierce_scale,
            struck,
            hitloc,
            dist,
            shot_damage_at_range(flight.facts, dist),
            leg.harm_multiplier,
            harm
        );
    }
    let living = world.client_meta(struck).is_some_and(|meta| meta.lifecycle == PlayerSessionStage::Alive);
    let lands = world.emits_state_snapshot() && harm > 0 && living;
    let lethal_hit = lands && {
        let attempt = crate::HarmRequest {
            source: HarmOrigin::Shot(round.shot_serial),
            pellet: round.pellet,
            attacker: round.attacker,
            killer_life: round.attacker_life,
            target: struck,
            victim_life_index: victim_life,
            gun: round.gun,
            much: harm,
            cam_ent_since: 0,
            attacker_source_pos: None,
            hit_part: hitloc,
            blast: false,
            passed_through_wall: leg.passed_through_wall,
            impact: false,
        };
        let outcome = crate::harm::land_harm(world, flight.tick, &attempt);
        matches!(outcome, crate::HarmResult::Died(_))
    };
    let head = hud_t6::gamemsg::kill_notice_was_skull(hitloc);
    fx_t6::runtime::flesh_hit_flags(head, lethal_hit) as u8
}

/// The entity event describing where this stretch of the bullet landed.
fn event_payload(
    world: &StepWorldView,
    round: &PelletShot,
    leg: &ShotPathLeg,
    flesh: u8,
) -> crate::HappeningBody {
    let other_entity_num = match leg.collider {
        Some(HitTargetId::PosedBone { owner, .. } | HitTargetId::AttachedBrush { owner, .. }) => {
            let number = owner.scripted_model().and_then(|model| world.game_actor_number(model));
            number.unwrap_or(NO_ENTITY_SLOT)
        }
        Some(HitTargetId::Player { client: who, .. }) => who.0 as i32,
        _ => NO_ENTITY_SLOT,
    };
    crate::HappeningBody {
        number: round.attacker.0 as i32,
        attacker_entity_num: round.attacker.0 as i32,
        other_entity_num,
        event_parm: i32::from(flesh),
        weapon: round.gun,
        correlation: round.shot_serial.0,
        pellet: round.pellet.0,
        hand: round.fist,
        origin: leg.to,
        origin2: leg.from,
        direction: leg.face_dir,
        surf_type: leg.face_kind,
        surface_flags: leg.face_bits,
        wall_pass_bits: u8::from(leg.passed_through_wall),
    }
}

/// Sends the impact to the clients: the shield events for a riot shield, the weapon's hit event otherwise, or a plain
/// effect record when the weapon has an impact table row but no hit event.
fn show_impact(
    sim_view: &mut StepWorldView,
    flight: &Flight,
    leg: &ShotPathLeg,
    detail: crate::HappeningBody,
    flesh: u8,
) {
    let (tick, round, impact_kind_id, bo2) = (flight.tick, flight.round, flight.facts.impact_kind_id, flight.bo2);
    let hit_player = match leg.collider {
        Some(HitTargetId::Player { client: who, .. }) => Some(who),
        _ => None,
    };
    if let Some(hit_player) = hit_player
        && on_shield(leg.collider, bo2)
    {
        // Bullet_ImpactEffect: a hit on a riot shield tells the others where it struck, and tells its bearer where on
        // their shield.
        let tag = flight
            .poses
            .iter()
            .find(|pose| pose.client == hit_player)
            .and_then(|pose| pose.shield_tag);
        crate::zombie_rt::riotshield::broadcast_hit(sim_view, tick, hit_player, tag.as_ref(), detail);
    } else if let Some(seen_by_all) = hit_event(impact_kind_id, false, bo2) {
        let audience = hit_player.map_or(EventRecipients::All, EventRecipients::AllExcept);
        sim_view.post_entity_event(tick, audience, seen_by_all, detail);
        if let Some(hit_player) = hit_player
            && let Some(seen_by_victim) = hit_event(impact_kind_id, true, bo2)
        {
            sim_view.post_entity_event(tick, EventRecipients::Client(hit_player), seen_by_victim, detail);
        }
    } else if fx_t6::runtime::impact_table_row(impact_kind_id, false).is_some() {
        sim_view.queue_pellet_effect(crate::ScatterPuff {
            attacker: round.attacker.0 as i32,
            weapon: round.gun,
            correlation: round.shot_serial.0,
            pellet: round.pellet.0,
            hand: round.fist,
            start: leg.from,
            end: leg.to,
            normal: leg.face_dir,
            surf_type: leg.face_kind,
            surface_flags: leg.face_bits,
            body_hit_bits: flesh,
        });
    }
}

/// A bullet strikes a script entity (a prop, a zombie, a planted shield, a car): a destructible takes the hit first,
/// anything else gets the script damage.
fn hurt_model(
    world: &mut StepWorldView,
    flight: &Flight,
    leg: &ShotPathLeg,
    owner: ServerModelSource,
    bone: Option<u16>,
    harm: i32,
) {
    let round = flight.round;
    // A Black Ops II map's loose prop takes the gun's full damage (BO2's client reads `damage[0]`); the kind of death
    // picks a breakable piece's scale.
    let amount = if crate::staged_props::is_loose_prop(world, owner) {
        shot_damage_at_range(flight.facts, 0.0)
    } else {
        harm
    };
    if let Some(bone) = bone {
        let along = [
            leg.to[0] - round.spot[0],
            leg.to[1] - round.spot[1],
            leg.to[2] - round.spot[2],
        ];
        let taken = crate::staged_props::resolve_destructible_hit(
            world,
            flight.tick,
            owner,
            bone,
            amount as u32,
            crate::glass_and_props::t6_means_from_name(flight.means),
            leg.to,
            along,
            Some(round.attacker),
            round.gun,
        );
        if taken {
            return;
        }
    }
    if let Some(target) = owner.scripted_model() {
        crate::gsc_host::damage_entity(
            world.ecs(),
            &crate::gsc_host::EntityHit {
                target,
                amount: harm,
                attacker: Some(round.attacker),
                means: flight.means,
                gun: round.gun,
                point: leg.to,
                way: round.way,
                bone: bone.map(usize::from),
                bits: 0,
            },
        );
    }
}

/// A bullet strikes world geometry that is a glass pane: Black Ops II panes take the bullet's damage as its kind.
/// Each pane breaks once per bullet however many segments cross it.
fn break_glass(
    world: &mut StepWorldView,
    flight: &Flight,
    leg: &ShotPathLeg,
    harm: i32,
    panes_broken: &mut Vec<u32>,
) {
    let Some(piece) = pane_from_hit(leg.impact_category, leg.struck_entity_num) else {
        return;
    };
    if panes_broken.contains(&piece) {
        return;
    }
    panes_broken.push(piece);
    let for_instant_millis = i32::try_from(flight.tick.0.saturating_mul(crate::SIM_STEP_MILLIS)).unwrap_or(i32::MAX);
    world.strike_glass(Blow {
        pane: piece,
        harm: harm,
        means: crate::glass_and_props::t6_means_from_name(flight.means),
        at_ms: for_instant_millis,
        strike: Strike { at: leg.to, way: flight.round.way },
    });
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
