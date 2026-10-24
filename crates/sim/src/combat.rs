use crate::bullet::bullet_damage_at_distance;
use crate::bullet_collision::{
    BulletTraceQuery, BulletTraceSegment, ColliderId, EntityCollisionEpoch,
    EntityCollisionTraceGeom, HITLOC_SHIELD, HistorySampleVerdict, MASK_BULLET_WORLD,
    PlayerCollisionPose, bullet_trace_segments_filtered, glass_piece_from_hit,
};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, MatchRng, PelletId, ShotId};
use crate::match_state::{ClientLifecycle, EventAudience};
use crate::world::{ClientId, Tick};
use crate::world_objects::glass_piece_is_solid;
use crate::world_objects::glass_add_damage;
use playerstate_t6::game::ENTITYNUM_NONE;
use std::cell::RefCell;
use std::collections::HashMap;
use weapon_t6::melee_trace::{MELEE_TRACE_OFFSETS, PLAYER_MELEE_HEIGHT_DEFAULT, PLAYER_MELEE_WIDTH_DEFAULT, melee_trace_count, melee_trace_end};
use weapon_t6::damage::{FireWeaponKind, fire_weapon_kind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcceptedShot {
    pub shot_id: ShotId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,

    pub hand: u8,
    pub weapon: u32,
    pub ammo_used: i32,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    pub ads_frac: f32,

    pub view_height_current: f32,

    pub aim_spread_scale: f32,

    pub perks0: u32,
    pub combat_seed: u32,
    pub owner_velocity: [f32; 3],
    pub spread_degrees: f32,
    /// Bullet_FirePenetrate's depth scale for this shot (BO2 brain): zero leaves the
    /// depths as the table has them. `None` keeps the weapon's own multiplier.
    pub pen_scale: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emission {
    pub shot_id: ShotId,
    pub pellet: PelletId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub hand: u8,
    pub weapon: u32,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_range: f32,
    pub base_damage: i32,
    pub pen_scale: Option<f32>,
    /// A player the bullet passes through: the one whose riot shield it bounced off.
    pub ignore_hit: Option<ClientId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerCollisionRepresentation {
    StandingAabbV1,
    PosedBonesV1,
}

impl PlayerCollisionRepresentation {
    pub const fn dump_label(self) -> &'static str {
        match self {
            Self::StandingAabbV1 => "aabb",
            Self::PosedBonesV1 => "bones",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityClipKind {
    CollTris,
    BoneBoxes,
    LinkedBrush,
}

impl EntityClipKind {
    pub const fn dump_label(self) -> &'static str {
        match self {
            Self::CollTris => "colltris",
            Self::BoneBoxes => "boxes",
            Self::LinkedBrush => "brush",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShotCollisionGeometry {
    Miss,
    World,
    Player {
        representation: PlayerCollisionRepresentation,
        history: HistorySampleVerdict,
    },
    Entity {
        epoch: EntityCollisionEpoch,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShotCollisionVerdict {
    pub shot_id: ShotId,
    pub pellet: PelletId,
    pub attacker: ClientId,
    pub geometry: ShotCollisionGeometry,
    pub terminal: Option<ColliderId>,
    pub startsolid: bool,
    pub bone_center: Option<[f32; 3]>,
    pub bone_half_size: Option<[f32; 3]>,
    pub xmodel_contents: Option<u32>,
    pub model_key: Option<String>,

    pub end: Option<[f32; 3]>,

    pub entity_clip: Option<EntityClipKind>,

    pub impact_n: u32,

    pub event_n: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracePhaseOutput {
    pub shot_verdicts: Vec<ShotCollisionVerdict>,
}

pub(crate) fn advance_weapon_command(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    cmd: playerstate_t6::game::UserCmd,
    msec: i32,
) -> Vec<AcceptedShot> {
    // bo2mp: the gun is BO2's own brain (t6_gun), on BO2's weapon facts. A world without
    // them (no T6 map) has no gun.
    if world.gun_facts().is_empty()
        || !world
            .client_meta(id)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive)
    {
        return Vec::new();
    }
    crate::t6_gun::advance(world, tick, id, &cmd, msec.clamp(1, 200))
}

pub fn spread_pellet_direction(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut MatchRng,
) -> [f32; 3] {
    spread_direction_on_plane(angles, spread_degrees, rng, 1.0)
}

/// bo2mp: `BO2MP_DAMAGE_LOG` set = log every bullet that reaches a player:
/// its damage at that range, the multiplier left after what it passed
/// through, and the damage dealt (test aid).
pub(crate) fn damage_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_DAMAGE_LOG").is_some())
}

/// bo2zm: `BO2MP_SHOT_LOG` set = log every shot's aim (test aid).
pub(crate) fn shot_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BO2MP_SHOT_LOG").is_some())
}

pub fn spread_direction_on_plane(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut MatchRng,
    plane: f32,
) -> [f32; 3] {
    let (forward, right, up) = game_math::angle_vectors(angles);
    if spread_degrees <= 0.0 {
        return forward;
    }
    let unit = |draw: u32| draw as f32 / u32::MAX as f32;
    let radius = unit(rng.next_u32());
    let theta = unit(rng.next_u32()) * core::f32::consts::TAU;
    let lateral = plane * spread_degrees.to_radians().tan() * radius;
    let q = [
        plane * forward[0] + lateral * (theta.cos() * right[0] + theta.sin() * up[0]),
        plane * forward[1] + lateral * (theta.cos() * right[1] + theta.sin() * up[1]),
        plane * forward[2] + lateral * (theta.cos() * right[2] + theta.sin() * up[2]),
    ];
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if len > 0.0 {
        [q[0] / len, q[1] / len, q[2] / len]
    } else {
        forward
    }
}

pub(crate) fn phase_emit(world: &FrameWorld, shots: &[AcceptedShot]) -> Vec<Emission> {
    let mut out = Vec::new();
    for shot in shots {
        let Some(facts) = world.combat_facts_for(shot.weapon) else {
            continue;
        };

        match fire_weapon_kind(facts.weap_type, facts.weap_class) {
            Some(FireWeaponKind::Bullet) => {
                if facts.damage <= 0 || facts.bullet_range() <= 0.0 {
                    continue;
                }
            }
            _ => continue,
        }
        let mut rng = MatchRng::new(shot.combat_seed as u64);
        let pellet_count = facts.pellet_count().clamp(1, u16::MAX as i32) as u16;
        for pellet in 0..pellet_count {
            out.push(Emission {
                shot_id: shot.shot_id,
                pellet: PelletId(pellet),
                attacker: shot.attacker,
                attacker_life: shot.attacker_life,
                hand: shot.hand,
                weapon: shot.weapon,
                origin: shot.origin,
                direction: spread_pellet_direction(shot.angles, shot.spread_degrees, &mut rng),
                max_range: facts.bullet_range(),
                base_damage: facts.damage,
                pen_scale: shot.pen_scale,
                ignore_hit: None,
            });
        }
    }
    out
}

pub(crate) fn phase_trace(
    world: &mut FrameWorld,
    tick: Tick,
    emissions: &[Emission],
) -> TracePhaseOutput {
    let mut output = TracePhaseOutput::default();
    // A bullet a riot shield turns aside flies on as a new one (BulletRicochet), traced after
    // this tick's shots; each carries how many times it has bounced.
    let mut queue: Vec<(Emission, u8)> = emissions.iter().map(|&em| (em, 0)).collect();
    let mut next = 0;
    let bo2 = !world.gun_facts().is_empty();
    while let Some(&(emission, bounces)) = queue.get(next) {
        next += 1;
        let em = &emission;
        let Some(facts) = world.combat_facts_for(em.weapon) else {
            continue;
        };
        let query = world.lagcomp_query_for(em.attacker, tick);
        let end = [
            em.origin[0] + em.direction[0] * em.max_range,
            em.origin[1] + em.direction[1] * em.max_range,
            em.origin[2] + em.direction[2] * em.max_range,
        ];
        let mut pen = world.bullet_pen_facts_for(em.weapon);
        if let Some(scale) = em.pen_scale {
            pen.penetrate_multiplier = if scale > 0.0 { scale } else { 1.0 };
        }
        let glass_damage = RefCell::new(
            world
                .world_objects()
                .glass_damage_pairs()
                .into_iter()
                .collect::<HashMap<u32, u16>>(),
        );
        // Black Ops II panes stand until their health is gone (a 5+ hit
        // takes its damage, less does nothing).
        let t6_health = RefCell::new(
            world
                .world_objects()
                .t6_glass_healths()
                .into_iter()
                .collect::<HashMap<u32, f32>>(),
        );
        let glass_seen = RefCell::new(Vec::<u32>::new());
        let on_glass_hit = |piece: u16, end: [f32; 3]| {
            let pane = u32::from(piece);
            if glass_seen.borrow().contains(&pane) {
                return;
            }
            glass_seen.borrow_mut().push(pane);
            let dist = {
                let dx = end[0] - em.origin[0];
                let dy = end[1] - em.origin[1];
                let dz = end[2] - em.origin[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            };
            let scaled = bullet_damage_at_distance(&facts, dist).max(0) as u32;
            if let Some(health) = t6_health.borrow_mut().get_mut(&pane) {
                if *health > 0.0 && scaled as f32 >= 5.0 * crate::world_objects::T6_GLASS_DAMAGE_MULTIPLIER {
                    *health -= scaled as f32;
                }
                return;
            }
            let mut map = glass_damage.borrow_mut();
            let cur = map.entry(pane).or_insert(0);
            *cur = glass_add_damage(*cur, scaled);
        };
        let (segments, terminal) = bullet_trace_segments_filtered(
            world.clip_brushes(),
            world.clip_bsp(),
            world.clip_cmodels(),
            world.clip_mesh(),
            &query.players.poses,
            &query.entities.rows,
            &BulletTraceQuery {
                start: em.origin,
                end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(em.attacker),
                ignore_hit: em.ignore_hit,
                ignore_model: None,
            },
            pen,
            world.penetration_table(),
            &|piece| {
                glass_piece_is_solid(
                    glass_damage
                        .borrow()
                        .get(&(u32::from(piece)))
                        .copied()
                        .unwrap_or(0),
                ) && t6_health
                    .borrow()
                    .get(&(u32::from(piece)))
                    .is_none_or(|health| *health > 0.0)
            },
            Some(&on_glass_hit),
        );
        let entity_epoch = entity_collision_epoch(terminal, &query.entities.rows);
        let startsolid = segments.first().is_some_and(|s| s.startsolid);
        let impact_n = segments
            .iter()
            .filter(|s| bullet_process_on_hit(s.collider))
            .count() as u32;
        let shown_n = segments.iter().filter(|s| shows_impact(s.collider)).count();
        // bo2zm M3 fix list 3: a shot that hits nothing (the sky, or past a
        // shotgun's reach) still flies its tracer to the end of its range;
        // the effects only came with a hit, so it drew nothing: part of his
        // "I don't see any tracers on a lot of weapons". No normal = no
        // impact.
        if shown_n == 0 && !startsolid {
            world.push_pellet_fx(crate::PelletFxRecord {
                attacker: em.attacker.0 as i32,
                weapon: em.weapon,
                correlation: em.shot_id.0,
                pellet: em.pellet.0,
                hand: em.hand,
                start: em.origin,
                end: segments.last().map_or(end, |s| s.end),
                normal: [0.0; 3],
                surf_type: 0,
                surface_flags: 0,
                flesh_flags: 0,
            });
        }
        let event_n = segments
            .iter()
            .filter(|s| {
                shows_impact(s.collider)
                    && (on_shield(s.collider, bo2)
                        || hit_event(facts.impact_type, false, bo2).is_some())
            })
            .count() as u32;
        let (bone_center, bone_half_size, xmodel_contents, model_key) =
            hit_dump(terminal, &query.entities.rows);
        output.shot_verdicts.push(ShotCollisionVerdict {
            shot_id: em.shot_id,
            pellet: em.pellet,
            attacker: em.attacker,
            geometry: shot_collision_geometry(
                terminal,
                query.players.verdict,
                entity_epoch,
                player_representation(terminal, &query.players.poses),
            ),
            terminal,
            startsolid,
            bone_center,
            bone_half_size,
            xmodel_contents,
            model_key,
            end: segments.last().map(|s| s.end),
            entity_clip: entity_clip_kind(terminal, &query.entities.rows),
            impact_n,
            event_n,
        });
        if let Some(ricochet) = shield_ricochet(world, em, bounces, &segments, &query.players.poses)
        {
            queue.push((ricochet, bounces + 1));
        }
        if segments.is_empty() {
            continue;
        }
        let mut glass_hit: Vec<u32> = Vec::new();
        let shot_means =
            crate::script_player::means(world, DamageSource::Shot(em.shot_id), em.weapon, 0, false);
        for segment in &segments {
            let exit = segment.surface_flags & fx_t6::runtime::FX_IMPACT_EXIT_SURFACE_FLAG != 0;
            let dist = {
                let dx = segment.end[0] - em.origin[0];
                let dy = segment.end[1] - em.origin[1];
                let dz = segment.end[2] - em.origin[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            };
            let scaled =
                ((bullet_damage_at_distance(&facts, dist) as f32) * segment.damage_mult) as i32;
            if damage_log() {
                diag::info!(
                    Sim,
                    "bo2mp damage: shot {:?} segment surface {} at ({:.1} {:.1} {:.1}) exit {} thickness {:.2} into {:?} multiplier {:.3}",
                    em.shot_id,
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
            if !exit
                && let Some(ColliderId::Player {
                    client: victim,
                    life: victim_life,
                    hitloc,
                }) = segment.collider
            {
                let head = hud_t6::gamemsg::obituary_is_headshot(hitloc);
                if damage_log() {
                    diag::info!(
                        Sim,
                        "bo2mp damage: shot {:?} weapon {} pen type {} x{} hit player {:?} hitloc {:?} at {:.0} base {} pen multiplier {:.3} damage {}",
                        em.shot_id,
                        em.weapon,
                        pen.penetrate_type,
                        pen.penetrate_multiplier,
                        victim,
                        hitloc,
                        dist,
                        bullet_damage_at_distance(&facts, dist),
                        segment.damage_mult,
                        scaled
                    );
                }
                let mut fatal = false;
                if world.publishes_snapshot()
                    && scaled > 0
                    && let Some(vmeta) = world.client_meta(victim)
                    && vmeta.lifecycle == ClientLifecycle::Alive
                {
                    let attempt = crate::DamageAttempt {
                        source: DamageSource::Shot(em.shot_id),
                        pellet: em.pellet,
                        attacker: em.attacker,
                        attacker_life: em.attacker_life,
                        target: victim,
                        target_life: victim_life,
                        weapon: em.weapon,
                        amount: scaled,
                        killcam_entity_start_time: 0,
                        inflictor_origin: None,
                        hitloc,
                        blast: false,
                        penetrated: segment.penetrated,
                        impact: false,
                    };
                    fatal = matches!(
                        crate::damage::apply_damage_attempt(world, tick, &attempt),
                        crate::DamageOutcome::Died(_)
                    );
                }
                flesh_flags = fx_t6::runtime::flesh_hit_flags(head, fatal) as u8;
            }
            let payload = crate::EntityEventPayload {
                number: em.attacker.0 as i32,
                attacker_entity_num: em.attacker.0 as i32,
                other_entity_num: match segment.collider {
                    Some(
                        ColliderId::EntityDObjBone { owner, .. }
                        | ColliderId::EntityLinkedBrush { owner, .. },
                    ) => owner
                        .script_model()
                        .and_then(|id| world.gentity_number(id))
                        .unwrap_or(ENTITYNUM_NONE),
                    Some(ColliderId::Player { client, .. }) => client.0 as i32,
                    _ => ENTITYNUM_NONE,
                },
                event_parm: i32::from(flesh_flags),
                weapon: em.weapon,
                correlation: em.shot_id.0,
                pellet: em.pellet.0,
                hand: em.hand,
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
            if shows_impact(segment.collider) {
                if let Some(victim) = victim
                    && on_shield(segment.collider, bo2)
                {
                    // Bullet_ImpactEffect: a hit on a riot shield tells the
                    // others where it struck and him where on his shield.
                    let tag = query
                        .players
                        .poses
                        .iter()
                        .find(|p| p.client == victim)
                        .and_then(|p| p.shield_tag);
                    crate::t6::riotshield::broadcast_hit(
                        world,
                        tick,
                        victim,
                        tag.as_ref(),
                        payload,
                    );
                } else if let Some(world_event) = hit_event(facts.impact_type, false, bo2) {
                    let world_audience =
                        victim.map_or(EventAudience::All, EventAudience::AllExcept);
                    world.push_entity_event(tick, world_audience, world_event, payload);
                    if let Some(victim) = victim
                        && let Some(local_event) = hit_event(facts.impact_type, true, bo2)
                    {
                        world.push_entity_event(
                            tick,
                            EventAudience::Client(victim),
                            local_event,
                            payload,
                        );
                    }
                } else if fx_t6::runtime::impact_table_row(facts.impact_type, false).is_some() {
                    world.push_pellet_fx(crate::PelletFxRecord {
                        attacker: em.attacker.0 as i32,
                        weapon: em.weapon,
                        correlation: em.shot_id.0,
                        pellet: em.pellet.0,
                        hand: em.hand,
                        start: segment.start,
                        end: segment.end,
                        normal: segment.normal,
                        surf_type: segment.surf_type,
                        surface_flags: segment.surface_flags,
                        flesh_flags,
                    });
                }
            }
            if exit || !world.publishes_snapshot() || scaled <= 0 {
                continue;
            }
            // Bullet_Process: the damage triggers along this stretch of the bullet's path.
            if segment.collider.is_some() {
                crate::t6::damage_triggers::bullet_path(
                    world.ecs(),
                    em.attacker,
                    segment.start,
                    segment.end,
                    scaled,
                    shot_means,
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
                Some(ColliderId::Player { .. }) => {}
                Some(
                    ColliderId::EntityDObjBone { owner, .. }
                    | ColliderId::EntityLinkedBrush { owner, .. },
                ) => {
                    // bo2mp: the bullet's kind of death (a pistol's, a
                    // rifle's) picks the breakable piece's scale.
                    let t6_means =
                        crate::world_objects::t6_means_from_name(crate::script_player::means(
                            world,
                            DamageSource::Shot(em.shot_id),
                            em.weapon,
                            0,
                            false,
                        ));
                    // bo2mp: a Black Ops II map's loose prop takes the gun's
                    // full damage (BO2's client: `damage[0]`).
                    let amount = if crate::t5_destructible::is_loose_prop(world, owner) {
                        bullet_damage_at_distance(&facts, 0.0)
                    } else {
                        scaled
                    };
                    if let Some(ColliderId::EntityDObjBone { bone, .. }) = segment.collider
                        && crate::t5_destructible::apply_hit(
                            world,
                            tick,
                            owner,
                            bone,
                            amount as u32,
                            t6_means,
                            segment.end,
                            [
                                segment.end[0] - em.origin[0],
                                segment.end[1] - em.origin[1],
                                segment.end[2] - em.origin[2],
                            ],
                            Some(em.attacker),
                            em.weapon,
                        )
                    {
                        continue;
                    }
                    if let Some(target) = owner.script_model() {
                        let bone = match segment.collider {
                            Some(ColliderId::EntityDObjBone { bone, .. }) => {
                                Some(usize::from(bone))
                            }
                            _ => None,
                        };
                        let means = crate::script_player::means(
                            world,
                            DamageSource::Shot(em.shot_id),
                            em.weapon,
                            0,
                            false,
                        );
                        crate::script::damage_entity(
                            world.ecs(),
                            &crate::script::EntityHit {
                                target,
                                amount: scaled,
                                attacker: Some(em.attacker),
                                means,
                                weapon: em.weapon,
                                point: segment.end,
                                dir: em.direction,
                                bone,
                                flags: 0,
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
                        let at_time_ms = i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS))
                            .unwrap_or(i32::MAX);
                        let mut holdrand = *world.stuck_holdrand_mut();
                        if world.world_objects().is_t6_glass(piece) {
                            // Black Ops II: the bullet's damage, as its kind.
                            let means = crate::world_objects::t6_means_from_name(
                                crate::script_player::means(
                                    world,
                                    DamageSource::Shot(em.shot_id),
                                    em.weapon,
                                    0,
                                    false,
                                ),
                            );
                            world.world_objects_mut().apply_t6_glass_damage(
                                piece,
                                scaled,
                                means,
                                at_time_ms,
                                segment.end,
                                em.direction,
                                &mut || crate::item::random_unit(&mut holdrand),
                            );
                        }
                        *world.stuck_holdrand_mut() = holdrand;
                    }
                }
                None => {}
            }
        }
    }
    world.record_shot_collision_verdicts(&output.shot_verdicts);
    output
}

/// bullet_ricochetBaseChance: how often a riot shield turns a bullet aside.
pub const RICOCHET_BASE_CHANCE: f32 = 0.5;
/// How many times one bullet may bounce off riot shields in a tick. BO2 sets no limit; this
/// stops two shields facing each other from bouncing it for ever.
const MAX_RICOCHETS: u8 = 8;

/// A player's bullet a riot shield turned (`shield_bounce`): it flies on as a new bullet.
fn shield_ricochet(
    world: &FrameWorld,
    em: &Emission,
    bounces: u8,
    segments: &[BulletTraceSegment],
    poses: &[PlayerCollisionPose],
) -> Option<Emission> {
    let seed = (u64::from(em.shot_id.0) << 24) ^ (u64::from(em.pellet.0) << 8) ^ u64::from(bounces);
    let bounce = shield_bounce(world, em.weapon, em.direction, seed, bounces, segments, poses)?;
    Some(Emission {
        origin: bounce.origin,
        direction: bounce.direction,
        max_range: bounce.max_range,
        ignore_hit: Some(bounce.bearer),
        ..*em
    })
}

/// A bullet a riot shield turned: where it flies on from, which way, how far, and the
/// shield's bearer, whom it passes.
pub(crate) struct ShieldBounce {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_range: f32,
    pub bearer: ClientId,
}

/// BulletRicochet (Black Ops II): a bullet a player's riot shield stops flies on, half the
/// time, as a new bullet: never a shotgun's pellet or a bullet that blows up, and only one
/// that meets the shield's face. It leaves a unit off the hit along CalcRicochet's line with
/// the gun's full range, its damage falling off from there. BO2 starts it outside the shield
/// bearer, on the shield's face; ours may hit him on his body, so it passes through him.
/// Players' bullets and scorestreak guns' (a turret's, a vehicle's: both reach Bullet_Fire)
/// alike; `seed` rolls the dice, `bounces` it has made already.
pub(crate) fn shield_bounce(
    world: &FrameWorld,
    weapon: u32,
    direction: [f32; 3],
    seed: u64,
    bounces: u8,
    segments: &[BulletTraceSegment],
    poses: &[PlayerCollisionPose],
) -> Option<ShieldBounce> {
    use weapon_t6::facts::{weap_class, weap_type};
    if bounces >= MAX_RICOCHETS || world.gun_facts().is_empty() {
        return None;
    }
    let gun = world.gun_facts().get(weapon).core;
    if gun.weap_type != weap_type::BULLET
        || gun.bullet_impact_explode
        || matches!(
            gun.weap_class,
            weap_class::SPREAD | weap_class::PISTOL_SPREAD
        )
    {
        return None;
    }
    let hit = segments.last()?;
    let Some(ColliderId::Player {
        client,
        hitloc: HITLOC_SHIELD,
        ..
    }) = hit.collider
    else {
        return None;
    };
    // G_GetShieldTagNormal: the way the shield faces (ours is flat).
    let cover = poses.iter().find(|p| p.client == client)?.shield?;
    let normal = [cover.facing[0], cover.facing[1], 0.0];
    let roll = MatchRng::new(seed).next_u32() as f32 / u32::MAX as f32;
    let along = direction[0] * normal[0] + direction[1] * normal[1];
    if roll > RICOCHET_BASE_CHANCE || along > 0.0 {
        return None;
    }
    let out = ricochet_direction(direction, normal)?;
    let max_range = world.combat_facts_for(weapon)?.bullet_range();
    Some(ShieldBounce {
        origin: [
            hit.end[0] + out[0],
            hit.end[1] + out[1],
            hit.end[2] + out[2],
        ],
        direction: out,
        max_range,
        bearer: client,
    })
}

/// CalcRicochet: the way a bullet travelling along `dir` leaves a shield facing `normal`. A
/// normal that points down is tipped up first (z + 0.5, at most 0.1); the bullet then leaves
/// along the normal with the part along its own line taken out, square to the way it came.
/// `None` when it meets the shield square on and nothing is left.
pub(crate) fn ricochet_direction(dir: [f32; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    let unit = |v: [f32; 3]| {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (len > 1e-4).then(|| [v[0] / len, v[1] / len, v[2] / len])
    };
    let mut n = normal;
    if n[2] < 0.0 {
        n[2] = (n[2] + 0.5).min(0.1);
        n = unit(n)?;
    }
    let back = [-dir[0], -dir[1], -dir[2]];
    let along = back[0] * n[0] + back[1] * n[1] + back[2] * n[2];
    unit([
        n[0] - back[0] * along,
        n[1] - back[1] * along,
        n[2] - back[2] * along,
    ])
}

/// A melee hit's parm bit: a riot shield (carried or planted) took the swing (BO2's client plays
/// the knife-on-shield sound for a knife, nothing for a shield bash).
pub const MELEE_PARM_SHIELD: i32 = 2;

/// One melee swing's numbers: base damage, hit reach, and whether it is a knife (the hit sound).
pub(crate) struct MeleeSwing {
    pub damage: i32,
    pub range: f32,
    pub knife: bool,
}

/// The server's melee hit: traces the swing box ahead of the eye and damages what it finds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fire_melee_hit(
    world: &mut FrameWorld,
    tick: Tick,
    attacker: ClientId,
    attacker_life: LifeSequence,
    weapon: u32,
    origin: [f32; 3],
    angles: [f32; 3],
    swing: MeleeSwing,
) {
    if swing.damage <= 0 {
        return;
    }
    let (forward, right, up) = game_math::angle_vectors(angles);
    let range = swing.range;
    let width = PLAYER_MELEE_WIDTH_DEFAULT;
    let height = PLAYER_MELEE_HEIGHT_DEFAULT;
    let query = world.lagcomp_query_for(attacker, tick);
    let glass_pairs = world.world_objects().glass_damage_pairs();
    let is_solid = |piece| {
        crate::world_objects::glass_piece_is_solid(
            glass_pairs
                .iter()
                .find(|(id, _)| *id == u32::from(piece))
                .map(|(_, d)| *d)
                .unwrap_or(0),
        )
    };
    // BO2 starts a player's swing 10 units behind the eye (a target pressed against the
    // attacker is still in front of the trace); the far end stays at the eye plus the reach.
    let start = [
        origin[0] - forward[0] * 10.0,
        origin[1] - forward[1] * 10.0,
        origin[2] - forward[2] * 10.0,
    ];
    let mut best_frac = 1.0f32;
    let mut best_hit: Option<crate::bullet_collision::BulletTraceSegment> = None;
    // Melee_Trace: the first line, as far as it reaches, crosses the damage triggers.
    let mut trigger_end: Option<[f32; 3]> = None;
    let n = melee_trace_count(width, height);
    for (index, offset) in MELEE_TRACE_OFFSETS.iter().take(n).enumerate() {
        let end = melee_trace_end(origin, forward, right, up, range, width, height, *offset);
        let (segments, _) = bullet_trace_segments_filtered(
            world.clip_brushes(),
            world.clip_bsp(),
            world.clip_cmodels(),
            world.clip_mesh(),
            &query.players.poses,
            &query.entities.rows,
            &BulletTraceQuery {
                start,
                end,
                mask: MASK_BULLET_WORLD,
                ignore: Some(attacker),
                ignore_hit: None,
                ignore_model: None,
            },
            weapon_t6::penetration::BulletPenFacts::default(),
            world.penetration_table(),
            &is_solid,
            None,
        );
        if index == 0 {
            trigger_end = Some(
                segments
                    .iter()
                    .find(|s| s.collider.is_some())
                    .map_or(end, |s| s.end),
            );
        }
        let Some(segment) = segments.iter().find(|s| s.collider.is_some()) else {
            continue;
        };
        if segment.surface_flags & 0x10 != 0 {
            continue;
        }
        let ray = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let hit = [
            segment.end[0] - start[0],
            segment.end[1] - start[1],
            segment.end[2] - start[2],
        ];
        let ray_len2 = ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2];
        if ray_len2 <= 0.0 {
            continue;
        }
        let frac = (hit[0] * hit[0] + hit[1] * hit[1] + hit[2] * hit[2]).sqrt() / ray_len2.sqrt();
        if frac >= 1.0 {
            continue;
        }
        // BO2: the first offset that touches a player is the hit, even when another
        // offset met a wall nearer; otherwise the nearest thing touched (a later offset
        // wins a tie).
        if matches!(segment.collider, Some(ColliderId::Player { .. })) {
            best_hit = Some(*segment);
            break;
        }
        if frac > best_frac {
            continue;
        }
        best_frac = frac;
        best_hit = Some(*segment);
    }
    if let Some(to) = trigger_end
        && world.publishes_snapshot()
    {
        crate::t6::damage_triggers::bullet_path(
            world.ecs(),
            attacker,
            start,
            to,
            swing.damage,
            "MOD_MELEE",
        );
    }
    let Some(segment) = best_hit else {
        if shot_log() {
            diag::info!(Sim, "melee hit: attacker {} touched nothing", attacker.0);
        }
        return;
    };
    let amount = swing.damage + (world.combat_rng_mut().next_u32() % 5) as i32;
    // BO2: a riot shield takes a swing it stops (the scripts hear "riotshield" and hurt no
    // one). A riot-shield bash into a shield still reaches the player, with no hit location.
    let shield = matches!(
        segment.collider,
        Some(ColliderId::Player { hitloc: crate::bullet_collision::HITLOC_SHIELD, .. })
    );
    let other = match segment.collider {
        Some(ColliderId::Player { client, .. }) => client.0 as i32,
        Some(
            ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. },
        ) => owner
            .script_model()
            .and_then(|id| world.gentity_number(id))
            .unwrap_or(i32::from(trace_t6::ENTITYNUM_WORLD)),
        _ => i32::from(trace_t6::ENTITYNUM_WORLD),
    };
    // Weapon_Melee_internal's sound and blood. On a BO2 map its rules decide (crate::t6::melee);
    // otherwise a player or a model's bones struck is a hit that bleeds, a shield or anything
    // else a miss.
    let bo2 = crate::t6::melee::runs(world.ecs());
    let verdict = if bo2 {
        match segment.collider {
            Some(ColliderId::Player { client, .. }) => Some(crate::t6::melee::Struck::Player {
                client: client.0,
                shield,
            }),
            Some(
                ColliderId::EntityDObjBone { owner, .. }
                | ColliderId::EntityLinkedBrush { owner, .. },
            ) => owner.script_model().map(crate::t6::melee::Struck::Model),
            _ => None,
        }
        .and_then(|struck| crate::t6::melee::verdict(world.ecs(), attacker.0, struck))
        .unwrap_or_default()
    } else {
        let bleeds = !shield
            && matches!(
                segment.collider,
                Some(ColliderId::Player { .. } | ColliderId::EntityDObjBone { .. })
            );
        crate::t6::melee::Verdict {
            hit: bleeds,
            shield: false,
            blood: bleeds,
        }
    };
    let kind = if verdict.hit {
        entity_t6::EntityEventKind::MELEE_HIT
    } else {
        entity_t6::EntityEventKind::MELEE_MISS
    };
    let victim_alive = match segment.collider {
        Some(ColliderId::Player { client, .. }) => world
            .client_meta(client)
            .is_some_and(|m| m.lifecycle == ClientLifecycle::Alive),
        _ => true,
    };
    let glass = match segment.collider {
        Some(ColliderId::World { .. }) => glass_piece_from_hit(segment.hit_type, segment.hit_id),
        _ => None,
    };
    if shot_log() {
        diag::info!(
            Sim,
            "melee hit: attacker {} hit {other} ({}) for {amount}",
            attacker.0,
            if verdict.hit { "hit" } else { "miss" }
        );
    }
    let event = |event_parm| crate::EntityEventPayload {
        number: attacker.0 as i32,
        attacker_entity_num: attacker.0 as i32,
        other_entity_num: other,
        event_parm,
        weapon,
        origin: segment.end,
        direction: forward,
        surf_type: segment.surf_type,
        surface_flags: segment.surface_flags,
        ..Default::default()
    };
    // BO2 sends the blood first, then the hit or miss; glass gets neither (the pane breaks).
    if verdict.blood && victim_alive {
        world.push_entity_event(
            tick,
            EventAudience::All,
            entity_t6::EntityEventKind::MELEE_BLOOD,
            event(0),
        );
    }
    if !(bo2 && glass.is_some()) {
        // The parm: 1 = a knife, not a shield bash (the hit sound); 2 = a riot shield took it.
        let parm = i32::from(swing.knife) | if verdict.shield { MELEE_PARM_SHIELD } else { 0 };
        world.push_entity_event(tick, EventAudience::All, kind, event(parm));
    }
    match segment.collider {
        Some(ColliderId::Player {
            client: victim,
            life: victim_life,
            hitloc,
        }) => {
            if victim_alive {
                let attempt = crate::DamageAttempt {
                    source: DamageSource::Melee,
                    pellet: PelletId(0),
                    attacker,
                    attacker_life,
                    target: victim,
                    target_life: victim_life,
                    weapon,
                    amount,
                    killcam_entity_start_time: 0,
                    inflictor_origin: Some(origin),
                    hitloc: if shield && !swing.knife { 0 } else { hitloc },
                    blast: false,
                    penetrated: false,
                    impact: false,
                };
                let _ = crate::damage::apply_damage_attempt(world, tick, &attempt);
            }
        }
        // The script entity (a zombie, a planted riot shield, a car) takes the knife, as it
        // takes a bullet.
        Some(
            ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. },
        ) => {
            if let Some(target) = owner.script_model() {
                let bone = match segment.collider {
                    Some(ColliderId::EntityDObjBone { bone, .. }) => Some(usize::from(bone)),
                    _ => None,
                };
                crate::script::damage_entity(
                    world.ecs(),
                    &crate::script::EntityHit {
                        target,
                        amount,
                        attacker: Some(attacker),
                        means: "MOD_MELEE",
                        weapon,
                        point: segment.end,
                        dir: forward,
                        bone,
                        flags: 0,
                    },
                );
            }
        }
        Some(ColliderId::World { .. }) => {
            if let Some(piece) = glass {
                let at_time_ms =
                    i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
                let mut holdrand = *world.stuck_holdrand_mut();
                if world.world_objects().is_t6_glass(piece) {
                    // Black Ops II: melee breaks the pane at once.
                    world.world_objects_mut().apply_t6_glass_damage(
                        piece,
                        crate::world_objects::T6_GLASS_MELEE_DAMAGE,
                        crate::world_objects::T6_MOD_MELEE,
                        at_time_ms,
                        segment.end,
                        forward,
                        &mut || crate::item::random_unit(&mut holdrand),
                    );
                }
                *world.stuck_holdrand_mut() = holdrand;
            }
        }
        _ => {}
    }
}

fn entity_collision_epoch(
    terminal: Option<ColliderId>,
    rows: &[EntityCollisionTraceGeom],
) -> Option<EntityCollisionEpoch> {
    let owner = match terminal? {
        ColliderId::EntityDObjBone { owner, .. } | ColliderId::EntityLinkedBrush { owner, .. } => {
            owner
        }
        ColliderId::World { .. } | ColliderId::Player { .. } => return None,
    };
    rows.iter()
        .find(|row| row.owner == owner)
        .map(|row| row.epoch)
}

fn hit_dump(
    terminal: Option<ColliderId>,
    rows: &[EntityCollisionTraceGeom],
) -> (
    Option<[f32; 3]>,
    Option<[f32; 3]>,
    Option<u32>,
    Option<String>,
) {
    let Some(ColliderId::EntityDObjBone { owner, bone, .. }) = terminal else {
        return (None, None, None, None);
    };
    let Some(row) = rows.iter().find(|row| row.owner == owner) else {
        return (None, None, None, None);
    };
    let bone = row
        .collision
        .as_ref()
        .and_then(|collision| collision.bones.iter().find(|b| b.bone == bone));
    (
        bone.map(|b| b.center),
        bone.map(|b| b.half_size),
        row.dobj_contents,
        row.model_key.clone(),
    )
}

fn player_representation(
    terminal: Option<ColliderId>,
    poses: &[crate::bullet_collision::PlayerCollisionPose],
) -> PlayerCollisionRepresentation {
    let Some(ColliderId::Player { client, .. }) = terminal else {
        return PlayerCollisionRepresentation::StandingAabbV1;
    };
    match poses.iter().find(|pose| pose.client == client) {
        Some(pose) if !pose.bones.is_empty() => PlayerCollisionRepresentation::PosedBonesV1,
        _ => PlayerCollisionRepresentation::StandingAabbV1,
    }
}

fn entity_clip_kind(
    terminal: Option<ColliderId>,
    rows: &[EntityCollisionTraceGeom],
) -> Option<EntityClipKind> {
    match terminal {
        Some(ColliderId::EntityLinkedBrush { .. }) => Some(EntityClipKind::LinkedBrush),
        Some(ColliderId::EntityDObjBone { owner, .. }) => {
            let coll = rows
                .iter()
                .find(|row| row.owner == owner)
                .and_then(|row| row.collision.as_ref())
                .and_then(|collision| collision.coll.as_ref());
            Some(
                if crate::bullet_collision::coll_tris_clip_available(coll, MASK_BULLET_WORLD) {
                    EntityClipKind::CollTris
                } else {
                    EntityClipKind::BoneBoxes
                },
            )
        }
        _ => None,
    }
}

fn shot_collision_geometry(
    terminal: Option<ColliderId>,
    player_history: HistorySampleVerdict,
    entity_epoch: Option<EntityCollisionEpoch>,
    representation: PlayerCollisionRepresentation,
) -> ShotCollisionGeometry {
    match terminal {
        Some(ColliderId::Player { .. }) => ShotCollisionGeometry::Player {
            representation,
            history: player_history,
        },
        Some(ColliderId::EntityDObjBone { .. } | ColliderId::EntityLinkedBrush { .. }) => {
            match entity_epoch {
                Some(epoch) => ShotCollisionGeometry::Entity { epoch },
                None => ShotCollisionGeometry::Miss,
            }
        }
        Some(ColliderId::World { .. }) => ShotCollisionGeometry::World,
        None => match player_history {
            HistorySampleVerdict::Refused { .. } => ShotCollisionGeometry::Player {
                representation,
                history: player_history,
            },
            _ => ShotCollisionGeometry::Miss,
        },
    }
}

pub(crate) fn bullet_process_on_hit(collider: Option<ColliderId>) -> bool {
    collider.is_some()
}

/// Bullet_ImpactEffect: a hit shows an impact unless it is on the sky.
pub(crate) fn shows_impact(collider: Option<ColliderId>) -> bool {
    collider.is_some_and(|c| !crate::bullet_collision::is_sky(c))
}

/// bo2mp: a hit on the riot shield a player bears (Black Ops II content
/// only: its own shield events, `t6::riotshield::broadcast_hit`).
pub(crate) fn on_shield(collider: Option<ColliderId>, bo2: bool) -> bool {
    bo2 && matches!(collider, Some(ColliderId::Player { hitloc: HITLOC_SHIELD, .. }))
}

/// The bullet hit event a weapon's impact type gives (`local`: the one for
/// the player struck). Black Ops II's list has armour-piercing at 3, where
/// the legacy has the shield; there it hits as a large round does.
pub(crate) fn hit_event(impact_type: i32, local: bool, bo2: bool) -> Option<entity_t6::EntityEventKind> {
    let impact_type = if bo2 && impact_type == 3 { 2 } else { impact_type };
    entity_t6::bullet_hit_event(impact_type, local)
}

#[cfg(test)]
mod ricochet_tests {
    use super::ricochet_direction;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3)
    }

    #[test]
    fn a_ricochet_leaves_square_to_the_way_the_bullet_came() {
        let (s, c) = 30f32.to_radians().sin_cos();
        let dir = [c, s, 0.0];
        let out = ricochet_direction(dir, [-1.0, 0.0, 0.0]).unwrap();
        assert!(close(out, [-0.5, 0.866, 0.0]), "{out:?}");
        let square = out[0] * dir[0] + out[1] * dir[1] + out[2] * dir[2];
        assert!(square.abs() < 1e-4, "{square}");
    }

    #[test]
    fn a_bullet_square_on_to_the_shield_does_not_ricochet() {
        assert_eq!(ricochet_direction([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_shield_tipped_down_is_tipped_up_first() {
        let n = [-1.0 / 1.04f32.sqrt(), 0.0, -0.2 / 1.04f32.sqrt()];
        let out = ricochet_direction([1.0, 0.0, 0.0], n).unwrap();
        assert!(close(out, [0.0, 0.0, 1.0]), "{out:?}");
    }
}
