//! Shots that leave the gun as a projectile rather than a bullet: rockets, launched and thrown grenades, bolts.
//! `fire_accepted_shot` handles a real trigger pull; `magic_bullet` is the script-driven version of the same launch.

mod platform;
mod rocket;

use crate::combat::AcceptedShot;
use crate::equipment::{GrenadeLaunchKind, ProjectileState, spawn_grenade_projectile};
use crate::frame::FrameWorld;
use weapon_t6::damage::{FireWeaponKind, fire_weapon_kind};

pub(crate) use platform::platform_velocity;

/// Launches whatever projectile the weapon of an accepted shot fires. Bullet weapons are not handled here.
pub(crate) fn fire_accepted_shot(world: &mut FrameWorld, tick: crate::Tick, shot: &AcceptedShot) {
    let Some(combat) = world.combat_facts_for(shot.weapon) else {
        return;
    };
    match fire_weapon_kind(combat.weap_type, combat.weap_class) {
        Some(FireWeaponKind::Missile) => {
            rocket::fire(world, tick, shot, true);
        }
        Some(FireWeaponKind::GrenadeLauncher) => {
            launch_grenade(world, tick, shot, GrenadeLaunchKind::Launcher);
        }
        Some(FireWeaponKind::ThrownGrenade) => {
            launch_grenade(world, tick, shot, GrenadeLaunchKind::Thrown { remaining_fuse_ms: None });
        }
        Some(FireWeaponKind::Bullet) | None => {}
    }
}

/// A grenade fired or thrown by the shooter, who passes on the speed of whatever moving platform they stand on.
fn launch_grenade(world: &mut FrameWorld, tick: crate::Tick, shot: &AcceptedShot, kind: GrenadeLaunchKind) {
    let platform = platform_velocity(world, shot.attacker, crate::level_time_ms(tick));
    spawn_grenade_projectile(
        world,
        shot.attacker,
        shot.weapon,
        tick,
        shot.origin,
        shot.angles,
        shot.owner_velocity,
        platform,
        kind,
    );
}

/// The angles of a shot from `start` toward `end`, by the game's own vector-to-angles: straight down is pitch 90 and
/// straight up 270 (the older engine has the two the other way round, which would send a shot fired down upward).
fn shot_angles(start: [f32; 3], end: [f32; 3]) -> [f32; 3] {
    gsc_t6::math::vector_to_angles(std::array::from_fn(|i| end[i] - start[i]))
}

/// The accepted shot a script's `MagicBullet` stands in for: fired by `owner` from `start` toward `end` with no
/// spread, no ammunition cost, and no motion of its own.
fn scripted_shot(owner: crate::ClientId, life: crate::LifeSequence, weapon: u32, start: [f32; 3], end: [f32; 3]) -> AcceptedShot {
    AcceptedShot {
        shot_id: crate::ShotId(0),
        attacker: owner,
        attacker_life: life,
        hand: 0,
        weapon,
        ammo_used: 0,
        origin: start,
        angles: shot_angles(start, end),
        ads_frac: 1.0,
        view_height_current: 0.0,
        aim_spread_scale: 0.0,
        perks0: 0,
        combat_seed: 0,
        owner_velocity: [0.0; 3],
        spread_degrees: 0.0,
        pen_scale: None,
    }
}

/// A script firing `weapon` for `owner` from `start` toward `end`. Returns the projectile it made, or why none was.
pub(crate) fn magic_bullet(
    world: &mut FrameWorld,
    tick: crate::Tick,
    owner: crate::ClientId,
    weapon: u32,
    start: [f32; 3],
    end: [f32; 3],
) -> Result<ProjectileState, String> {
    let combat = world
        .combat_facts_for(weapon)
        .ok_or_else(|| format!("weapon {weapon} has no combat facts"))?;
    let life = world
        .client_meta(owner)
        .ok_or("MagicBullet owner is not connected")?
        .life_sequence;
    let shot = scripted_shot(owner, life, weapon, start, end);
    let launched = match fire_weapon_kind(combat.weap_type, combat.weap_class) {
        Some(FireWeaponKind::Missile) => rocket::fire(world, tick, &shot, false),
        Some(FireWeaponKind::GrenadeLauncher) => spawn_scripted_grenade(world, tick, &shot, GrenadeLaunchKind::Launcher),
        Some(FireWeaponKind::ThrownGrenade) => spawn_scripted_grenade(
            world,
            tick,
            &shot,
            GrenadeLaunchKind::Thrown { remaining_fuse_ms: None },
        ),
        Some(FireWeaponKind::Bullet) | None => return Err("MagicBullet bullets are not simulated".into()),
    };
    launched.ok_or_else(|| format!("weapon {weapon} launched no projectile"))
}

/// Spawns a grenade for a script and finds it again: the spawn call only says whether it worked, so the new
/// projectile is the owner's newest one of that weapon made this tick.
fn spawn_scripted_grenade(
    world: &mut FrameWorld,
    tick: crate::Tick,
    shot: &AcceptedShot,
    kind: GrenadeLaunchKind,
) -> Option<ProjectileState> {
    let spawned = spawn_grenade_projectile(
        world,
        shot.attacker,
        shot.weapon,
        tick,
        shot.origin,
        shot.angles,
        [0.0; 3],
        [0.0; 3],
        kind,
    );
    if !spawned {
        return None;
    }
    let now = crate::level_time_ms(tick);
    crate::frame::collect_projectiles(world.ecs())
        .into_iter()
        .filter(|p| p.owner == shot.attacker && p.weapon == shot.weapon && p.spawn_time_ms == now)
        .max_by_key(|p| p.id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_down_is_pitch_90_and_straight_up_270() {
        assert_eq!(shot_angles([10.0, 20.0, 500.0], [10.0, 20.0, 0.0]), [90.0, 0.0, 0.0]);
        assert_eq!(shot_angles([0.0; 3], [0.0, 0.0, 100.0]), [270.0, 0.0, 0.0]);
    }

    #[test]
    fn a_shot_fired_down_flies_down_and_one_fired_up_flies_up() {
        let (down, _, _) = game_math::angle_vectors(shot_angles([0.0, 0.0, 9.0], [0.0; 3]));
        assert!(down[2] < -0.999, "{down:?}");
        let (up, _, _) = game_math::angle_vectors(shot_angles([0.0; 3], [0.0, 0.0, 9.0]));
        assert!(up[2] > 0.999, "{up:?}");
    }

    #[test]
    fn a_slanted_shot_gets_the_usual_angles() {
        let slant = shot_angles([0.0; 3], [0.0, 100.0, -100.0]);
        assert!((slant[0] - 45.0).abs() < 1e-4 && (slant[1] - 90.0).abs() < 1e-4, "{slant:?}");
    }

    #[test]
    fn a_scripted_shot_costs_nothing_and_does_not_spread() {
        let shot = scripted_shot(crate::ClientId(3), crate::LifeSequence(7), 12, [1.0, 2.0, 3.0], [1.0, 2.0, 103.0]);
        assert_eq!((shot.attacker, shot.attacker_life, shot.weapon), (crate::ClientId(3), crate::LifeSequence(7), 12));
        assert_eq!((shot.ammo_used, shot.spread_degrees, shot.combat_seed), (0, 0.0, 0));
        assert_eq!(shot.origin, [1.0, 2.0, 3.0]);
        assert_eq!(shot.angles, [270.0, 0.0, 0.0]);
        assert_eq!(shot.owner_velocity, [0.0; 3]);
    }
}
