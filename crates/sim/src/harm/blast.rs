//! An explosion: what it does to the players, entities, map triggers, breakables and glass inside its radius.

use super::shield::ShieldSplash;
use super::{HarmRequest, Falloff, land_harm, dot, length, minus, player_share, reachable_clients};
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, LifeCount, BuckshotId};
use crate::player_ledger::PlayerSessionStage;
use crate::client_script::IDFLAGS_RADIUS;
use crate::sim_parts::{SeatNo, StepNo};
use crate::glass_and_props::Blast;

/// An explosion at `origin`: `inner_damage` at its centre falling to `outer_damage` at `radius`, optionally only
/// inside a cone (its facing and the cosine of its half angle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ExplosionBlast {
    pub cone: Option<([f32; 3], f32)>,
    pub origin: [f32; 3],
    pub reach: f32,
    pub inner_damage: f32,
    pub outer_damage: f32,
    pub gun: u32,
    pub source: HarmOrigin,
    pub attacker: SeatNo,
    pub killer_life: LifeCount,
    pub cam_ent_since: i32,
}

impl ExplosionBlast {
    /// Whether `point` is inside the blast's cone (always, for a blast without one). The centre itself is.
    pub(crate) fn contains(&self, at: [f32; 3]) -> bool {
        let Some((facing, cosine)) = self.cone else {
            return true;
        };
        let offset = minus(at, self.origin);
        let reach = length(offset);
        reach <= f32::EPSILON || dot(offset, facing) >= cosine * reach
    }

    fn falloff(&self) -> Falloff {
        Falloff { inner: self.inner_damage, outer: self.outer_damage, reach: self.reach }
    }
}

/// Sets off `blast`. The damage every player takes is worked out first and dealt after, so one death does not change
/// what the next player takes. Then come the entities, the map's damage triggers, its breakables and its glass.
pub(crate) fn apply_explosion_blast(level: &mut StepWorldView, step: StepNo, blast: &ExplosionBlast) {
    if !level.emits_state_snapshot() {
        return;
    }
    let means = crate::client_script::means(level, blast.source, blast.gun, 0, true);

    let (attempts, splashes) = players_in_reach(level, blast);
    for splash in splashes {
        splash.announce(level, step);
    }
    for try_no in &attempts {
        let _ = land_harm(level, step, try_no);
    }

    hurt_entities(level, blast, means);
    crate::zombie_rt::damage_triggers::blast(
        level.ecs(),
        blast.origin,
        blast.reach,
        blast.inner_damage,
        blast.outer_damage,
        Some(blast.attacker),
        means,
        blast.gun,
        &|point| blast.contains(point),
    );
    break_destructibles(level, step, blast, means);
    break_glass(level, step, blast.origin, blast.reach, blast.cone, blast.falloff(), means);
}

/// Every player the blast reaches, as the damage to attempt on him; and for those with a riot shield up who are
/// alive, the splash event of what the shield saved.
fn players_in_reach(level: &StepWorldView, blast: &ExplosionBlast) -> (Vec<HarmRequest>, Vec<ShieldSplash>) {
    let mut attempts = Vec::new();
    let mut sprays = Vec::new();
    if blast.reach <= 0.0 {
        return (attempts, sprays);
    }
    let falloff = blast.falloff();
    for target in reachable_clients(level, blast.origin, blast.reach) {
        let Some(info) = level.client_meta(target) else {
            continue;
        };
        let Some(bounds) = level.seat_box(target) else {
            continue;
        };
        if !blast.contains(bounds.mid()) {
            continue;
        }
        let share = player_share(level, blast.origin, falloff, target, &bounds);
        if info.lifecycle == PlayerSessionStage::Alive {
            sprays.extend(share.splash);
        }
        if share.much <= 0 {
            continue;
        }
        attempts.push(HarmRequest {
            source: blast.source,
            pellet: BuckshotId(0),
            attacker: blast.attacker,
            killer_life: blast.killer_life,
            target,
            victim_life_index: info.life_sequence,
            gun: blast.gun,
            much: share.much,
            cam_ent_since: blast.cam_ent_since,
            attacker_source_pos: Some(blast.origin),
            hit_part: 0,
            blast: true,
            passed_through_wall: false,
            impact: false,
        });
    }
    (attempts, sprays)
}

/// The blast's damage to the script entities in its radius. A vehicle takes it its own way.
fn hurt_entities(world: &mut StepWorldView, blast: &ExplosionBlast, means: &'static str) {
    if blast.reach <= 0.0 {
        return;
    }
    let falloff = blast.falloff();
    for (aim_at, middle, distance) in crate::gsc_host::radius_targets(world.ecs(), blast.origin, blast.reach) {
        if !blast.contains(middle) {
            continue;
        }
        let vehicle_took_it = crate::zombie_rt::blast_vehicle(
            world.ecs(),
            aim_at,
            blast.origin,
            blast.reach,
            blast.inner_damage,
            blast.outer_damage,
            blast.cone,
            means,
            Some(blast.attacker),
            blast.gun,
        );
        if vehicle_took_it {
            continue;
        }
        let qty = falloff.at(distance, 1.0);
        if qty <= 0 {
            continue;
        }
        let hit = crate::gsc_host::EntityHit {
            target: aim_at,
            amount: qty,
            attacker: Some(blast.attacker),
            means,
            gun: blast.gun,
            point: middle,
            way: minus(middle, blast.origin),
            bone: None,
            bits: IDFLAGS_RADIUS,
        };
        crate::gsc_host::damage_entity(world.ecs(), &hit);
    }
}

/// What a blast does to a breakable `distance` from its centre. The share is kept unrounded; the breakable's own
/// damage call cuts it to whole points.
fn destructible_damage(blast: &ExplosionBlast, distance: f32) -> f32 {
    let spread = blast.reach.max(1.0);
    blast.outer_damage + (1.0 - distance / spread) * (blast.inner_damage - blast.outer_damage)
}

/// The map's breakables (cars, barrels, mannequins) each take the blast by their own distance, as the thrower's
/// weapon.
fn break_destructibles(world: &mut StepWorldView, tick: StepNo, blast: &ExplosionBlast, means: &'static str) {
    let broken = crate::staged_props::apply_blast_by(
        world,
        tick,
        blast.origin,
        blast.reach,
        |distance| destructible_damage(blast, distance),
        crate::glass_and_props::t6_means_from_name(means),
        &[],
        Some(blast.attacker),
        blast.gun,
    );
    for (owner, notifies) in broken {
        for notify in notifies {
            // A prop the script hosts (Nuketown's mannequins, outlets, TVs) hears its break.
            match owner.scripted_model() {
                Some(target) => crate::zombie_rt::destructible_broken(
                    world.ecs(),
                    target,
                    &notify,
                    Some(blast.attacker),
                    blast.gun,
                ),
                None => diag::warn!(Sim, "T5 destructible blast notify unhosted: {notify}"),
            }
        }
    }
}

/// Cracks and shatters the map's glass the blast reaches, with the blast's own cone, damage and kind.
fn break_glass(
    world: &mut StepWorldView,
    tick: StepNo,
    centre: [f32; 3],
    blast_reach: f32,
    cone: Option<([f32; 3], f32)>,
    falloff: Falloff,
    means: &str,
) {
    let at_ms = crate::level_time_ms(tick);
    let blast = Blast {
        spot: centre,
        reach: blast_reach,
        cone,
        max_damage: falloff.inner,
        min_damage: falloff.outer,
        means: crate::glass_and_props::t6_means_from_name(means),
    };
    world.blast_glass(&blast, at_ms);
}

/// A blast on the glass alone, with no cone (a map's own explosion).
pub(crate) fn apply_shared_glass_blast(
    world: &mut StepWorldView,
    tick: StepNo,
    spot: [f32; 3],
    inner_damage: f32,
    outer_damage: f32,
    blast_reach: f32,
    means: &str,
) {
    let falloff = Falloff { inner: inner_damage, outer: outer_damage, reach: blast_reach };
    break_glass(world, tick, spot, blast_reach, None, falloff, means);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blast(cone: Option<([f32; 3], f32)>) -> ExplosionBlast {
        ExplosionBlast {
            cone,
            origin: [100.0, 200.0, 50.0],
            reach: 150.0,
            inner_damage: 130.0,
            outer_damage: 30.0,
            gun: 7,
            source: HarmOrigin::Melee,
            attacker: SeatNo(0),
            killer_life: LifeCount(1),
            cam_ent_since: 0,
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
        assert_eq!((falloff.inner, falloff.outer, falloff.reach), (130.0, 30.0, 150.0));
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
        let tiny = ExplosionBlast { reach: 0.0, ..blast(None) };
        assert_eq!(destructible_damage(&tiny, 0.0), 130.0);
        assert_eq!(destructible_damage(&tiny, 1.0), 30.0);
        assert_eq!(destructible_damage(&tiny, 0.5), 80.0);
    }
}
