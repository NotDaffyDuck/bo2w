use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::match_state::ClientLifecycle;
use crate::world::{ClientId, Tick};
use gamemode_t6::{
    G_CAN_DAMAGE_CONTENTS_MASK, can_damage_player_vis_scale, radius_damage_amount,
    radius_damage_distance_to_aabb,
};

const AREA_ENTITY_CAPACITY: usize = entity_t6::MAX_GENTITIES as usize;

const _: () = assert!(crate::MATCH_TICK_MS == 50);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageAttempt {
    pub source: crate::DamageSource,
    pub pellet: PelletId,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub target: ClientId,
    pub target_life: LifeSequence,
    pub weapon: u32,
    pub amount: i32,

    pub killcam_entity_start_time: i32,

    pub inflictor_origin: Option<[f32; 3]>,

    pub hitloc: u8,
    /// An explosion's blast (radius damage), not a direct hit by the
    /// missile or bolt itself.
    pub blast: bool,
    /// bo2mp: a bullet that went through something first (a wall, another
    /// player): the scripts' `idflags` carry BO2's penetration bit.
    pub penetrated: bool,
    /// bo2mp: a missile's own hit that does not go off on it (a grenade
    /// before it arms, a stuck Semtex, a bolt, a blade): Black Ops II's
    /// MOD_IMPACT.
    pub impact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageRefusal {
    FriendlyFire,
    NonPositive,
    MissingTarget,
    TargetNotAlive,
    StaleLife,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathCommit {
    pub victim: ClientId,
    pub victim_life: LifeSequence,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub source: crate::DamageSource,
    pub pellet: PelletId,
    pub weapon: u32,
    pub amount: i32,
    pub killcam_entity_start_time: i32,
    pub inflictor_origin: Option<[f32; 3]>,
    pub hitloc: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageOutcome {
    Refused(DamageRefusal),
    Nonlethal { health_after: i32 },
    Died(DeathCommit),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExplosionBlast {
    pub cone: Option<([f32; 3], f32)>,
    pub origin: [f32; 3],
    pub radius: f32,
    pub inner_damage: f32,
    pub outer_damage: f32,
    pub weapon: u32,
    pub source: DamageSource,
    pub attacker: ClientId,
    pub attacker_life: LifeSequence,
    pub killcam_entity_start_time: i32,
}

impl ExplosionBlast {
    pub(crate) fn contains(&self, point: [f32; 3]) -> bool {
        self.cone.is_none_or(|(forward, cosine)| {
            let delta: [f32; 3] = std::array::from_fn(|i| point[i] - self.origin[i]);
            let len = delta.iter().map(|v| v * v).sum::<f32>().sqrt();
            len <= f32::EPSILON
                || delta.iter().zip(forward).map(|(a, b)| a * b).sum::<f32>() >= cosine * len
        })
    }
}

pub(crate) fn apply_explosion_blast(world: &mut FrameWorld, tick: Tick, blast: &ExplosionBlast) {
    if !world.publishes_snapshot() {
        return;
    }
    let (attempts, shields) = radius_player_attempts(world, blast);
    for s in shields {
        shield_splash_event(world, tick, s.id, s.origin, s.unshielded);
    }
    for attempt in &attempts {
        let _ = apply_damage_attempt(world, tick, attempt);
    }
    apply_entity_blast(world, blast);
    // bo2mp: and the map's damage triggers.
    let means = crate::script_player::means(world, blast.source, blast.weapon, 0, true);
    crate::t6::damage_triggers::blast(
        world.ecs(),
        blast.origin,
        blast.radius,
        blast.inner_damage,
        blast.outer_damage,
        Some(blast.attacker),
        means,
        blast.weapon,
        &|p| blast.contains(p),
    );
    // A map's breakables (cars, barrels, mannequins): each piece by its own
    // distance, by the thrower's weapon (BO2's DestructibleRadiusDamage).
    let means = crate::world_objects::t6_means_from_name(crate::script_player::means(
        world,
        blast.source,
        blast.weapon,
        0,
        true,
    ));
    for (_, notifies) in crate::t5_destructible::apply_blast_by(
        world,
        tick,
        blast.origin,
        blast.radius,
        |d| {
            // BO2's G_RadiusDamage keeps the share unrounded (the entity's
            // G_Damage truncates it).
            let r = blast.radius.max(1.0);
            blast.outer_damage + (1.0 - d / r) * (blast.inner_damage - blast.outer_damage)
        },
        means,
        &[],
        Some(blast.attacker),
        blast.weapon,
    ) {
        for notify in notifies {
            diag::warn!(Sim, "T5 destructible blast notify unhosted: {notify}");
        }
    }
    // G_RadiusDamage ends with GlassSv_RadiusDamage: the blast's own
    // damage, radius and cone, as the explosion's splash kind.
    let means = crate::world_objects::t6_means_from_name(crate::script_player::means(
        world,
        blast.source,
        blast.weapon,
        0,
        true,
    ));
    let time = crate::level_time_ms(tick);
    let mut holdrand = *world.stuck_holdrand_mut();
    world.world_objects_mut().apply_t6_glass_radius(
        blast.origin,
        blast.radius,
        blast.cone,
        blast.inner_damage,
        blast.outer_damage,
        means,
        time,
        &mut || crate::item::random_unit(&mut holdrand),
    );
    *world.stuck_holdrand_mut() = holdrand;
}

fn apply_entity_blast(world: &mut FrameWorld, blast: &ExplosionBlast) {
    if blast.radius <= 0.0 {
        return;
    }
    let means = crate::script_player::means(world, blast.source, blast.weapon, 0, true);
    for (target, mid, dist) in
        crate::script::radius_targets(world.ecs(), blast.origin, blast.radius)
    {
        if !blast.contains(mid) {
            continue;
        }
        if crate::t6::blast_vehicle(
            world.ecs(),
            target,
            blast.origin,
            blast.radius,
            blast.inner_damage,
            blast.outer_damage,
            blast.cone,
            means,
            Some(blast.attacker),
            blast.weapon,
        ) {
            continue;
        }
        let amount = radius_damage_amount(
            blast.inner_damage,
            blast.outer_damage,
            blast.radius,
            dist,
            1.0,
        );
        if amount <= 0 {
            continue;
        }
        crate::script::damage_entity(
            world.ecs(),
            &crate::script::EntityHit {
                target,
                amount,
                attacker: Some(blast.attacker),
                means,
                weapon: blast.weapon,
                point: mid,
                dir: std::array::from_fn(|i| mid[i] - blast.origin[i]),
                bone: None,
                flags: IDFLAGS_RADIUS,
            },
        );
    }
}

const IDFLAGS_RADIUS: i32 = 1;

pub(crate) fn apply_script_blast(
    world: &mut FrameWorld,
    tick: Tick,
    blast: &crate::script::ScriptBlast,
) {
    if blast.radius <= 0.0 {
        return;
    }
    for target in radius_player_candidates(world, blast.origin, blast.radius) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        if meta.lifecycle != ClientLifecycle::Alive {
            continue;
        }
        let victim_life = meta.life_sequence;
        let Some(bounds) = world.player_area_bounds(target) else {
            continue;
        };
        let dist = radius_damage_distance_to_aabb(blast.origin, bounds.mid(), bounds.half());
        let mut vis_scale = player_radius_vis_scale(world, blast.origin, target);
        // bo2mp: the riot shield he bears takes its share of the blast, and
        // his view shakes.
        let shield = world
            .player(target)
            .map(|ps| (world.shield_blast_protection(target, ps, blast.origin), ps.origin));
        if let Some((shield, at)) = shield {
            if shield > 0.0 {
                let unshielded =
                    radius_damage_amount(blast.max, blast.min, blast.radius, dist, vis_scale);
                shield_splash_event(world, tick, target, at, unshielded);
            }
            vis_scale *= 1.0 - shield;
        }
        let amount = radius_damage_amount(blast.max, blast.min, blast.radius, dist, vis_scale);
        if amount <= 0 {
            continue;
        }
        let victim_origin = world.player(target).map_or(bounds.mid(), |ps| ps.origin);
        let dir: [f32; 3] = std::array::from_fn(|i| victim_origin[i] - blast.origin[i]);
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        let commit = blast.attacker.and_then(|attacker| {
            Some(DeathCommit {
                victim: target,
                victim_life,
                attacker,
                attacker_life: world.client_meta(attacker)?.life_sequence,
                source: DamageSource::Radius(
                    blast
                        .inflictor
                        .unwrap_or(crate::ScriptModelId::from_wire(u32::MAX)),
                ),
                pellet: PelletId(0),
                weapon: blast.weapon,
                amount,
                killcam_entity_start_time: 0,
                inflictor_origin: Some(blast.origin),
                hitloc: 0,
            })
        });
        let hit = crate::script_player::Hit {
            victim: target,
            attacker: blast.attacker,
            amount,
            flags: IDFLAGS_RADIUS,
            means: blast.means,
            weapon: blast.weapon,
            point: blast.origin,
            dir: if len > 0.0 {
                dir.map(|c| c / len)
            } else {
                [0.0; 3]
            },
            hitloc: 0,
            inflictor: None,
            commit,
        };
        crate::script::player_damage(world.ecs(), tick, &hit);
    }
    apply_shared_glass_blast(
        world,
        tick,
        blast.origin,
        blast.max,
        blast.min,
        blast.radius,
        blast.means,
    );
    // bo2mp: and the map's damage triggers.
    crate::t6::damage_triggers::blast(
        world.ecs(),
        blast.origin,
        blast.radius,
        blast.max,
        blast.min,
        blast.attacker,
        blast.means,
        blast.weapon,
        &|_| true,
    );
}

pub(crate) fn apply_script_hit(world: &mut FrameWorld, tick: Tick, hit: &crate::script::ScriptHit) {
    let crate::script::HitTarget::Player(target) = hit.target else {
        return;
    };
    if hit.amount <= 0 {
        return;
    }
    let Some(meta) = world.client_meta(target) else {
        return;
    };
    if meta.lifecycle != ClientLifecycle::Alive {
        return;
    }
    let victim_life = meta.life_sequence;
    let Some(victim_origin) = world.player(target).map(|ps| ps.origin) else {
        return;
    };
    let dir: [f32; 3] = std::array::from_fn(|i| victim_origin[i] - hit.origin[i]);
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    let commit = hit.attacker.and_then(|attacker| {
        Some(DeathCommit {
            victim: target,
            victim_life,
            attacker,
            attacker_life: world.client_meta(attacker)?.life_sequence,
            source: DamageSource::Radius(
                hit.inflictor
                    .unwrap_or(crate::ScriptModelId::from_wire(u32::MAX)),
            ),
            pellet: PelletId(0),
            weapon: hit.weapon,
            amount: hit.amount,
            killcam_entity_start_time: 0,
            inflictor_origin: Some(hit.origin),
            hitloc: hit.hitloc,
        })
    });
    let player_hit = crate::script_player::Hit {
        victim: target,
        attacker: hit.attacker,
        amount: hit.amount,
        flags: hit.flags,
        means: hit.means,
        weapon: hit.weapon,
        point: hit.origin,
        dir: if len > 0.0 {
            dir.map(|c| c / len)
        } else {
            [0.0; 3]
        },
        hitloc: hit.hitloc,
        inflictor: None,
        commit,
    };
    crate::script::player_damage(world.ecs(), tick, &player_hit);
}

/// bo2mp: a riot shield that took a share of a blast: its bearer, where he
/// stands, and what the blast would have done to him unshielded.
struct ShieldSplash {
    id: ClientId,
    origin: [f32; 3],
    unshielded: i32,
}

/// bo2mp: BO2's G_AddShieldSplashDamageEvent: the riot shield `id` bears
/// took a share of a blast. Everyone gets the event; it shakes his view,
/// harder for a blast that would have done more (`unshielded`).
pub(crate) fn shield_splash_event(
    world: &mut FrameWorld,
    tick: Tick,
    id: ClientId,
    origin: [f32; 3],
    unshielded: i32,
) {
    if std::env::var_os("BO2MP_SHIELDLOG").is_some() {
        diag::info!(Sim, "bo2mp shield: splash event for {} unshielded {unshielded}", id.0);
    }
    world.push_entity_event(
        tick,
        crate::EventAudience::All,
        entity_t6::EntityEventKind::EXPLOSIVE_SPLASH_ON_SHIELD,
        shield_splash(id, origin, unshielded),
    );
}

/// The shield splash event's fields: its bearer, and the blast's damage
/// unshielded in event_parm (BO2 keeps it to 26 bits).
fn shield_splash(id: ClientId, origin: [f32; 3], unshielded: i32) -> crate::EntityEventPayload {
    crate::EntityEventPayload {
        number: id.0 as i32,
        event_parm: unshielded.clamp(0, 0x3ff_ffff),
        origin,
        ..Default::default()
    }
}

fn radius_player_attempts(
    world: &FrameWorld,
    blast: &ExplosionBlast,
) -> (Vec<DamageAttempt>, Vec<ShieldSplash>) {
    let mut intents = Vec::new();
    let mut shields = Vec::new();
    if blast.radius <= 0.0 {
        return (intents, shields);
    }
    for target in radius_player_candidates(world, blast.origin, blast.radius) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        let Some(bounds) = world.player_area_bounds(target) else {
            continue;
        };
        if !blast.contains(bounds.mid()) {
            continue;
        }
        let dist = radius_damage_distance_to_aabb(blast.origin, bounds.mid(), bounds.half());
        let mut vis_scale = player_radius_vis_scale(world, blast.origin, target);
        // bo2mp: the riot shield he bears takes its share of the blast, and
        // his view shakes.
        if let Some(ps) = world.player(target) {
            let shield = world.shield_blast_protection(target, ps, blast.origin);
            if shield > 0.0 && meta.lifecycle == ClientLifecycle::Alive {
                shields.push(ShieldSplash {
                    id: target,
                    origin: ps.origin,
                    unshielded: radius_damage_amount(
                        blast.inner_damage,
                        blast.outer_damage,
                        blast.radius,
                        dist,
                        vis_scale,
                    ),
                });
            }
            vis_scale *= 1.0 - shield;
        }
        let amount = radius_damage_amount(
            blast.inner_damage,
            blast.outer_damage,
            blast.radius,
            dist,
            vis_scale,
        );
        if amount <= 0 {
            continue;
        }
        intents.push(DamageAttempt {
            source: blast.source,
            pellet: PelletId(0),
            attacker: blast.attacker,
            attacker_life: blast.attacker_life,
            target,
            target_life: meta.life_sequence,
            weapon: blast.weapon,
            amount,
            killcam_entity_start_time: blast.killcam_entity_start_time,
            inflictor_origin: Some(blast.origin),
            hitloc: 0,
            blast: true,
            penetrated: false,
            impact: false,
        });
    }
    (intents, shields)
}

pub(crate) fn radius_player_candidates(
    world: &FrameWorld,
    origin: [f32; 3],
    radius: f32,
) -> Vec<ClientId> {
    let area_half = gamemode_t6::radius_damage_area_half_extent(radius);
    let Some(query) = crate::area_world::AreaBox::from_mid_half(origin, [area_half; 3]) else {
        diag::warn!(
            Sim,
            "radius damage at {origin:?} r={radius} has no valid area"
        );
        return Vec::new();
    };
    world
        .area_entity_candidates(query, u32::MAX, AREA_ENTITY_CAPACITY)
        .into_iter()
        .map(|entity_num| ClientId(u32::from(entity_num)))
        .collect()
}

pub(crate) fn apply_shared_glass_blast(
    world: &mut FrameWorld,
    tick: Tick,
    origin: [f32; 3],
    inner_damage: f32,
    outer_damage: f32,
    radius: f32,
    means: &str,
) {
    let time = crate::level_time_ms(tick);
    let mut holdrand = *world.stuck_holdrand_mut();
    // Black Ops II: GlassSv_RadiusDamage, no cone, the blast's kind.
    world.world_objects_mut().apply_t6_glass_radius(
        origin,
        radius,
        None,
        inner_damage,
        outer_damage,
        crate::world_objects::t6_means_from_name(means),
        time,
        &mut || crate::item::random_unit(&mut holdrand),
    );
    *world.stuck_holdrand_mut() = holdrand;
}

fn player_radius_vis_scale(world: &FrameWorld, inflictor: [f32; 3], target: ClientId) -> f32 {
    let Some(ps) = world.player(target) else {
        return 1.0;
    };
    let eye = [ps.origin[0], ps.origin[1], ps.origin[2] + ps.view_height_current];
    can_damage_player_vis_scale(
        ps.origin,
        eye,
        inflictor,
        None,
        |start, end| {
            let trace =
                world.trace_world(start, end, [0.0; 3], [0.0; 3], G_CAN_DAMAGE_CONTENTS_MASK);
            t_trace_passed(&trace)
        },
    )
}

fn t_trace_passed(trace: &trace_t6::Trace) -> bool {
    trace.fraction >= 1.0 && !trace.startsolid
}

pub(crate) fn apply_flashbang_blast(
    world: &mut FrameWorld,
    origin: [f32; 3],
    radius_max: f32,
    radius_min: f32,
    attacker: ClientId,
) {
    if crate::t6::flashbang::blast(world.ecs(), origin, radius_max, radius_min, attacker) {
        return;
    }
    let min_r = radius_min.max(1.0);
    let max_r = if radius_max < min_r {
        min_r
    } else {
        radius_max
    };
    let mut hits = Vec::new();
    for target in radius_player_candidates(world, origin, max_r) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        if meta.lifecycle != ClientLifecycle::Alive {
            continue;
        }
        let Some(ps) = world.player(target) else {
            continue;
        };
        let dist = {
            let dx = ps.origin[0] - origin[0];
            let dy = ps.origin[1] - origin[1];
            let dz = ps.origin[2] - origin[2];
            (dx * dx + dy * dy + dz * dz).sqrt()
        };
        if dist > max_r {
            continue;
        }
        if player_radius_vis_scale(world, origin, target) <= 0.0 {
            continue;
        }
        let amount_distance = flashbang_amount_distance(dist, min_r, max_r);
        let view_origin = [
            ps.origin[0],
            ps.origin[1],
            ps.origin[2] + ps.view_height_current,
        ];
        let (forward, _, _) = game_math::angle_vectors(ps.viewangles);
        let to_blast = [
            origin[0] - view_origin[0],
            origin[1] - view_origin[1],
            origin[2] - view_origin[2],
        ];
        let len =
            (to_blast[0] * to_blast[0] + to_blast[1] * to_blast[1] + to_blast[2] * to_blast[2])
                .sqrt();
        let amount_angle = if len > 0.0 {
            flashbang_amount_angle(
                (to_blast[0] * forward[0] + to_blast[1] * forward[1] + to_blast[2] * forward[2])
                    / len,
            )
        } else {
            flashbang_amount_angle(1.0)
        };
        hits.push((target, amount_distance, amount_angle));
    }
    for (target, amount_distance, amount_angle) in hits {
        crate::script::flashbang(
            world.ecs(),
            target,
            origin,
            amount_distance,
            amount_angle,
            attacker,
        );
    }
}

fn flashbang_amount_distance(dist: f32, min_r: f32, max_r: f32) -> f32 {
    if dist <= min_r || max_r <= min_r {
        1.0
    } else {
        1.0 - (dist - min_r) / (max_r - min_r)
    }
}

fn flashbang_amount_angle(dot: f32) -> f32 {
    (dot + 1.0) * 0.5
}

pub(crate) fn apply_damage_attempt(
    world: &mut FrameWorld,
    tick: Tick,
    intent: &DamageAttempt,
) -> DamageOutcome {
    if intent.amount <= 0 {
        return DamageOutcome::Refused(DamageRefusal::NonPositive);
    }
    let Some(meta) = world.client_meta(intent.target) else {
        return DamageOutcome::Refused(DamageRefusal::MissingTarget);
    };
    if meta.lifecycle != ClientLifecycle::Alive {
        return DamageOutcome::Refused(DamageRefusal::TargetNotAlive);
    }
    if meta.life_sequence != intent.target_life {
        return DamageOutcome::Refused(DamageRefusal::StaleLife);
    }
    let mut amount = intent.amount;
    // A hit the riot shield stopped carries the bullet's whole damage to
    // the scripts (BO2's callback counts what the shield blocked).
    if takes_location_scale(intent.source, intent.hitloc, intent.blast) {
        let scale = world
            .combat_facts_for(intent.weapon)
            .map(|facts| facts.location_scale(intent.hitloc))
            .unwrap_or(1.0);
        // BO2: cut to a whole number, at least 1.
        if amount > 0 {
            amount = weapon_t6::hitloc::scaled_damage(amount, scale);
        }
    }
    if amount <= 0 {
        return DamageOutcome::Refused(DamageRefusal::NonPositive);
    }
    crate::script_player::damage(world, tick, intent, amount)
}

pub(crate) fn play_death(
    world: &mut FrameWorld,
    victim: ClientId,
    attacker: Option<ClientId>,
    _commit: Option<&DeathCommit>,
) -> bool {
    let Some(self_ps) = world.player(victim).copied() else {
        return false;
    };
    let attacker_origin = attacker
        .filter(|&id| id != victim)
        .and_then(|id| world.player(id).map(|p| p.origin));
    let yaw =
        playerstate_t6::game::look_at_killer_yaw(attacker_origin, self_ps.origin, self_ps.viewangles[1]);

    let dead_viewangles = [0.0, self_ps.viewangles[1], 0.0];

    if let Some(ps) = world.player_mut(victim) {
        ps.viewangles = dead_viewangles;
        ps.pm_type = playerstate_t6::game::PM_TYPE_DEAD;
    }
    // bo2mp: BO2's player_die stops after the death anim for a player who
    // drives a remote-control vehicle (EF2_REMOTE_CONTROL, the RC-XD's) or
    // whose view is linked away (PLF_WEAPONVIEW_ONLY, the Lodestar's): his
    // view stays his and he is not turned to his killer.
    if self_ps.link_flags & 4 != 0 || crate::t6::in_remote_vehicle(world.ecs(), victim.0) {
        if let Some(ps) = world.player_mut(victim) {
            ps.viewangles = self_ps.viewangles;
        }
        return true;
    }
    world.client_meta_mut(victim).look_at_killer_yaw = yaw;
    true
}

/// BO2 multiplies every hit but melee by its hit location's multiplier, a
/// projectile's own hit by its weapon's table as a bullet's is. bo2mp: a
/// projectile's blast (hit location none) keeps its damage unscaled, as
/// before: TODO(fact) BO2's per-weapon value at "none" was not found.
fn takes_location_scale(source: crate::DamageSource, hitloc: u8, blast: bool) -> bool {
    match source {
        crate::DamageSource::Melee => false,
        crate::DamageSource::Projectile(_) if blast => false,
        _ => hitloc != crate::bullet_collision::HITLOC_SHIELD,
    }
}

#[cfg(test)]
mod location_scale_tests {
    use super::takes_location_scale;
    use crate::{DamageSource, ProjectileId};

    #[test]
    fn a_projectile_s_own_hit_takes_its_hit_location_multiplier() {
        let missile = DamageSource::Projectile(ProjectileId(1));
        assert!(takes_location_scale(missile, weapon_t6::hitloc::HITLOC_HEAD, false));
        assert!(!takes_location_scale(missile, 0, true));
        assert!(!takes_location_scale(missile, crate::bullet_collision::HITLOC_SHIELD, false));
        assert!(!takes_location_scale(DamageSource::Melee, weapon_t6::hitloc::HITLOC_HEAD, false));
        // A launcher's 150 impact to the head through a 1.5 table is 225; to the gun (0.0) it is 1.
        assert_eq!(weapon_t6::hitloc::scaled_damage(150, 1.5), 225);
        assert_eq!(weapon_t6::hitloc::scaled_damage(150, 0.0), 1);
    }
}

#[cfg(test)]
mod shield_splash_tests {
    use super::{shield_splash, ClientId};

    #[test]
    fn a_shield_splash_names_its_bearer_and_the_blast_unshielded() {
        let p = shield_splash(ClientId(3), [1.0, 2.0, 3.0], 120);
        assert_eq!((p.number, p.event_parm, p.origin), (3, 120, [1.0, 2.0, 3.0]));
        assert_eq!(shield_splash(ClientId(0), [0.0; 3], -5).event_parm, 0);
        assert_eq!(shield_splash(ClientId(0), [0.0; 3], i32::MAX).event_parm, 0x3ff_ffff);
    }
}
