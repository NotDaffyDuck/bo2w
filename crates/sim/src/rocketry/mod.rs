//! Shots that leave the gun as a projectile rather than a bullet: rockets, launched and thrown grenades, bolts.
//! `fire_accepted_shot` handles a real trigger pull; `magic_bullet` is the script-driven version of the same launch.

mod platform;
mod rocket;

use crate::gunfire::ValidatedShot;
use crate::gear::{GrenadeLaunchKind, FlyingProjectile, spawn_grenade_projectile};
use crate::step_view::StepWorldView;
use weapon_t6::damage::{ShotOriginKind, fire_weapon_kind};

pub(crate) use platform::platform_velocity;

/// Launches whatever projectile the weapon of an accepted shot fires. Bullet weapons are not handled here.
pub(crate) fn fire_accepted_shot(world: &mut StepWorldView, tick: crate::StepNo, shot: &ValidatedShot) {
    let Some(combat) = world.damage_profile_of(shot.gun) else {
        return;
    };
    match fire_weapon_kind(combat.gun_family_code, combat.gun_kind) {
        Some(ShotOriginKind::Missile) => {
            rocket::fire(world, tick, shot, true);
        }
        Some(ShotOriginKind::TubeLauncher) => {
            launch_grenade(world, tick, shot, GrenadeLaunchKind::Launcher);
        }
        Some(ShotOriginKind::HandTossedExplosive) => {
            launch_grenade(world, tick, shot, GrenadeLaunchKind::Thrown { remaining_fuse_ms: None });
        }
        Some(ShotOriginKind::Bullet) | None => {}
    }
}

/// A grenade fired or thrown by the shooter, who passes on the speed of whatever moving platform they stand on.
fn launch_grenade(world: &mut StepWorldView, step: crate::StepNo, shot: &ValidatedShot, kind: GrenadeLaunchKind) {
    let platform = platform_velocity(world, shot.attacker, crate::level_time_ms(step));
    spawn_grenade_projectile(
        world,
        shot.attacker,
        shot.gun,
        step,
        shot.spot,
        shot.turn,
        shot.owner_velocity,
        platform,
        kind,
    );
}

/// The angles of a shot from `start` toward `end`, by the game's own vector-to-angles: straight down is pitch 90 and
/// straight up 270 (the older engine has the two the other way round, which would send a shot fired down upward).
fn shot_angles(start: [f32; 3], till: [f32; 3]) -> [f32; 3] {
    gsc_t6::math::vector_to_angles(std::array::from_fn(|i| till[i] - start[i]))
}

/// The accepted shot a script's `MagicBullet` stands in for: fired by `owner` from `start` toward `end` with no
/// spread, no ammunition cost, and no motion of its own.
fn scripted_shot(owner: crate::SeatNo, life: crate::LifeCount, gun_no: u32, from: [f32; 3], till: [f32; 3]) -> ValidatedShot {
    ValidatedShot {
        shot_serial: crate::BulletNo(0),
        attacker: owner,
        killer_life: life,
        fist: 0,
        gun: gun_no,
        rounds_spent: 0,
        spot: from,
        turn: shot_angles(from, till),
        scope_frac: 1.0,
        eye_rise: 0.0,
        sight_spread_factor: 0.0,
        combat_random_seed: 0,
        owner_velocity: [0.0; 3],
        cone_angle_deg: 0.0,
        pen_scale: None,
    }
}

/// A script firing `weapon` for `owner` from `start` toward `end`. Returns the projectile it made, or why none was.
pub(crate) fn magic_bullet(
    level: &mut StepWorldView,
    tick: crate::StepNo,
    owner: crate::SeatNo,
    gun: u32,
    from: [f32; 3],
    end: [f32; 3],
) -> Result<FlyingProjectile, String> {
    let combat = level
        .damage_profile_of(gun)
        .ok_or_else(|| format!("weapon {gun} has no combat facts"))?;
    let life = level
        .client_meta(owner)
        .ok_or("MagicBullet owner is not connected")?
        .life_sequence;
    let shot = scripted_shot(owner, life, gun, from, end);
    let launched = match fire_weapon_kind(combat.gun_family_code, combat.gun_kind) {
        Some(ShotOriginKind::Missile) => rocket::fire(level, tick, &shot, false),
        Some(ShotOriginKind::TubeLauncher) => spawn_scripted_grenade(level, tick, &shot, GrenadeLaunchKind::Launcher),
        Some(ShotOriginKind::HandTossedExplosive) => spawn_scripted_grenade(
            level,
            tick,
            &shot,
            GrenadeLaunchKind::Thrown { remaining_fuse_ms: None },
        ),
        Some(ShotOriginKind::Bullet) | None => return Err("MagicBullet bullets are not simulated".into()),
    };
    launched.ok_or_else(|| format!("weapon {gun} launched no projectile"))
}

/// bo2zm: a script's `magicgrenadetype` (BO2's `ScrCmd_MagicGrenadeType`): a
/// grenade from `origin` at the given velocity, going off after `fuse_ms`.
pub(crate) fn magic_grenade(
    world: &mut StepWorldView,
    step: crate::StepNo,
    owner: crate::SeatNo,
    gun: u32,
    spot: [f32; 3],
    drift: [f32; 3],
    fuse_ms: i32,
) -> Result<FlyingProjectile, String> {
    let angles = game_math::dir_to_euler(drift);
    let kind = GrenadeLaunchKind::Magic { fuse_ms };
    let now = crate::level_time_ms(step);
    spawn_grenade_projectile(world, owner, gun, step, spot, angles, drift, [0.0; 3], kind)
        .then(|| {
            crate::step_view::gather_shells(world.ecs())
                .into_iter()
                .filter(|p| p.owner == owner && p.gun == gun && p.spawn_time_ms == now)
                .max_by_key(|p| p.id.0)
        })
        .flatten()
        .ok_or_else(|| format!("weapon {gun} launched no grenade"))
}

/// Spawns a grenade for a script and finds it again: the spawn call only says whether it worked, so the new
/// projectile is the owner's newest one of that weapon made this tick.
fn spawn_scripted_grenade(
    world: &mut StepWorldView,
    step: crate::StepNo,
    shot: &ValidatedShot,
    kind: GrenadeLaunchKind,
) -> Option<FlyingProjectile> {
    let spawned = spawn_grenade_projectile(
        world,
        shot.attacker,
        shot.gun,
        step,
        shot.spot,
        shot.turn,
        [0.0; 3],
        [0.0; 3],
        kind,
    );
    if !spawned {
        return None;
    }
    let now = crate::level_time_ms(step);
    crate::step_view::gather_shells(world.ecs())
        .into_iter()
        .filter(|p| p.owner == shot.attacker && p.gun == shot.gun && p.spawn_time_ms == now)
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
        let (down, _, _) = game_math::euler_basis(shot_angles([0.0, 0.0, 9.0], [0.0; 3]));
        assert!(down[2] < -0.999, "{down:?}");
        let (up, _, _) = game_math::euler_basis(shot_angles([0.0; 3], [0.0, 0.0, 9.0]));
        assert!(up[2] > 0.999, "{up:?}");
    }

    #[test]
    fn a_slanted_shot_gets_the_usual_angles() {
        let slant = shot_angles([0.0; 3], [0.0, 100.0, -100.0]);
        assert!((slant[0] - 45.0).abs() < 1e-4 && (slant[1] - 90.0).abs() < 1e-4, "{slant:?}");
    }

    #[test]
    fn a_scripted_shot_costs_nothing_and_does_not_spread() {
        let shot = scripted_shot(crate::SeatNo(3), crate::LifeCount(7), 12, [1.0, 2.0, 3.0], [1.0, 2.0, 103.0]);
        assert_eq!((shot.attacker, shot.killer_life, shot.gun), (crate::SeatNo(3), crate::LifeCount(7), 12));
        assert_eq!((shot.rounds_spent, shot.cone_angle_deg, shot.combat_random_seed), (0, 0.0, 0));
        assert_eq!(shot.spot, [1.0, 2.0, 3.0]);
        assert_eq!(shot.turn, [270.0, 0.0, 0.0]);
        assert_eq!(shot.owner_velocity, [0.0; 3]);
    }
}
