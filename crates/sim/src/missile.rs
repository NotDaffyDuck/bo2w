use crate::combat::{AcceptedShot, spread_direction_on_plane};
use crate::equipment::{GrenadeLaunchKind, ProjectileState, spawn_grenade_projectile};
use crate::frame::FrameWorld;
use crate::identities::MatchRng;
use entity_t6::{
    Trajectory, missile::fire_grenade_no_draw_ms, trajectory::tr_type::LINEAR as TR_LINEAR,
};
use crate::entity_local::{fire_missile_apos, truncated_tr_delta};
use game_math::vec3_length;
use weapon_t6::damage::{FireWeaponKind, ROCKET_SPREAD_PLANE, fire_weapon_kind};

pub(crate) fn fire_accepted_shot(world: &mut FrameWorld, tick: crate::Tick, shot: &AcceptedShot) {
    let Some(combat) = world.combat_facts_for(shot.weapon) else {
        return;
    };
    match fire_weapon_kind(combat.weap_type, combat.weap_class) {
        Some(FireWeaponKind::Bullet) => {}
        Some(FireWeaponKind::GrenadeLauncher) => {
            let carry = platform_velocity(world, shot.attacker, crate::level_time_ms(tick));
            spawn_grenade_projectile(
                world,
                shot.attacker,
                shot.weapon,
                tick,
                shot.origin,
                shot.angles,
                shot.owner_velocity,
                carry,
                GrenadeLaunchKind::Launcher,
            );
        }
        Some(FireWeaponKind::Missile) => {
            fire_missile(world, tick, shot, true);
        }
        Some(FireWeaponKind::ThrownGrenade) => {
            let carry = platform_velocity(world, shot.attacker, crate::level_time_ms(tick));
            spawn_grenade_projectile(
                world,
                shot.attacker,
                shot.weapon,
                tick,
                shot.origin,
                shot.angles,
                shot.owner_velocity,
                carry,
                GrenadeLaunchKind::Thrown {
                    remaining_fuse_ms: None,
                },
            );
        }
        None => {}
    }
}

/// MagicBullet's aim from start to end, by Black Ops II's vectoangles: straight down is pitch 90
/// and straight up 270 (MW2's has the two the other way round, so a shot fired straight down
/// would leave straight up).
fn shot_angles(start: [f32; 3], end: [f32; 3]) -> [f32; 3] {
    gsc_t6::math::vector_to_angles(std::array::from_fn(|i| end[i] - start[i]))
}

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
    let attacker_life = world
        .client_meta(owner)
        .ok_or("MagicBullet owner is not connected")?
        .life_sequence;
    let shot = AcceptedShot {
        shot_id: crate::ShotId(0),
        attacker: owner,
        attacker_life,
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
    };
    let launched = match fire_weapon_kind(combat.weap_type, combat.weap_class) {
        Some(FireWeaponKind::Missile) => fire_missile(world, tick, &shot, false),
        Some(kind @ (FireWeaponKind::GrenadeLauncher | FireWeaponKind::ThrownGrenade)) => {
            let launch = if kind == FireWeaponKind::GrenadeLauncher {
                GrenadeLaunchKind::Launcher
            } else {
                GrenadeLaunchKind::Thrown {
                    remaining_fuse_ms: None,
                }
            };
            spawn_grenade_projectile(
                world,
                owner,
                weapon,
                tick,
                start,
                shot.angles,
                [0.0; 3],
                [0.0; 3],
                launch,
            )
            .then(|| {
                let now = crate::level_time_ms(tick);
                crate::frame::collect_projectiles(world.ecs())
                    .into_iter()
                    .filter(|p| p.owner == owner && p.weapon == weapon && p.spawn_time_ms == now)
                    .max_by_key(|p| p.id.0)
            })
            .flatten()
        }
        Some(FireWeaponKind::Bullet) | None => {
            return Err("MagicBullet bullets are not simulated".into());
        }
    };
    launched.ok_or_else(|| format!("weapon {weapon} launched no projectile"))
}

/// BO2's `MISSILE_GUIDANCE_BALLISTIC`: a shot that drops (the Crossbow's bolt,
/// the Ballistic Knife's blade).
const MISSILE_GUIDANCE_BALLISTIC: i32 = 4;
/// BO2's `MISSILE_GUIDANCE_TVGUIDED`: a missile he flies himself.
const MISSILE_GUIDANCE_TVGUIDED: i32 = 6;
/// Weapon_RocketLauncher_Fire: how hard a rocket shoves him back.
const ROCKET_KICKBACK: f32 = 64.0;

/// G_FireRocket. `fired`: he pulled the trigger, so Weapon_RocketLauncher_Fire's
/// own rules hold too: the rocket keeps the speed of the platform he stands
/// on, and it shoves him back.
fn fire_missile(
    world: &mut FrameWorld,
    tick: crate::Tick,
    shot: &AcceptedShot,
    fired: bool,
) -> Option<ProjectileState> {
    let Some(facts) = world.missile_launch_facts(shot.weapon) else {
        diag::warn!(
            Sim,
            "weapon {} fires no missile: no projectile speed",
            shot.weapon
        );
        return None;
    };
    let mut rng = MatchRng::new(shot.combat_seed as u64);
    let mut dir = spread_direction_on_plane(
        shot.angles,
        shot.spread_degrees,
        &mut rng,
        ROCKET_SPREAD_PLANE,
    );
    let mut origin = shot.origin;
    if facts.missile_guidance == 3 {
        dir = glam::Vec3::new(dir[0], dir[1], dir[2] + 0.3)
            .normalize()
            .to_array();
        let (_, right, _) = game_math::angle_vectors(shot.angles);
        origin = core::array::from_fn(|i| origin[i] + right[i] * 2.5);
    }
    // A missile he flies starts 13 units out along its way.
    if facts.missile_guidance == MISSILE_GUIDANCE_TVGUIDED {
        origin = core::array::from_fn(|i| origin[i] + dir[i] * 13.0);
    }
    let lock = world
        .client_meta(shot.attacker)
        .map(|m| m.weapon_lock)
        .filter(|lock| lock.can_fire(shot.weapon, shot.attacker_life.0));
    let guide = crate::MissileGuide {
        target: lock.and_then(|lock| lock.aim),
        top: lock.is_some_and(|lock| lock.flags & 4 != 0),
        ..Default::default()
    };
    let speed = facts.projectile_speed as f32;
    let mut gun_vel = shot.owner_velocity;
    if fired && facts.missile_guidance != MISSILE_GUIDANCE_TVGUIDED {
        let carry = platform_velocity(world, shot.attacker, crate::level_time_ms(tick));
        gun_vel = core::array::from_fn(|i| gun_vel[i] + carry[i]);
    }
    let id = world.allocate_projectile_id();
    let entnum = match world.allocate_dynamic_entity(crate::gentity::EntityRunKind::Missile) {
        Ok(entity) => entity.number(),
        Err(error) => {
            diag::warn!(Sim, "missile not spawned: {error:?}");
            return None;
        }
    };
    let time_ms = crate::level_time_ms(tick);
    let launch = rocket_launch_velocity(dir, speed, facts.projectile_speed_up, gun_vel);
    let velocity = truncated_tr_delta(launch);
    let raw_speed = vec3_length(launch);
    let pos = Trajectory {
        tr_time: time_ms,
        // A javelin climbs and a ballistic shot drops: both fall.
        tr_type: if matches!(facts.missile_guidance, 3 | MISSILE_GUIDANCE_BALLISTIC) {
            entity_t6::trajectory::tr_type::GRAVITY
        } else {
            TR_LINEAR
        },
        tr_duration: 0,
        tr_delta: velocity,
        tr_base: origin,
    };
    perf::projectile(shot.weapon);
    let projectile = ProjectileState {
        id,
        owner: shot.attacker,
        owner_life: shot.attacker_life,
        weapon: shot.weapon,
        origin,
        velocity,
        pos,
        apos: fire_missile_apos(dir),
        entnum,
        launch_time: time_ms + fire_grenade_no_draw_ms(raw_speed),
        spawn_time_ms: time_ms,
        detonate_at_ms: None,
        cleanup_at_ms: time_ms.saturating_add(crate::equipment::ROCKET_CLEANUP_MS),
        travel_distance: 0.0,
        live: true,
        stuck_pane: None,
        grounded: false,
        guide,
        impact_target: None,
        rider: None,
        coasting: false,
    };
    world.push_projectile(projectile);
    let full_auto = world.gun_facts().is_empty()
        || world.gun_facts().get(shot.weapon).core.fire_type
            == weapon_t6::facts::fire_type::FULLAUTO;
    if fired && !full_auto {
        let (forward, _, _) = game_math::angle_vectors(shot.angles);
        if let Some(ps) = world.player_mut(shot.attacker) {
            kick_back(&mut ps.velocity, forward);
        }
    }
    Some(projectile)
}

/// G_FireRocket: the speed along its aim, plus its speed up (90 for the
/// Crossbow and the Ballistic Knife, none for a rocket), plus `gun_vel`.
fn rocket_launch_velocity(dir: [f32; 3], speed: f32, speed_up: i32, gun_vel: [f32; 3]) -> [f32; 3] {
    let mut v: [f32; 3] = core::array::from_fn(|i| dir[i] * speed);
    v[2] += speed_up as f32;
    core::array::from_fn(|i| v[i] + gun_vel[i])
}

/// Weapon_RocketLauncher_Fire (and BO2's grenade throws and launches): the
/// speed of the script mover he stands on, from where its path had it
/// 200 ms ago to where it has it now.
pub(crate) fn platform_velocity(world: &FrameWorld, who: crate::ClientId, now: i32) -> [f32; 3] {
    let Some(ground) = world.player(who).map(|ps| ps.ground_entity_num) else {
        return [0.0; 3];
    };
    let Some(mover) = world.script_mover_by_number(ground) else {
        return [0.0; 3];
    };
    let pos = crate::gentity::pos_from_entity_state(&mover.state);
    let was = entity_t6::evaluate_trajectory(&pos, now - 200);
    let is = entity_t6::evaluate_trajectory(&pos, now);
    core::array::from_fn(|i| (is[i] - was[i]) * 5.0)
}

/// Weapon_RocketLauncher_Fire: a rocket from a launcher that is not
/// full-auto shoves him back along his aim, 64 units a second, but never
/// up (aimed down it only shoves him level).
fn kick_back(velocity: &mut [f32; 3], forward: [f32; 3]) {
    velocity[0] -= forward[0] * ROCKET_KICKBACK;
    velocity[1] -= forward[1] * ROCKET_KICKBACK;
    velocity[2] += (-forward[2]).min(0.0) * ROCKET_KICKBACK;
}

#[cfg(test)]
mod tests {
    use super::{kick_back, rocket_launch_velocity, shot_angles};

    #[test]
    fn a_magic_bullet_fired_straight_down_flies_down() {
        let down = shot_angles([10.0, 20.0, 500.0], [10.0, 20.0, 0.0]);
        assert_eq!(down, [90.0, 0.0, 0.0]);
        let (forward, _, _) = game_math::angle_vectors(down);
        assert!(forward[2] < -0.999, "{forward:?}");
        let up = shot_angles([0.0; 3], [0.0, 0.0, 100.0]);
        assert_eq!(up, [270.0, 0.0, 0.0]);
        let (forward, _, _) = game_math::angle_vectors(up);
        assert!(forward[2] > 0.999, "{forward:?}");
        // Any other aim is MW2's and BO2's alike.
        let slant = shot_angles([0.0; 3], [0.0, 100.0, -100.0]);
        assert!((slant[0] - 45.0).abs() < 1e-4 && (slant[1] - 90.0).abs() < 1e-4, "{slant:?}");
    }

    #[test]
    fn a_bolt_leaves_with_its_speed_up_and_his_own_speed() {
        // Aimed level along x at 2180 (the Ballistic Knife), speed up 90, running along y.
        let v = rocket_launch_velocity([1.0, 0.0, 0.0], 2180.0, 90, [0.0, 190.0, 0.0]);
        assert_eq!(v, [2180.0, 190.0, 90.0]);
        // A rocket has no speed up.
        assert_eq!(
            rocket_launch_velocity([0.0, 0.0, 1.0], 1500.0, 0, [0.0; 3]),
            [0.0, 0.0, 1500.0]
        );
    }

    #[test]
    fn a_rocket_shoves_him_back_but_never_up() {
        // Aimed level along x: 64 back.
        let mut v = [0.0; 3];
        kick_back(&mut v, [1.0, 0.0, 0.0]);
        assert_eq!(v, [-64.0, 0.0, 0.0]);
        // Aimed straight up: shoved down.
        let mut v = [0.0; 3];
        kick_back(&mut v, [0.0, 0.0, 1.0]);
        assert_eq!(v, [0.0, 0.0, -64.0]);
        // Aimed straight down: no lift (no rocket jumps).
        let mut v = [0.0, 0.0, 10.0];
        kick_back(&mut v, [0.0, 0.0, -1.0]);
        assert_eq!(v, [0.0, 0.0, 10.0]);
    }
}
