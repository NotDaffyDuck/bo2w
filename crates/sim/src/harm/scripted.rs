//! Blasts and hits the game scripts ask for (`radiusDamage`, `damage` on a player): the same players, shield and
//! cover rules as an explosion, but the damage goes straight to the victim's script callback.

use super::blast::apply_shared_glass_blast;
use super::{FatalityRecord, Falloff, length, minus, player_share, reachable_clients};
use crate::ScriptedModel;
use crate::step_view::StepWorldView;
use crate::match_ids::{HarmOrigin, LifeCount, BuckshotId};
use crate::player_ledger::PlayerSessionStage;
use crate::gsc_host::{HitTarget, ScriptBlast, ScriptHit};
use crate::client_script::IDFLAGS_RADIUS;
use crate::sim_parts::{SeatNo, StepNo};

/// What struck: who threw or fired it, with what, and from where.
struct Strike {
    shooter: Option<SeatNo>,
    inflictor: Option<ScriptedModel>,
    gun: u32,
    means: &'static str,
    flags: i32,
    hitloc: u8,
    at: [f32; 3],
}

/// `v` scaled to length 1; the zero vector stays zero.
fn unit_vector(v: [f32; 3]) -> [f32; 3] {
    let len = length(v);
    if len > 0.0 { v.map(|c| c / len) } else { [0.0; 3] }
}

/// The record handed to the death callback if this strike kills `victim`. A script's strike names the entity that did
/// it (or, if it has none, an id no entity has) as the radius-damage inflictor.
fn death_commit(strike: &Strike, victim: SeatNo, victim_life: LifeCount, attacker: SeatNo, striker_span: LifeCount, amount: i32) -> FatalityRecord {
    FatalityRecord {
        victim,
        victim_life,
        attacker,
        killer_life: striker_span,
        source: HarmOrigin::Radius(strike.inflictor.unwrap_or(ScriptedModel::decode_net(u32::MAX))),
        pellet: BuckshotId(0),
        gun: strike.gun,
        much: amount,
        cam_ent_since: 0,
        attacker_source_pos: Some(strike.at),
        hit_part: strike.hitloc,
    }
}

/// Hands `amount` of damage to the scripts for `victim`, who stands at `victim_at`; the hit pushes him away from where
/// it struck. With no live attacker there is no death record.
fn strike_player(
    world: &mut StepWorldView,
    tick: StepNo,
    victim: SeatNo,
    victim_life: LifeCount,
    victim_at: [f32; 3],
    harm: i32,
    strike: &Strike,
) {
    let commit = strike.shooter.and_then(|shooter| {
        let striker_span = world.client_meta(shooter)?.life_sequence;
        Some(death_commit(strike, victim, victim_life, shooter, striker_span, harm))
    });
    let hit = crate::client_script::Hit {
        victim,
        striker: strike.shooter,
        amount: harm,
        bits: strike.flags,
        means: strike.means,
        gun: strike.gun,
        point: strike.at,
        way: unit_vector(minus(victim_at, strike.at)),
        hit_part: strike.hitloc,
        inflictor: None,
        commit,
    };
    crate::gsc_host::player_damage(world.ecs(), tick, &hit);
}

/// A script's radius damage: each live player in reach takes his share (cover and riot shield counted, and the shield's
/// splash event sent as it is found); then the glass and the map's damage triggers take theirs.
pub(crate) fn apply_script_blast(world: &mut StepWorldView, tick: StepNo, blast: &ScriptBlast) {
    if blast.reach <= 0.0 {
        return;
    }
    let falloff = Falloff { inner: blast.max, outer: blast.min, reach: blast.reach };
    let strike = Strike {
        shooter: blast.attacker,
        inflictor: blast.inflictor,
        gun: blast.gun,
        means: blast.means,
        flags: IDFLAGS_RADIUS,
        hitloc: 0,
        at: blast.spot,
    };
    for target in reachable_clients(world, blast.spot, blast.reach) {
        let Some(record) = world.client_meta(target) else {
            continue;
        };
        if record.lifecycle != PlayerSessionStage::Alive {
            continue;
        }
        let victim_life_index = record.life_sequence;
        let Some(bounds) = world.seat_box(target) else {
            continue;
        };
        let share = player_share(world, blast.spot, falloff, target, &bounds);
        if let Some(splash) = share.splash {
            splash.announce(world, tick);
        }
        if share.much <= 0 {
            continue;
        }
        let at = world.avatar(target).map_or(bounds.mid(), |ps| ps.origin);
        strike_player(world, tick, target, victim_life_index, at, share.much, &strike);
    }
    apply_shared_glass_blast(world, tick, blast.spot, blast.max, blast.min, blast.reach, blast.means);
    crate::zombie_rt::damage_triggers::blast(
        world.ecs(),
        blast.spot,
        blast.reach,
        blast.max,
        blast.min,
        blast.attacker,
        blast.means,
        blast.gun,
        &|_| true,
    );
}

/// A script's hit on one player (a hit on any other kind of entity is not this module's). It lands in full on a live
/// player and is pushed along the line from where it struck.
pub(crate) fn apply_script_hit(world: &mut StepWorldView, tick: StepNo, hit: &ScriptHit) {
    let HitTarget::Player(target) = hit.target else {
        return;
    };
    if hit.much <= 0 {
        return;
    }
    let Some(record) = world.client_meta(target) else {
        return;
    };
    if record.lifecycle != PlayerSessionStage::Alive {
        return;
    }
    let victim_life_index = record.life_sequence;
    let Some(at) = world.avatar(target).map(|ps| ps.origin) else {
        return;
    };
    let strike = Strike {
        shooter: hit.attacker,
        inflictor: hit.inflictor,
        gun: hit.gun,
        means: hit.means,
        flags: hit.bits,
        hitloc: hit.hit_part,
        at: hit.spot,
    };
    strike_player(world, tick, target, victim_life_index, at, hit.much, &strike);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strike(inflictor: Option<ScriptedModel>) -> Strike {
        Strike {
            shooter: Some(SeatNo(1)),
            inflictor,
            gun: 9,
            means: "MOD_EXPLOSIVE",
            flags: 0,
            hitloc: 3,
            at: [10.0, 20.0, 30.0],
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
        let commit = death_commit(&strike(None), SeatNo(4), LifeCount(2), SeatNo(1), LifeCount(7), 55);
        assert_eq!((commit.victim, commit.victim_life), (SeatNo(4), LifeCount(2)));
        assert_eq!((commit.attacker, commit.killer_life), (SeatNo(1), LifeCount(7)));
        assert_eq!((commit.gun, commit.much, commit.hit_part), (9, 55, 3));
        assert_eq!(commit.attacker_source_pos, Some([10.0, 20.0, 30.0]));
        assert_eq!((commit.pellet, commit.cam_ent_since), (BuckshotId(0), 0));
    }

    #[test]
    fn a_strike_with_an_inflictor_names_it_and_one_without_names_no_entity() {
        let named = death_commit(&strike(Some(ScriptedModel::decode_net(12))), SeatNo(4), LifeCount(2), SeatNo(1), LifeCount(7), 1);
        assert_eq!(named.source, HarmOrigin::Radius(ScriptedModel::decode_net(12)));
        let unnamed = death_commit(&strike(None), SeatNo(4), LifeCount(2), SeatNo(1), LifeCount(7), 1);
        assert_eq!(unnamed.source, HarmOrigin::Radius(ScriptedModel::decode_net(u32::MAX)));
    }
}
