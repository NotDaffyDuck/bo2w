//! Blasts and hits the game scripts ask for (`radiusDamage`, `damage` on a player): the same players, shield and
//! cover rules as an explosion, but the damage goes straight to the victim's script callback.

use super::blast::apply_shared_glass_blast;
use super::{DeathCommit, Falloff, length, minus, player_share, reachable_clients};
use crate::ScriptModelId;
use crate::frame::FrameWorld;
use crate::identities::{DamageSource, LifeSequence, PelletId};
use crate::match_state::ClientLifecycle;
use crate::script::{HitTarget, ScriptBlast, ScriptHit};
use crate::script_player::IDFLAGS_RADIUS;
use crate::world::{ClientId, Tick};

/// What struck: who threw or fired it, with what, and from where.
struct Strike {
    attacker: Option<ClientId>,
    inflictor: Option<ScriptModelId>,
    weapon: u32,
    means: &'static str,
    flags: i32,
    hitloc: u8,
    point: [f32; 3],
}

/// `v` scaled to length 1; the zero vector stays zero.
fn unit_vector(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len > 0.0 { v.map(|c| c / len) } else { [0.0; 3] }
}

/// The record handed to the death callback if this strike kills `victim`. A script's strike names the entity that did
/// it (or, if it has none, an id no entity has) as the radius-damage inflictor.
fn death_commit(strike: &Strike, victim: ClientId, victim_life: LifeSequence, attacker: ClientId, attacker_life: LifeSequence, amount: i32) -> DeathCommit {
    DeathCommit {
        victim,
        victim_life,
        attacker,
        attacker_life,
        source: DamageSource::Radius(strike.inflictor.unwrap_or(ScriptModelId::from_wire(u32::MAX))),
        pellet: PelletId(0),
        weapon: strike.weapon,
        amount,
        killcam_entity_start_time: 0,
        inflictor_origin: Some(strike.point),
        hitloc: strike.hitloc,
    }
}

/// Hands `amount` of damage to the scripts for `victim`, who stands at `victim_at`; the hit pushes him away from where
/// it struck. With no live attacker there is no death record.
fn strike_player(
    world: &mut FrameWorld,
    tick: Tick,
    victim: ClientId,
    victim_life: LifeSequence,
    victim_at: [f32; 3],
    amount: i32,
    strike: &Strike,
) {
    let commit = strike.attacker.and_then(|attacker| {
        let attacker_life = world.client_meta(attacker)?.life_sequence;
        Some(death_commit(strike, victim, victim_life, attacker, attacker_life, amount))
    });
    let hit = crate::script_player::Hit {
        victim,
        attacker: strike.attacker,
        amount,
        flags: strike.flags,
        means: strike.means,
        weapon: strike.weapon,
        point: strike.point,
        dir: unit_vector(minus(victim_at, strike.point)),
        hitloc: strike.hitloc,
        inflictor: None,
        commit,
    };
    crate::script::player_damage(world.ecs(), tick, &hit);
}

/// A script's radius damage: each live player in reach takes his share (cover and riot shield counted, and the shield's
/// splash event sent as it is found); then the glass and the map's damage triggers take theirs.
pub(crate) fn apply_script_blast(world: &mut FrameWorld, tick: Tick, blast: &ScriptBlast) {
    if blast.radius <= 0.0 {
        return;
    }
    let falloff = Falloff { inner: blast.max, outer: blast.min, radius: blast.radius };
    let strike = Strike {
        attacker: blast.attacker,
        inflictor: blast.inflictor,
        weapon: blast.weapon,
        means: blast.means,
        flags: IDFLAGS_RADIUS,
        hitloc: 0,
        point: blast.origin,
    };
    for target in reachable_clients(world, blast.origin, blast.radius) {
        let Some(meta) = world.client_meta(target) else {
            continue;
        };
        if meta.lifecycle != ClientLifecycle::Alive {
            continue;
        }
        let life = meta.life_sequence;
        let Some(bounds) = world.player_area_bounds(target) else {
            continue;
        };
        let share = player_share(world, blast.origin, falloff, target, &bounds);
        if let Some(splash) = share.splash {
            splash.announce(world, tick);
        }
        if share.amount <= 0 {
            continue;
        }
        let at = world.player(target).map_or(bounds.mid(), |ps| ps.origin);
        strike_player(world, tick, target, life, at, share.amount, &strike);
    }
    apply_shared_glass_blast(world, tick, blast.origin, blast.max, blast.min, blast.radius, blast.means);
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

/// A script's hit on one player (a hit on any other kind of entity is not this module's). It lands in full on a live
/// player and is pushed along the line from where it struck.
pub(crate) fn apply_script_hit(world: &mut FrameWorld, tick: Tick, hit: &ScriptHit) {
    let HitTarget::Player(target) = hit.target else {
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
    let life = meta.life_sequence;
    let Some(at) = world.player(target).map(|ps| ps.origin) else {
        return;
    };
    let strike = Strike {
        attacker: hit.attacker,
        inflictor: hit.inflictor,
        weapon: hit.weapon,
        means: hit.means,
        flags: hit.flags,
        hitloc: hit.hitloc,
        point: hit.origin,
    };
    strike_player(world, tick, target, life, at, hit.amount, &strike);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike(inflictor: Option<ScriptModelId>) -> Strike {
        Strike {
            attacker: Some(ClientId(1)),
            inflictor,
            weapon: 9,
            means: "MOD_EXPLOSIVE",
            flags: 0,
            hitloc: 3,
            point: [10.0, 20.0, 30.0],
        }
    }

    #[test]
    fn the_unit_vector_has_length_one_and_keeps_its_direction() {
        let unit = unit_vector([0.0, 3.0, 4.0]);
        assert_eq!(unit, [0.0, 0.6, 0.8]);
        assert!((length(unit) - 1.0).abs() < 1e-6);
        assert_eq!(unit_vector([-5.0, 0.0, 0.0]), [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn the_zero_vector_has_no_direction_and_is_left_alone() {
        assert_eq!(unit_vector([0.0; 3]), [0.0; 3]);
    }

    #[test]
    fn a_death_record_carries_the_strikes_weapon_place_and_hit_location() {
        let commit = death_commit(&strike(None), ClientId(4), LifeSequence(2), ClientId(1), LifeSequence(7), 55);
        assert_eq!((commit.victim, commit.victim_life), (ClientId(4), LifeSequence(2)));
        assert_eq!((commit.attacker, commit.attacker_life), (ClientId(1), LifeSequence(7)));
        assert_eq!((commit.weapon, commit.amount, commit.hitloc), (9, 55, 3));
        assert_eq!(commit.inflictor_origin, Some([10.0, 20.0, 30.0]));
        assert_eq!((commit.pellet, commit.killcam_entity_start_time), (PelletId(0), 0));
    }

    #[test]
    fn a_strike_with_an_inflictor_names_it_and_one_without_names_no_entity() {
        let named = death_commit(&strike(Some(ScriptModelId::from_wire(12))), ClientId(4), LifeSequence(2), ClientId(1), LifeSequence(7), 1);
        assert_eq!(named.source, DamageSource::Radius(ScriptModelId::from_wire(12)));
        let unnamed = death_commit(&strike(None), ClientId(4), LifeSequence(2), ClientId(1), LifeSequence(7), 1);
        assert_eq!(unnamed.source, DamageSource::Radius(ScriptModelId::from_wire(u32::MAX)));
    }
}
