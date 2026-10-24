use crate::bullet::bullet_damage_at_distance;
use crate::bullet_collision::{
    BulletTraceQuery, ColliderId, EntityCollisionEpoch, EntityCollisionTraceGeom,
    HistorySampleVerdict, MASK_BULLET_WORLD, bullet_trace_segments_filtered, glass_piece_from_hit,
};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, MatchRng, PelletId, ShotId};
use crate::match_state::{ClientLifecycle, EventAudience};
use crate::world::{ClientId, Tick};
use crate::world_objects::glass_piece_is_solid;
use entity_iw4::glass_add_damage;
use playerstate_iw4::ENTITYNUM_NONE;
use std::cell::RefCell;
use std::collections::HashMap;
use weapon_iw4::{
    FireWeaponKind, MELEE_TRACE_OFFSETS, PLAYER_MELEE_HEIGHT_DEFAULT, PLAYER_MELEE_WIDTH_DEFAULT,
    fire_weapon_kind, melee_trace_count, melee_trace_end,
};

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
    cmd: playerstate_iw4::UserCmd,
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

pub(crate) fn apply_player_anim_event(world: &mut FrameWorld, id: ClientId, event: u8) {
    let Some(script) = world.player_anim_script() else {
        return;
    };
    let mut seed = world.anim_event_seed();
    let (view_w, primary) = {
        let Some(ps) = world.player_mut(id) else {
            return;
        };
        crate::pmove_anim_weapon_ids(ps)
    };
    let view_facts = world.combat_facts_for(view_w);
    let primary_facts = world.combat_facts_for(primary);
    {
        let Some(ps) = world.player_mut(id) else {
            return;
        };
        let conds = crate::anim_conditions_from_pmove(ps, view_facts, primary_facts);
        script.apply_event(ps, event, &conds, &mut seed);
    }
    world.set_anim_event_seed(seed);
}

pub fn spread_pellet_direction(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut MatchRng,
) -> [f32; 3] {
    spread_direction_on_plane(angles, spread_degrees, rng, 1.0)
}

/// Black Ops II Zombies' flat spread (Bullet_Endpos, a weapon with the
/// accuracy perk: the Blundergat): pellet 0 straight ahead, the rest out
/// to either side in turn along the aim's right, never up or down. Pellet
/// i sits `spread * 0.5 / (count / 2) * ((i - 1) / 2 + 1)` cone widths out,
/// right for even i, left for odd.
pub fn flat_pellet_direction(angles: [f32; 3], spread_degrees: f32, pellet: u16, count: u16) -> [f32; 3] {
    let (forward, right, _) = math_iw4::angle_vectors(angles);
    if spread_degrees <= 0.0 || pellet == 0 || count <= 1 {
        return forward;
    }
    let side = if pellet % 2 == 0 { 1.0 } else { -1.0 };
    let step = (i32::from(pellet - 1) >> 1) + 1;
    let out = spread_degrees * 0.5 / f32::from(count >> 1) * step as f32 * side;
    let lateral = spread_degrees.to_radians().tan() * out;
    let q: [f32; 3] = std::array::from_fn(|k| forward[k] + lateral * right[k]);
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if len > 0.0 { [q[0] / len, q[1] / len, q[2] / len] } else { forward }
}

/// bo2zm: `IW4L_SHOT_LOG` set = log every shot's aim (test aid).
pub(crate) fn shot_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("IW4L_SHOT_LOG").is_some())
}

pub fn spread_direction_on_plane(
    angles: [f32; 3],
    spread_degrees: f32,
    rng: &mut MatchRng,
    plane: f32,
) -> [f32; 3] {
    let (forward, right, up) = math_iw4::angle_vectors(angles);
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
            // BO2 fires every bullet weapon, whatever its damage: a 0-damage
            // one (the Paralyzer, whose script does its harm) still hits the
            // models and triggers it crosses, which take at least 1 (the
            // easter egg's amplifiers count Paralyzer hits). Players keep
            // their own `scaled > 0` gate in phase_trace.
            Some(FireWeaponKind::Bullet) => {
                if facts.bullet_range() <= 0.0 {
                    continue;
                }
            }
            _ => continue,
        }
        let mut rng = MatchRng::new(shot.combat_seed as u64);
        // bo2zm: Black Ops II Zombies' Double Tap Root Beer II fires every
        // bullet twice, shotgun pellets too (Bullet_Fire: PERK_RATEOFFIRE).
        let doubled = if crate::t6::has_perk(world, shot.attacker, "specialty_rof") { 2 } else { 1 };
        let pellet_count = (facts.pellet_count() * doubled).clamp(1, u16::MAX as i32) as u16;
        if doubled > 1 && std::env::var_os("IW4L_T6_WEAPLOG").is_some() {
            diag::info!(Sim, "bo2zm t6 double tap: weapon {} fires {} bullets", shot.weapon, pellet_count);
        }
        for pellet in 0..pellet_count {
            out.push(Emission {
                shot_id: shot.shot_id,
                pellet: PelletId(pellet),
                attacker: shot.attacker,
                attacker_life: shot.attacker_life,
                hand: shot.hand,
                weapon: shot.weapon,
                origin: shot.origin,
                direction: if facts.flat_spread {
                    flat_pellet_direction(shot.angles, shot.spread_degrees, pellet, pellet_count)
                } else {
                    spread_pellet_direction(shot.angles, shot.spread_degrees, &mut rng)
                },
                max_range: facts.bullet_range(),
                base_damage: facts.damage,
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
    for em in emissions {
        let Some(facts) = world.combat_facts_for(em.weapon) else {
            continue;
        };
        let query = world.lagcomp_query_for(em.attacker, tick);
        let end = [
            em.origin[0] + em.direction[0] * em.max_range,
            em.origin[1] + em.direction[1] * em.max_range,
            em.origin[2] + em.direction[2] * em.max_range,
        ];
        let pen = world.bullet_pen_facts_for(em.weapon);
        let glass_damage = RefCell::new(
            world
                .world_objects()
                .glass_damage_pairs()
                .into_iter()
                .collect::<HashMap<u32, u16>>(),
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
                ignore_hit: None,
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
                )
            },
            Some(&on_glass_hit),
        );
        let entity_epoch = entity_collision_epoch(terminal, &query.entities.rows);
        // BO2: a shot through a damage trigger fires it. Glass does not stop the bullet (the trace goes on
        // through the pane), so it does not end this check either: Buried's easter egg targets stand behind windows.
        if world.publishes_snapshot() {
            let to = segments
                .iter()
                .find(|s| match s.collider {
                    Some(ColliderId::World { contents, .. }) => {
                        contents & weapon_iw4::CONTENTS_GLASS == 0
                    }
                    other => other.is_some(),
                })
                .map_or(end, |s| s.end);
            let d = (0..3).map(|i| (to[i] - em.origin[i]).powi(2)).sum::<f32>().sqrt();
            crate::t6::damage_triggers(
                world.ecs(),
                em.origin,
                to,
                0.0,
                bullet_damage_at_distance(&facts, d).max(1),
                em.attacker,
                em.weapon,
                crate::t6::DamageKind::Bullet,
            );
        }
        let startsolid = segments.first().is_some_and(|s| s.startsolid);
        let impact_n = segments
            .iter()
            .filter(|s| bullet_process_on_hit(s.collider))
            .count() as u32;
        // bo2zm M3 fix list 3: a shot that hits nothing (the sky, or past a
        // shotgun's reach) still flies its tracer to the end of its range;
        // the effects only came with a hit, so it drew nothing: part of his
        // "I don't see any tracers on a lot of weapons". No normal = no
        // impact.
        if impact_n == 0 && !startsolid {
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
                bullet_process_on_hit(s.collider)
                    && entity_iw4::bullet_hit_event(facts.impact_type, false).is_some()
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
        if segments.is_empty() {
            continue;
        }
        let mut glass_hit: Vec<u32> = Vec::new();
        for segment in &segments {
            let exit = segment.surface_flags & fx_iw4::FX_IMPACT_EXIT_SURFACE_FLAG != 0;
            let dist = {
                let dx = segment.end[0] - em.origin[0];
                let dy = segment.end[1] - em.origin[1];
                let dz = segment.end[2] - em.origin[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            };
            let scaled =
                ((bullet_damage_at_distance(&facts, dist) as f32) * segment.damage_mult) as i32;
            let mut flesh_flags = 0u8;
            if !exit
                && let Some(ColliderId::Player {
                    client: victim,
                    life: victim_life,
                    hitloc,
                }) = segment.collider
            {
                let head = hud_iw4::obituary_is_headshot(hitloc);
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
                    };
                    fatal = matches!(
                        crate::damage::apply_damage_attempt(world, tick, &attempt),
                        crate::DamageOutcome::Died(_)
                    );
                }
                flesh_flags = fx_iw4::flesh_hit_flags(head, fatal) as u8;
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
            if bullet_process_on_hit(segment.collider) {
                if let Some(world_event) = entity_iw4::bullet_hit_event(facts.impact_type, false) {
                    let world_audience =
                        victim.map_or(EventAudience::All, EventAudience::AllExcept);
                    world.push_entity_event(tick, world_audience, world_event, payload);
                    if let Some(victim) = victim
                        && let Some(local_event) =
                            entity_iw4::bullet_hit_event(facts.impact_type, true)
                    {
                        world.push_entity_event(
                            tick,
                            EventAudience::Client(victim),
                            local_event,
                            payload,
                        );
                    }
                } else if fx_iw4::impact_table_row(facts.impact_type, false).is_some() {
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
            match segment.collider {
                Some(ColliderId::Player { .. }) => {}
                Some(
                    ColliderId::EntityDObjBone { owner, .. }
                    | ColliderId::EntityLinkedBrush { owner, .. },
                ) => {
                    if let Some(ColliderId::EntityDObjBone { bone, .. }) = segment.collider
                        && let Some(notifies) = crate::t5_destructible::apply_hit(
                            world,
                            tick,
                            owner,
                            bone,
                            scaled.max(1) as u32,
                        )
                    {
                        if std::env::var_os("IW4L_T6_HITLOG").is_some() {
                            diag::info!(Sim, "destructible hit surface {}", segment.surf_type);
                        }
                        // BO2: the map's scripts hear each break.
                        if let Some(target) = owner.script_model() {
                            for notify in notifies {
                                crate::t6::destructible_broken(
                                    world.ecs(),
                                    target,
                                    &notify,
                                    Some(em.attacker),
                                    em.weapon,
                                );
                            }
                        }
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
                        world.world_objects_mut().apply_glass_hit(
                            piece,
                            scaled as u32,
                            at_time_ms,
                            segment.end,
                            em.direction,
                            &mut || crate::item::random_unit(&mut holdrand),
                        );
                        *world.stuck_holdrand_mut() = holdrand;
                    }
                }
                None => {}
            }
        }
        // bo2zm: a Black Ops II bullet weapon whose def sets
        // bBulletImpactExplode explodes where its bullet first hits.
        if let Some(eq) = world.equipment_row_raw(em.weapon)
            && eq.bullet_impact_explode
            && let Some(hit) = segments.iter().find(|s| {
                s.collider.is_some() && s.surface_flags & fx_iw4::FX_IMPACT_EXIT_SURFACE_FLAG == 0
            })
        {
            bullet_impact_blast(world, tick, em, eq, hit.end, hit.normal, hit.surf_type);
        }
    }
    world.record_shot_collision_verdicts(&output.shot_verdicts);
    output
}

/// bo2zm: a bBulletImpactExplode bullet's explosion at `origin`: the
/// weapon's explosion effect (its impact type, as a grenade's) and its
/// explosion radius with inner / outer damage, as splash damage.
fn bullet_impact_blast(
    world: &mut FrameWorld,
    tick: Tick,
    em: &Emission,
    eq: crate::EquipmentRuntimeFacts,
    origin: [f32; 3],
    normal: [f32; 3],
    surf_type: u8,
) {
    world.push_entity_event(
        tick,
        EventAudience::All,
        entity_iw4::EntityEventKind::GRENADE_EXPLODE,
        crate::EntityEventPayload {
            number: i32::from(trace_iw4::ENTITYNUM_WORLD),
            attacker_entity_num: em.attacker.0 as i32,
            weapon: em.weapon,
            correlation: em.shot_id.0,
            origin,
            direction: normal,
            surf_type: surf_type.into(),
            ..Default::default()
        },
    );
    let radius = eq.explosion_radius.max(eq.explosion_radius_min).max(0) as f32;
    if radius <= 0.0 || (eq.explosion_inner_damage <= 0 && eq.explosion_outer_damage <= 0) {
        return;
    }
    let blast = crate::damage::ExplosionBlast {
        cone: None,
        origin,
        radius,
        inner_damage: eq.explosion_inner_damage as f32,
        outer_damage: eq.explosion_outer_damage.max(0) as f32,
        weapon: em.weapon,
        source: DamageSource::Projectile(crate::ProjectileId(0)),
        attacker: em.attacker,
        attacker_life: em.attacker_life,
        killcam_entity_start_time: 0,
    };
    crate::damage::apply_explosion_blast(world, tick, &blast);
}

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
    let (forward, right, up) = math_iw4::angle_vectors(angles);
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
    let n = melee_trace_count(width, height);
    for offset in MELEE_TRACE_OFFSETS.iter().take(n) {
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
            weapon_iw4::BulletPenFacts::default(),
            world.penetration_table(),
            &is_solid,
            None,
        );
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
    // BO2: a knife through a damage trigger fires it (Nuketown's bunker
    // hatch: Marlton).
    if world.publishes_snapshot() {
        let to = best_hit.map_or(
            std::array::from_fn(|i| origin[i] + forward[i] * range),
            |s| std::array::from_fn(|i| s.end[i] + forward[i] * 4.0),
        );
        crate::t6::damage_triggers(
            world.ecs(),
            origin,
            to,
            0.0,
            swing.damage,
            attacker,
            weapon,
            crate::t6::DamageKind::Melee,
        );
    }
    let Some(segment) = best_hit else {
        if shot_log() {
            diag::info!(Sim, "melee hit: attacker {} touched nothing", attacker.0);
        }
        return;
    };
    let amount = swing.damage + (world.combat_rng_mut().next_u32() % 5) as i32;
    // BO2: a swing stopped by a riot shield gives the attacker no hit and draws no blood; the
    // shield takes it (the scripts hear "riotshield" and hurt no one). A riot-shield bash
    // into a shield still reaches the player, with no hit location.
    let shield = matches!(
        segment.collider,
        Some(ColliderId::Player { hitloc: crate::bullet_collision::HITLOC_SHIELD, .. })
    );
    let (kind, other) = match segment.collider {
        Some(ColliderId::Player { client, .. }) if shield => {
            (entity_iw4::EntityEventKind::MELEE_MISS, client.0 as i32)
        }
        Some(ColliderId::Player { client, .. }) => {
            (entity_iw4::EntityEventKind::MELEE_HIT, client.0 as i32)
        }
        // bo2zm M3: a knife into a zombie (a script model's bones) is a hit.
        Some(ColliderId::EntityDObjBone { owner, .. }) => (
            entity_iw4::EntityEventKind::MELEE_HIT,
            owner
                .script_model()
                .and_then(|id| world.gentity_number(id))
                .unwrap_or(i32::from(trace_iw4::ENTITYNUM_WORLD)),
        ),
        _ => (
            entity_iw4::EntityEventKind::MELEE_MISS,
            i32::from(trace_iw4::ENTITYNUM_WORLD),
        ),
    };
    if shot_log() {
        diag::info!(
            Sim,
            "melee hit: attacker {} hit {other} ({}) for {amount}",
            attacker.0,
            if kind == entity_iw4::EntityEventKind::MELEE_HIT { "hit" } else { "miss" }
        );
    }
    world.push_entity_event(
        tick,
        EventAudience::All,
        kind,
        crate::EntityEventPayload {
            number: attacker.0 as i32,
            attacker_entity_num: attacker.0 as i32,
            other_entity_num: other,
            event_parm: i32::from(swing.knife),
            weapon,
            origin: segment.end,
            direction: forward,
            surf_type: segment.surf_type,
            surface_flags: segment.surface_flags,
            ..Default::default()
        },
    );
    match segment.collider {
        Some(ColliderId::Player {
            client: victim,
            life: victim_life,
            hitloc,
        }) => {
            if let Some(vmeta) = world.client_meta(victim)
                && vmeta.lifecycle == ClientLifecycle::Alive
            {
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
                };
                let _ = crate::damage::apply_damage_attempt(world, tick, &attempt);
                if shield {
                    return;
                }
                world.push_entity_event(
                    tick,
                    EventAudience::All,
                    entity_iw4::EntityEventKind::MELEE_BLOOD,
                    crate::EntityEventPayload {
                        number: attacker.0 as i32,
                        attacker_entity_num: attacker.0 as i32,
                        other_entity_num: victim.0 as i32,
                        weapon,
                        origin: segment.end,
                        direction: forward,
                        surf_type: segment.surf_type,
                        surface_flags: segment.surface_flags,
                        ..Default::default()
                    },
                );
            }
        }
        // bo2zm M3: the script entity (a zombie) takes the knife.
        Some(ColliderId::EntityDObjBone { owner, bone, .. }) => {
            // A breakable prop (a mannequin) takes the knife as a hit.
            if let Some(notifies) =
                crate::t5_destructible::apply_hit(world, tick, owner, bone, amount.max(0) as u32)
            {
                if let Some(target) = owner.script_model() {
                    for notify in notifies {
                        crate::t6::destructible_broken(
                            world.ecs(),
                            target,
                            &notify,
                            Some(attacker),
                            weapon,
                        );
                    }
                }
            } else if let Some(target) = owner.script_model() {
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
                        bone: Some(usize::from(bone)),
                        flags: 0,
                    },
                );
                world.push_entity_event(
                    tick,
                    EventAudience::All,
                    entity_iw4::EntityEventKind::MELEE_BLOOD,
                    crate::EntityEventPayload {
                        number: attacker.0 as i32,
                        attacker_entity_num: attacker.0 as i32,
                        other_entity_num: other,
                        weapon,
                        origin: segment.end,
                        direction: forward,
                        surf_type: segment.surf_type,
                        surface_flags: segment.surface_flags,
                        ..Default::default()
                    },
                );
            }
        }
        Some(ColliderId::World { .. }) => {
            if let Some(piece) = glass_piece_from_hit(segment.hit_type, segment.hit_id) {
                let at_time_ms =
                    i32::try_from(tick.0.saturating_mul(crate::MATCH_TICK_MS)).unwrap_or(i32::MAX);
                let mut holdrand = *world.stuck_holdrand_mut();
                world.world_objects_mut().apply_glass_hit(
                    piece,
                    u32::from(crate::world_objects::GLASS_MELEE_DAMAGE),
                    at_time_ms,
                    segment.end,
                    forward,
                    &mut || crate::item::random_unit(&mut holdrand),
                );
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
