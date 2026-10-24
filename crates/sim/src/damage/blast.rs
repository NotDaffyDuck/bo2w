//! An explosion: what it does to the players, entities, map triggers, breakables and glass inside its radius.

use super::shield::ShieldSplash;
use super::{DamageAttempt, Falloff, apply_damage_attempt, dot, length, minus, player_share, reachable_clients};
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::match_state::ClientLifecycle;
use crate::script_player::IDFLAGS_RADIUS;
use crate::world::{ClientId, Tick};

/// An explosion at `origin`: `inner_damage` at its centre falling to `outer_damage` at `radius`, optionally only
/// inside a cone (its facing and the cosine of its half angle).
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
    /// Whether `point` is inside the blast's cone (always, for a blast without one). The centre itself is.
    pub(crate) fn contains(&self, point: [f32; 3]) -> bool {
        let Some((facing, cosine)) = self.cone else {
            return true;
        };
        let offset = minus(point, self.origin);
        let reach = length(offset);
        reach <= f32::EPSILON || dot(offset, facing) >= cosine * reach
    }

    fn falloff(&self) -> Falloff {
        Falloff { inner: self.inner_damage, outer: self.outer_damage, radius: self.radius }
    }
}

/// Sets off `blast`. The damage every player takes is worked out first and dealt after, so one death does not change
/// what the next player takes. Then come the entities, the map's damage triggers, its breakables and its glass.
pub(crate) fn apply_explosion_blast(world: &mut FrameWorld, tick: Tick, blast: &ExplosionBlast) {
    if !world.publishes_snapshot() {
        return;
    }
    let means = crate::script_player::means(world, blast.source, blast.weapon, 0, true);

    let (attempts, splashes) = players_in_reach(world, blast);
    for splash in splashes {
        splash.announce(world, tick);
    }
    for attempt in &attempts {
        let _ = apply_damage_attempt(world, tick, attempt);
    }

    hurt_entities(world, blast, means);
    crate::t6::damage_triggers::blast(
        world.ecs(),
        blast.origin,
        blast.radius,
        blast.inner_damage,
        blast.outer_damage,
        Some(blast.attacker),
        means,
        blast.weapon,
        &|point| blast.contains(point),
    );
    break_destructibles(world, tick, blast, means);
    break_glass(world, tick, blast.origin, blast.radius, blast.cone, blast.falloff(), means);
}

/// Every player the blast reaches, as the damage to attempt on him; and for those with a riot shield up who are
/// alive, the splash event of what the shield saved.
fn players_in_reach(world: &FrameWorld, blast: &ExplosionBlast) -> (Vec<DamageAttempt>, Vec<ShieldSplash>) {
    let mut attempts = Vec::new();
    let mut splashes = Vec::new();
    if blast.radius <= 0.0 {
        return (attempts, splashes);
    }
    let falloff = blast.falloff();
    for target in reachable_clients(world, blast.origin, blast.radius) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        let Some(bounds) = world.player_area_bounds(target) else {
            continue;
        };
        if !blast.contains(bounds.mid()) {
            continue;
        }
        let share = player_share(world, blast.origin, falloff, target, &bounds);
        if meta.lifecycle == ClientLifecycle::Alive {
            splashes.extend(share.splash);
        }
        if share.amount <= 0 {
            continue;
        }
        attempts.push(DamageAttempt {
            source: blast.source,
            pellet: PelletId(0),
            attacker: blast.attacker,
            attacker_life: blast.attacker_life,
            target,
            target_life: meta.life_sequence,
            weapon: blast.weapon,
            amount: share.amount,
            killcam_entity_start_time: blast.killcam_entity_start_time,
            inflictor_origin: Some(blast.origin),
            hitloc: 0,
            blast: true,
            penetrated: false,
            impact: false,
        });
    }
    (attempts, splashes)
}

/// The blast's damage to the script entities in its radius. A vehicle takes it its own way.
fn hurt_entities(world: &mut FrameWorld, blast: &ExplosionBlast, means: &'static str) {
    if blast.radius <= 0.0 {
        return;
    }
    let falloff = blast.falloff();
    for (target, middle, distance) in crate::script::radius_targets(world.ecs(), blast.origin, blast.radius) {
        if !blast.contains(middle) {
            continue;
        }
        let vehicle_took_it = crate::t6::blast_vehicle(
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
        );
        if vehicle_took_it {
            continue;
        }
        let amount = falloff.at(distance, 1.0);
        if amount <= 0 {
            continue;
        }
        let hit = crate::script::EntityHit {
            target,
            amount,
            attacker: Some(blast.attacker),
            means,
            weapon: blast.weapon,
            point: middle,
            dir: minus(middle, blast.origin),
            bone: None,
            flags: IDFLAGS_RADIUS,
        };
        crate::script::damage_entity(world.ecs(), &hit);
    }
}

/// What a blast does to a breakable `distance` from its centre. The share is kept unrounded; the breakable's own
/// damage call cuts it to whole points.
fn destructible_damage(blast: &ExplosionBlast, distance: f32) -> f32 {
    let spread = blast.radius.max(1.0);
    blast.outer_damage + (1.0 - distance / spread) * (blast.inner_damage - blast.outer_damage)
}

/// The map's breakables (cars, barrels, mannequins) each take the blast by their own distance, as the thrower's
/// weapon.
fn break_destructibles(world: &mut FrameWorld, tick: Tick, blast: &ExplosionBlast, means: &'static str) {
    let broken = crate::t5_destructible::apply_blast_by(
        world,
        tick,
        blast.origin,
        blast.radius,
        |distance| destructible_damage(blast, distance),
        crate::world_objects::t6_means_from_name(means),
        &[],
        Some(blast.attacker),
        blast.weapon,
    );
    for (_, notifies) in broken {
        for notify in notifies {
            diag::warn!(Sim, "T5 destructible blast notify unhosted: {notify}");
        }
    }
}

/// Cracks and shatters the map's glass the blast reaches, with the blast's own cone, damage and kind.
fn break_glass(
    world: &mut FrameWorld,
    tick: Tick,
    origin: [f32; 3],
    radius: f32,
    cone: Option<([f32; 3], f32)>,
    falloff: Falloff,
    means: &str,
) {
    let at_ms = crate::level_time_ms(tick);
    let mut holdrand = *world.stuck_holdrand_mut();
    world.world_objects_mut().apply_t6_glass_radius(
        origin,
        radius,
        cone,
        falloff.inner,
        falloff.outer,
        crate::world_objects::t6_means_from_name(means),
        at_ms,
        &mut || crate::item::random_unit(&mut holdrand),
    );
    *world.stuck_holdrand_mut() = holdrand;
}

/// A blast on the glass alone, with no cone (a map's own explosion).
pub(crate) fn apply_shared_glass_blast(
    world: &mut FrameWorld,
    tick: Tick,
    origin: [f32; 3],
    inner_damage: f32,
    outer_damage: f32,
    radius: f32,
    means: &str,
) {
    let falloff = Falloff { inner: inner_damage, outer: outer_damage, radius };
    break_glass(world, tick, origin, radius, None, falloff, means);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blast(cone: Option<([f32; 3], f32)>) -> ExplosionBlast {
        ExplosionBlast {
            cone,
            origin: [100.0, 200.0, 50.0],
            radius: 150.0,
            inner_damage: 130.0,
            outer_damage: 30.0,
            weapon: 7,
            source: DamageSource::Melee,
            attacker: ClientId(0),
            attacker_life: LifeSequence(1),
            killcam_entity_start_time: 0,
        }
    }

    #[test]
    fn a_blast_without_a_cone_reaches_every_direction() {
        let round = blast(None);
        assert!(round.contains([100.0, 200.0, 50.0]));
        assert!(round.contains([400.0, 200.0, 50.0]));
        assert!(round.contains([-300.0, -9.0, 1000.0]));
    }

    #[test]
    fn a_cone_reaches_ahead_not_behind_and_not_square_to_its_facing() {
        let ahead = blast(Some(([1.0, 0.0, 0.0], 0.5)));
        assert!(ahead.contains([160.0, 200.0, 50.0]));
        assert!(ahead.contains([160.0, 230.0, 50.0]), "about 27 degrees off the facing, inside 60");
        assert!(!ahead.contains([40.0, 200.0, 50.0]));
        assert!(!ahead.contains([100.0, 300.0, 50.0]));
        assert!(!ahead.contains([160.0, 340.0, 50.0]), "about 67 degrees off the facing, outside 60");
    }

    #[test]
    fn the_centre_of_a_cone_is_inside_it() {
        assert!(blast(Some(([0.0, 1.0, 0.0], 0.9))).contains([100.0, 200.0, 50.0]));
    }

    #[test]
    fn the_falloff_runs_from_the_inner_damage_to_the_outer() {
        let falloff = blast(None).falloff();
        assert_eq!((falloff.inner, falloff.outer, falloff.radius), (130.0, 30.0, 150.0));
    }

    #[test]
    fn a_breakable_takes_the_inner_damage_at_the_centre_and_the_outer_at_the_edge() {
        let b = blast(None);
        assert_eq!(destructible_damage(&b, 0.0), 130.0);
        assert_eq!(destructible_damage(&b, 150.0), 30.0);
        assert_eq!(destructible_damage(&b, 75.0), 80.0);
    }

    #[test]
    fn a_tiny_radius_spreads_over_one_unit_so_it_cannot_divide_by_nothing() {
        let tiny = ExplosionBlast { radius: 0.0, ..blast(None) };
        assert_eq!(destructible_damage(&tiny, 0.0), 130.0);
        assert_eq!(destructible_damage(&tiny, 1.0), 30.0);
        assert_eq!(destructible_damage(&tiny, 0.5), 80.0);
    }
}
