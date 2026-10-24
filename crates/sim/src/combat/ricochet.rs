//! Riot shields turning bullets aside.

use super::Emission;
use crate::bullet_collision::{BulletTraceSegment, ColliderId, HITLOC_SHIELD, PlayerCollisionPose};
use crate::frame::FrameWorld;
use crate::identities::MatchRng;
use crate::world::ClientId;
use weapon_t6::facts::{weap_class, weap_type};

/// The share of eligible bullets a shield turns aside (BO2's `bullet_ricochetBaseChance`).
pub(crate) const RICOCHET_BASE_CHANCE: f32 = 0.5;
/// How many times one bullet may bounce in a tick. BO2 sets no limit; this only stops two shields facing each other
/// from passing a bullet back and forth for ever.
const MAX_RICOCHETS: u8 = 8;

/// A bullet a riot shield turned: where it flies on from, which way, how far, and the shield's bearer, whom it passes.
pub(crate) struct ShieldBounce {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub max_range: f32,
    pub bearer: ClientId,
}

/// The bullet that continues from `em` after a shield bounce, if there was one.
pub(super) fn follow_up(
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

/// BO2's BulletRicochet: a bullet stopped by a player's riot shield flies on, half the time, as a new bullet. A
/// shotgun's pellet or a bullet that explodes never does, and only one that meets the shield's face. The new bullet
/// leaves one unit off the hit along `ricochet_direction`, with the gun's full range (damage falls off from there).
/// BO2 starts it outside the shield bearer, on the shield's face; ours can hit the bearer's body, so it passes
/// through them instead. Players' bullets and scorestreak guns' (a turret's, a vehicle's) both come through here.
/// `seed` rolls the dice and `bounces` is how many it has made already.
pub(crate) fn shield_bounce(
    world: &FrameWorld,
    weapon: u32,
    direction: [f32; 3],
    seed: u64,
    bounces: u8,
    segments: &[BulletTraceSegment],
    poses: &[PlayerCollisionPose],
) -> Option<ShieldBounce> {
    if bounces >= MAX_RICOCHETS || world.gun_facts().is_empty() {
        return None;
    }
    let gun = world.gun_facts().get(weapon).core;
    let shotgun_like = matches!(gun.weap_class, weap_class::SPREAD | weap_class::PISTOL_SPREAD);
    if gun.weap_type != weap_type::BULLET || gun.bullet_impact_explode || shotgun_like {
        return None;
    }
    let last = segments.last()?;
    let Some(ColliderId::Player { client, hitloc: HITLOC_SHIELD, .. }) = last.collider else {
        return None;
    };
    // The way the shield faces; ours is flat, so it has no up or down.
    let facing = poses.iter().find(|pose| pose.client == client)?.shield?.facing;
    let normal = [facing[0], facing[1], 0.0];
    let roll = MatchRng::new(seed).next_u32() as f32 / u32::MAX as f32;
    let toward_shield = direction[0] * normal[0] + direction[1] * normal[1];
    if roll > RICOCHET_BASE_CHANCE || toward_shield > 0.0 {
        return None;
    }
    let out = ricochet_direction(direction, normal)?;
    let max_range = world.combat_facts_for(weapon)?.bullet_range();
    Some(ShieldBounce {
        origin: [last.end[0] + out[0], last.end[1] + out[1], last.end[2] + out[2]],
        direction: out,
        max_range,
        bearer: client,
    })
}

fn normalised(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 1e-4).then(|| [v[0] / length, v[1] / length, v[2] / length])
}

/// The way a bullet travelling along `dir` leaves a shield facing `normal` (BO2's CalcRicochet). A normal that
/// points down is tipped up first (z + 0.5, at most 0.1). The bullet then leaves along the normal with the part along
/// its own line taken out, so square to the way it came. `None` when it meets the shield square on and nothing is
/// left.
pub(crate) fn ricochet_direction(dir: [f32; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    let mut face = normal;
    if face[2] < 0.0 {
        face[2] = (face[2] + 0.5).min(0.1);
        face = normalised(face)?;
    }
    let back = [-dir[0], -dir[1], -dir[2]];
    let along = back[0] * face[0] + back[1] * face[1] + back[2] * face[2];
    normalised([
        face[0] - back[0] * along,
        face[1] - back[1] * along,
        face[2] - back[2] * along,
    ])
}

#[cfg(test)]
mod tests {
    use super::ricochet_direction;

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn the_bullet_leaves_square_to_the_way_it_came() {
        let (s, c) = 30f32.to_radians().sin_cos();
        let dir = [c, s, 0.0];
        let out = ricochet_direction(dir, [-1.0, 0.0, 0.0]).expect("a slanted hit bounces");
        assert!(near(out, [-0.5, 0.866, 0.0]), "{out:?}");
        let dot = out[0] * dir[0] + out[1] * dir[1] + out[2] * dir[2];
        assert!(dot.abs() < 1e-4, "{dot}");
    }

    #[test]
    fn a_square_hit_leaves_nothing_to_bounce() {
        assert_eq!(ricochet_direction([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]), None);
    }

    #[test]
    fn a_shield_tipped_down_is_tipped_up_first() {
        let root = 1.04f32.sqrt();
        let out = ricochet_direction([1.0, 0.0, 0.0], [-1.0 / root, 0.0, -0.2 / root]).expect("bounces");
        assert!(near(out, [0.0, 0.0, 1.0]), "{out:?}");
    }

    #[test]
    fn a_zero_normal_gives_nothing() {
        assert_eq!(ricochet_direction([1.0, 0.0, 0.0], [0.0, 0.0, 0.0]), None);
    }
}
